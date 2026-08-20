#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::sync::watch;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use redfish_exporter::config;
use redfish_exporter::http::serve;
use redfish_exporter::registry::Snapshot;
use redfish_exporter::scraper::Scraper;

#[derive(clap::Parser)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.yaml")]
    config: PathBuf,
    /// Override the configured listen port
    #[arg(short, long)]
    port: Option<u16>,
    /// Log level (error|warn|info|debug|trace), RUST_LOG takes precedence
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut cfg = config::load_config(&args.config)
        .with_context(|| format!("failed to load config from {}", args.config.display()))?;
    if let Some(port) = args.port {
        cfg.listen_addr.set_port(port);
    }

    let snapshot = Arc::new(Snapshot::new());
    let scraper = Scraper::new(&cfg, Arc::clone(&snapshot)).await?;
    let (stop_tx, stop_rx) = watch::channel(false);

    let serve_fut = serve(&cfg, snapshot, stop_rx.clone());
    let scraper_handle = scraper.run(stop_rx);
    let shutdown = async {
        shutdown_signal().await;
        info!("shutdown signal received");
        let _ = stop_tx.send(true);
    };
    tokio::pin!(serve_fut, scraper_handle, shutdown);

    let serve_result = tokio::select! {
        result = &mut serve_fut => result,
        scraper_result = &mut scraper_handle => {
            if scraper_result.is_err() {
                warn!("scraper task panicked or aborted");
            }
            warn!("scraper exited unexpectedly, shutting down");
            let _ = stop_tx.send(true);
            serve_fut.await
        }
        _ = &mut shutdown => {
            let _ = tokio::join!(&mut serve_fut, &mut scraper_handle);
            Ok(())
        }
    };
    serve_result?;
    info!("shutdown complete");
    Ok(())
}

/// 等待 SIGINT（Ctrl+C）或 SIGTERM（Unix）；Windows 仅支持 Ctrl+C。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
