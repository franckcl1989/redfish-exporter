use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::sync::watch;
use tracing::info;

use crate::config::Config;
use crate::metrics::encode;
use crate::registry::Snapshot;

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// 构造 HTTP 路由：`GET /metrics` 返回当前指标快照，`GET /healthz` 返回 ok。
pub fn router(snapshot: Arc<Snapshot>) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler))
        .with_state(snapshot)
}

async fn metrics_handler(State(snapshot): State<Arc<Snapshot>>) -> Response {
    match snapshot.registry() {
        Some(registry) => (
            StatusCode::OK,
            [(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
            encode(&registry),
        )
            .into_response(),
        None => {
            let mut headers = HeaderMap::new();
            headers.insert(
                HeaderName::from_static("x-redfish-exporter"),
                HeaderValue::from_static("no-data-yet"),
            );
            (StatusCode::OK, headers, "").into_response()
        }
    }
}

async fn healthz_handler() -> &'static str {
    "ok"
}

/// 绑定 `cfg.listen_addr` 并启动 HTTP 服务；`stop` 触发后优雅关闭。
pub async fn serve(
    cfg: &Config,
    snapshot: Arc<Snapshot>,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(cfg.listen_addr)
        .await
        .with_context(|| format!("failed to bind listen address {}", cfg.listen_addr))?;
    info!(addr = %cfg.listen_addr, "http server listening");
    axum::serve(listener, router(snapshot))
        .with_graceful_shutdown(async move {
            let _ = stop.changed().await;
        })
        .await
        .context("http server error")?;
    Ok(())
}
