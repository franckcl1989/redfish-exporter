use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tokio::sync::{RwLock, watch};
use tracing::info;

use crate::auth::bearer_authorized;
use crate::config::{Config, SecretString, load_config};
use crate::metrics::encode;
use crate::registry::Snapshot;

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// HTTP 路由共享状态：指标快照 + 运行时配置（/reload 热替换）+ 配置文件路径 + 启动时 token。
#[derive(Clone)]
struct AppState {
    snapshot: Arc<Snapshot>,
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    auth_token: Option<SecretString>,
}

/// 构造 HTTP 路由：`/metrics`、`/healthz`、`/info`、`/discover`、`/reload`。
pub fn router(
    snapshot: Arc<Snapshot>,
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    auth_token: Option<SecretString>,
) -> Router {
    let state = AppState {
        snapshot,
        config,
        config_path,
        auth_token,
    };
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler))
        .route("/info", get(info_handler))
        .route("/discover", get(discover_handler))
        .route("/reload", post(reload_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// Bearer 认证中间件：配置了 token 时全部端点（含 /healthz、/reload）统一要求认证。
/// 401 + `WWW-Authenticate: Bearer`；未配置 token 时直通（默认安全基线外的显式选择）。
async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(token) = &state.auth_token
        && !bearer_authorized(req.headers(), token.expose())
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
            "unauthorized",
        )
            .into_response();
    }
    next.run(req).await
}

async fn metrics_handler(State(state): State<AppState>) -> Response {
    if state.snapshot.is_empty() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-redfish-exporter"),
            HeaderValue::from_static("no-data-yet"),
        );
        return (StatusCode::OK, headers, "").into_response();
    }
    // TextEncoder 输出以换行结尾，各 BMC 快照直接拼接；
    // registries() 按 BMC 名排序，保证输出确定性。
    let mut body = String::new();
    for (_, registry) in state.snapshot.registries() {
        body.push_str(&encode(&registry));
    }
    (
        StatusCode::OK,
        [(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

async fn healthz_handler() -> &'static str {
    "ok"
}

/// 构建信息（版本、Rust 版本、目标 OS/架构），供运维定位部署产物。
async fn info_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "rust_version": env!("CARGO_PKG_RUST_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

/// Prometheus HTTP SD 格式：单条 entry，targets 为全部 BMC 的 host。
async fn discover_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = state.config.read().await;
    let targets: Vec<String> = cfg
        .bmcs
        .iter()
        .map(|b| b.host.as_str().to_string())
        .collect();
    Json(serde_json::json!([{ "targets": targets }]))
}

/// 重新读取并校验配置文件；成功后热替换运行时配置，失败返回 400 并保留旧配置。
/// 注意：BMC 增删/凭据变更需重启生效（scraper 不重建，见差距表）。
async fn reload_handler(State(state): State<AppState>) -> Response {
    let path = &state.config_path;
    match load_config(path) {
        Ok(new_cfg) => {
            *state.config.write().await = new_cfg;
            info!(path = %path.display(), "config reloaded");
            "ok".into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

/// 绑定 `cfg.listen_addr` 并启动 HTTP 服务；`stop` 触发后优雅关闭。
pub async fn serve(
    config: Arc<RwLock<Config>>,
    snapshot: Arc<Snapshot>,
    config_path: PathBuf,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let (listen_addr, auth_token) = {
        let cfg = config.read().await;
        (cfg.listen_addr, cfg.web.auth_token.clone())
    };
    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("failed to bind listen address {listen_addr}"))?;
    info!(addr = %listen_addr, "http server listening");
    axum::serve(listener, router(snapshot, config, config_path, auth_token))
        .with_graceful_shutdown(async move {
            let _ = stop.changed().await;
        })
        .await
        .context("http server error")?;
    Ok(())
}
