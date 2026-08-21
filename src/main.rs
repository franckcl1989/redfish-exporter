#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::sync::{RwLock, watch};
use tracing::{debug, error, info};
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

    let cfg = Arc::new(RwLock::new(cfg));
    let snapshot = Arc::new(Snapshot::new());
    let scraper_cfg = cfg.read().await;
    let scraper = Scraper::new(&scraper_cfg, Arc::clone(&snapshot)).await?;
    drop(scraper_cfg);
    let (stop_tx, stop_rx) = watch::channel(false);

    let serve_fut = serve(cfg, snapshot, args.config, stop_rx.clone());
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
            // 正常 shutdown 时 stop 信号已发出，scraper 正常 break（Ok），
            // 此时与 shutdown 分支竞争触发本分支：仅记 debug。
            match scraper_result {
                Ok(()) => {
                    debug!("scraper task ended without stop signal");
                    let _ = stop_tx.send(true);
                    serve_fut.await
                }
                Err(e) => {
                    error!(error = %e, "scraper task panicked or aborted");
                    std::process::exit(1);
                }
            }
        }
        _ = &mut shutdown => {
            let (serve_result, scraper_result) = tokio::join!(&mut serve_fut, &mut scraper_handle);
            if let Err(e) = scraper_result {
                error!(error = %e, "scraper task panicked or aborted");
                std::process::exit(1);
            }
            serve_result
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
