use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum_server::tls_rustls::RustlsConfig;
use tokio::sync::{RwLock, watch};
use tracing::info;

use crate::auth::bearer_authorized;
use crate::config::{Config, SecretString, load_config};
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
    // 预编码字节直接拼接（发布时已编码）；registries() 按 BMC 名排序，输出确定性不变。
    let entries = state.snapshot.registries();
    let total: usize = entries.iter().map(|(_, e)| e.encoded.len()).sum();
    let mut body: Vec<u8> = Vec::with_capacity(total);
    for (_, entry) in &entries {
        body.extend_from_slice(&entry.encoded);
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

/// 默认 HTTP/1.1 header 读取超时（慢连接防护，spec §3.1）。
pub const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// 读取 web 节的 TLS 配置；未配置返回 Ok(None)（纯 HTTP）。失败即启动失败（fail-fast）。
pub async fn load_tls(web: &crate::config::WebConfig) -> anyhow::Result<Option<RustlsConfig>> {
    match (&web.tls_cert_file, &web.tls_key_file) {
        (Some(cert), Some(key)) => {
            let cfg = RustlsConfig::from_pem_file(cert, key)
                .await
                .with_context(|| {
                    format!(
                        "failed to load TLS cert '{}' / key '{}'",
                        cert.display(),
                        key.display()
                    )
                })?;
            // 服务端固定 HTTP/1.1（见 http1_server），ALPN 不再声明 h2，
            // 避免客户端协商到服务端不支持的协议。
            let mut server_config = (*cfg.get_inner()).clone();
            server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
            cfg.reload_from_config(Arc::new(server_config));
            Ok(Some(cfg))
        }
        (None, None) => Ok(None),
        _ => Err(anyhow::anyhow!(
            "web: tls_cert_file and tls_key_file must be set together (validated in load_config)"
        )),
    }
}

/// 在已绑定的 listener 上服务（测试接缝：测试自建 listener 以获取端口 0 的实际地址）。
/// stop 触发后优雅停机（axum-server Handle）。
pub async fn serve_on(
    listener: StdTcpListener,
    router: Router,
    tls: Option<RustlsConfig>,
    header_read_timeout: Duration,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    // Windows：tokio/mio 的 from_std 要求 socket 已处于非阻塞模式，
    // 否则 accept 出来的连接在 IOCP 下 I/O 永久挂起（blocking listener 直通）。
    listener
        .set_nonblocking(true)
        .context("failed to set listener nonblocking")?;
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        let _ = stop.changed().await;
        shutdown_handle.graceful_shutdown(None);
    });
    match tls {
        // TLS 与纯 HTTP 语义一致：header 读超时覆盖"握手/连接后零字节"的慢连接。
        Some(cfg) => http1_server(
            axum_server::from_tcp_rustls(listener, cfg).context("failed to start TLS server")?,
            header_read_timeout,
        )
        .handle(handle)
        .serve(router.into_make_service())
        .await
        .context("http server error"),
        None => http1_server(
            axum_server::from_tcp(listener).context("failed to start server")?,
            header_read_timeout,
        )
        .handle(handle)
        .serve(router.into_make_service())
        .await
        .context("http server error"),
    }
}

/// 固定 HTTP/1.1 并配置 header 读超时（慢连接防护）。两种模式共用：
/// hyper-util 的版本探测阶段（读前 24 字节区分 h2 前言）没有超时，
/// 若不固定，则"握手/连接后零字节"的慢连接（slowloris）永远不会被关闭。
/// 代价：不再服务 h2/h2c（Prometheus 走 HTTP/1.1）。
fn http1_server<Acc>(
    mut server: axum_server::Server<std::net::SocketAddr, Acc>,
    header_read_timeout: Duration,
) -> axum_server::Server<std::net::SocketAddr, Acc> {
    server = server.http1_only();
    server
        .http_builder()
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(Some(header_read_timeout));
    server
}

/// 绑定 `cfg.listen_addr` 并启动 HTTP 服务；`stop` 触发后优雅关闭。
pub async fn serve(
    config: Arc<RwLock<Config>>,
    snapshot: Arc<Snapshot>,
    config_path: PathBuf,
    stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let (listen_addr, web) = {
        let cfg = config.read().await;
        (cfg.listen_addr, cfg.web.clone())
    };
    let tls = load_tls(&web).await?;
    let listener = StdTcpListener::bind(listen_addr)
        .with_context(|| format!("failed to bind listen address {listen_addr}"))?;
    info!(addr = %listen_addr, tls = tls.is_some(), "http server listening");
    let auth_token = web.auth_token.clone();
    serve_on(
        listener,
        router(snapshot, config, config_path, auth_token),
        tls,
        DEFAULT_HEADER_READ_TIMEOUT,
        stop,
    )
    .await
}
