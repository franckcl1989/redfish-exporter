use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum_server::tls_rustls::RustlsConfig;
use tokio::sync::watch;
use tracing::info;

use crate::auth::bearer_authorized;
use crate::config::{Config, SecretString};
use crate::registry::Snapshot;

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// HTTP 路由共享状态：指标快照、预期 BMC 数和启动时 token。
#[derive(Clone)]
struct AppState {
    snapshot: Arc<Snapshot>,
    expected_bmcs: usize,
    auth_token: Option<SecretString>,
}

/// 构造 HTTP 路由：`/metrics`、`/healthz`、`/readyz`、`/info`。
pub fn router(
    snapshot: Arc<Snapshot>,
    expected_bmcs: usize,
    auth_token: Option<SecretString>,
) -> Router {
    let state = AppState {
        snapshot,
        expected_bmcs,
        auth_token,
    };
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler))
        .route("/readyz", get(readyz_handler))
        .route("/info", get(info_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// Bearer 认证中间件：探针端点始终公开，其余端点在配置 token 后要求认证。
/// 401 + `WWW-Authenticate: Bearer`；未配置 token 时直通。
async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let is_probe = matches!(req.uri().path(), "/healthz" | "/readyz");
    if !is_probe
        && let Some(token) = &state.auth_token
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
    let Some(body) = state.snapshot.encoded() else {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-redfish-exporter"),
            HeaderValue::from_static("no-data-yet"),
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
        );
        return (StatusCode::OK, headers, "").into_response();
    };
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

async fn readyz_handler(State(state): State<AppState>) -> Response {
    if state.snapshot.registry_count() >= state.expected_bmcs {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response()
    }
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
        while !*stop.borrow() {
            if stop.changed().await.is_err() {
                break;
            }
        }
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(25)));
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
    config: Arc<Config>,
    snapshot: Arc<Snapshot>,
    stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let listen_addr = config.listen_addr;
    let web = config.web.clone();
    let expected_bmcs = config.bmcs.len();
    let tls = load_tls(&web).await?;
    let listener = StdTcpListener::bind(listen_addr)
        .with_context(|| format!("failed to bind listen address {listen_addr}"))?;
    info!(addr = %listen_addr, tls = tls.is_some(), "http server listening");
    let auth_token = web.auth_token.clone();
    serve_on(
        listener,
        router(snapshot, expected_bmcs, auth_token),
        tls,
        DEFAULT_HEADER_READ_TIMEOUT,
        stop,
    )
    .await
}
