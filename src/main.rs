#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::EnvFilter;

use redfish_exporter::config;
use redfish_exporter::http::serve;
use redfish_exporter::registry::Snapshot;
use redfish_exporter::scraper::Scraper;

#[derive(clap::Parser)]
#[command(version, about)]
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
    let filter = if std::env::var_os("RUST_LOG").is_some() {
        EnvFilter::try_from_default_env().context("invalid RUST_LOG filter")?
    } else {
        EnvFilter::try_new(&args.log_level)
            .with_context(|| format!("invalid --log-level filter: {}", args.log_level))?
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut cfg = config::load_config(&args.config)
        .with_context(|| format!("failed to load config from {}", args.config.display()))?;
    #[cfg(unix)]
    if config::config_file_is_wide_open(&args.config) {
        tracing::warn!(
            path = %args.config.display(),
            "config file is readable by group/others; consider chmod 600"
        );
    }
    if let Some(port) = args.port {
        cfg.listen_addr.set_port(port);
    }

    let cfg = Arc::new(cfg);
    let snapshot = Arc::new(Snapshot::new());
    let scraper = Scraper::new(&cfg, Arc::clone(&snapshot))?;
    let (stop_tx, stop_rx) = watch::channel(false);

    let serve_fut = serve(cfg, snapshot, stop_rx.clone());
    let scraper_handle = scraper.run(stop_rx);
    tokio::pin!(serve_fut, scraper_handle);

    enum Exit {
        Signal,
        Http(anyhow::Result<()>),
        Scraper(Result<(), tokio::task::JoinError>),
    }

    let exit = tokio::select! {
        result = &mut serve_fut => Exit::Http(result),
        result = &mut scraper_handle => Exit::Scraper(result),
        result = shutdown_signal() => {
            result?;
            info!("shutdown signal received");
            Exit::Signal
        }
    };
    let _ = stop_tx.send(true);

    const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    match exit {
        Exit::Signal => {
            // 两边都依赖 stop watch 推进退出；并发轮询，避免串行等待把总退出时长
            // 放大到 2 × SHUTDOWN_TIMEOUT，也确保 HTTP graceful shutdown 立即被驱动。
            let (scraper_result, http_result) = tokio::join!(
                tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut scraper_handle),
                tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut serve_fut),
            );
            scraper_result
                .context("scraper shutdown timed out")?
                .context("scraper task panicked or was aborted")?;
            http_result.context("HTTP shutdown timed out")??;
        }
        Exit::Http(result) => {
            let scraper_result = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut scraper_handle)
                .await
                .context("scraper cleanup timed out after HTTP server exit")?;
            scraper_result.context("scraper task panicked or was aborted")?;
            result?;
            anyhow::bail!("HTTP server stopped unexpectedly");
        }
        Exit::Scraper(result) => {
            // 即使 scraper panic，也先完成 HTTP graceful shutdown，再返回根因。
            let scraper_result = result.context("scraper task panicked or was aborted");
            let http_result = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut serve_fut)
                .await
                .context("HTTP shutdown timed out after scraper exit")
                .and_then(|result| result);
            scraper_result?;
            http_result?;
            anyhow::bail!("scraper stopped unexpectedly");
        }
    }
    info!("shutdown complete");
    Ok(())
}

/// 等待 SIGINT（Ctrl+C）或 SIGTERM（Unix）；Windows 仅支持 Ctrl+C。
async fn shutdown_signal() -> anyhow::Result<()> {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .context("failed to install Ctrl+C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .context("failed to install SIGTERM handler")?;
        signal.recv().await;
        Ok::<(), anyhow::Error>(())
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<anyhow::Result<()>>();
    tokio::select! {
        result = ctrl_c => result,
        result = terminate => result,
    }
}
