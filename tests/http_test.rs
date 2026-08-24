use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::config::{
    AuthMethod, BmcConfig, Config, SecretString, WebConfig, load_config,
};
use redfish_exporter::http::router;
use redfish_exporter::metrics::Metric;
use redfish_exporter::registry::Snapshot;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tower::ServiceExt;
use url::Url;

const CONFIG_A: &str = "\
bmcs:
  - name: bmc1
    host: https://10.0.0.1
    username: root
    password: secret
";

const CONFIG_B: &str = "\
bmcs:
  - name: bmc1
    host: https://10.0.0.2
    username: root
    password: secret
";

fn bmc(name: &str, host: &str) -> BmcConfig {
    BmcConfig {
        name: name.into(),
        host: Url::parse(host).unwrap(),
        username: "root".into(),
        password: SecretString::new("secret".into()),
        auth: AuthMethod::Basic,
        insecure_skip_verify: false,
        ca_cert_file: None,
    }
}

fn test_config(hosts: &[(&str, &str)]) -> Config {
    Config {
        listen_addr: "127.0.0.1:9417".parse().unwrap(),
        scrape_interval: Duration::from_secs(30),
        slow_interval: None,
        scrape_timeout: Duration::from_secs(15),
        request_timeout: Duration::from_secs(10),
        bmcs: hosts.iter().map(|(n, h)| bmc(n, h)).collect(),
        web: WebConfig::default(),
        stability: redfish_exporter::config::StabilityConfig {
            cooldown_failures: 3,
            cooldown_base: Duration::from_secs(60),
            cooldown_max: Duration::from_secs(300),
        },
    }
}

fn test_router(snapshot: Arc<Snapshot>, cfg: Config) -> Router {
    router(
        snapshot,
        Arc::new(RwLock::new(cfg)),
        PathBuf::from("config.yaml"),
        None,
    )
}

fn test_router_with_token(snapshot: Arc<Snapshot>, cfg: Config, token: &str) -> Router {
    router(
        snapshot,
        Arc::new(RwLock::new(cfg)),
        PathBuf::from("config.yaml"),
        Some(SecretString::new(token.into())),
    )
}

fn temp_config_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "redfish-exporter-http-test-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

async fn body_text(resp: Response) -> String {
    String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn metrics_endpoint_returns_cached_snapshot() {
    let snap = Arc::new(Snapshot::new());
    let reg = redfish_exporter::registry::build_registry(
        "bmc1",
        &ScrapeReport {
            metrics: vec![
                Metric::gauge("redfish_up", "up")
                    .label("bmc", "bmc1".into())
                    .build(1.0),
            ],
            failed_resources: vec![],
        },
        0,
    )
    .await
    .unwrap();
    snap.update("bmc1", reg);
    let resp = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]))
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    assert!(body.contains("redfish_up"));
}

#[tokio::test]
async fn two_bmcs_both_served() {
    let snap = Arc::new(Snapshot::new());
    let reg1 = redfish_exporter::registry::build_registry(
        "bmc1",
        &ScrapeReport {
            metrics: vec![
                Metric::gauge("redfish_up", "up")
                    .label("bmc", "bmc1".into())
                    .build(1.0),
            ],
            failed_resources: vec![],
        },
        0,
    )
    .await
    .unwrap();
    let reg2 = redfish_exporter::registry::build_registry(
        "bmc2",
        &ScrapeReport {
            metrics: vec![
                Metric::gauge("redfish_up", "up")
                    .label("bmc", "bmc2".into())
                    .build(1.0),
            ],
            failed_resources: vec![],
        },
        0,
    )
    .await
    .unwrap();
    snap.update("bmc1", reg1);
    snap.update("bmc2", reg2);
    let resp = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]))
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    assert!(body.contains("redfish_up{bmc=\"bmc1\"} 1"), "{body}");
    assert!(body.contains("redfish_up{bmc=\"bmc2\"} 1"), "{body}");
}

#[tokio::test]
async fn healthz_returns_ok() {
    let snap = Arc::new(Snapshot::new());
    let resp = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]))
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn metrics_without_snapshot_returns_empty_with_header() {
    let snap = Arc::new(Snapshot::new());
    let resp = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]))
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-redfish-exporter")
            .map(|v| v.to_str().unwrap()),
        Some("no-data-yet")
    );
}

#[tokio::test]
async fn info_endpoint_reports_build_info() {
    let snap = Arc::new(Snapshot::new());
    let resp = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]))
        .oneshot(Request::builder().uri("/info").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "application/json");
    let json: serde_json::Value = serde_json::from_str(&body_text(resp).await).unwrap();
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert!(json.get("rust_version").is_some());
    assert!(json.get("os").is_some());
    assert!(json.get("arch").is_some());
}

#[tokio::test]
async fn discover_endpoint_lists_targets() {
    let snap = Arc::new(Snapshot::new());
    let resp = test_router(
        snap,
        test_config(&[
            ("bmc1", "https://10.0.0.1"),
            ("bmc2", "https://10.0.0.2:8443"),
        ]),
    )
    .oneshot(
        Request::builder()
            .uri("/discover")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "application/json");
    let body = body_text(resp).await;
    assert!(body.contains("https://10.0.0.1"), "{body}");
    assert!(body.contains("https://10.0.0.2:8443"), "{body}");
}

#[tokio::test]
async fn reload_endpoint_swaps_config() {
    let dir = temp_config_dir();
    let path = dir.join("config.yaml");
    std::fs::write(&path, CONFIG_A).unwrap();
    let cfg = Arc::new(RwLock::new(load_config(&path).unwrap()));
    let snap = Arc::new(Snapshot::new());
    let app = router(snap, cfg, path.clone(), None);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/discover")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_text(resp).await;
    assert!(body.contains("https://10.0.0.1"), "{body}");

    std::fs::write(&path, CONFIG_B).unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_text(resp).await, "ok");

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/discover")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_text(resp).await;
    assert!(body.contains("https://10.0.0.2"), "{body}");
    assert!(!body.contains("https://10.0.0.1"), "{body}");
    cleanup(&dir);
}

#[tokio::test]
async fn reload_rejects_invalid_config() {
    let dir = temp_config_dir();
    let path = dir.join("config.yaml");
    std::fs::write(&path, CONFIG_A).unwrap();
    let cfg = Arc::new(RwLock::new(load_config(&path).unwrap()));
    let snap = Arc::new(Snapshot::new());
    let app = router(snap, cfg, path.clone(), None);

    std::fs::write(&path, "bmcs: [broken").unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_text(resp).await;
    assert!(body.contains("failed to parse config"), "{body}");

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/discover")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_text(resp).await;
    assert!(body.contains("https://10.0.0.1"), "{body}");
    cleanup(&dir);
}

const TEST_TOKEN: &str = "0123456789abcdef";

#[tokio::test]
async fn auth_missing_token_returns_401_with_challenge() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(
        snap,
        test_config(&[("bmc1", "https://10.0.0.1")]),
        TEST_TOKEN,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers()["www-authenticate"], "Bearer");
}

#[tokio::test]
async fn auth_wrong_token_returns_401() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(
        snap,
        test_config(&[("bmc1", "https://10.0.0.1")]),
        TEST_TOKEN,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("authorization", "Bearer fedcba9876543210")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// RFC 6750：scheme 区分大小写，小写 `bearer` 必须被拒绝（401）。
#[tokio::test]
async fn auth_lowercase_bearer_scheme_is_rejected() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(
        snap,
        test_config(&[("bmc1", "https://10.0.0.1")]),
        TEST_TOKEN,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("authorization", format!("bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_correct_token_grants_all_get_endpoints() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(
        snap,
        test_config(&[("bmc1", "https://10.0.0.1")]),
        TEST_TOKEN,
    );
    for path in ["/metrics", "/healthz", "/info", "/discover"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "endpoint {path}");
    }
}

#[tokio::test]
async fn auth_correct_token_grants_reload() {
    let dir = temp_config_dir();
    let path = dir.join("config.yaml");
    std::fs::write(&path, CONFIG_A).unwrap();
    let cfg = Arc::new(RwLock::new(load_config(&path).unwrap()));
    let snap = Arc::new(Snapshot::new());
    let app = router(
        snap,
        cfg,
        path.clone(),
        Some(SecretString::new(TEST_TOKEN.into())),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reload")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    cleanup(&dir);
}

#[tokio::test]
async fn auth_disabled_keeps_open_access() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn load_tls_rejects_invalid_pem() {
    let dir = temp_config_dir();
    std::fs::write(dir.join("cert.pem"), "not a pem").unwrap();
    std::fs::write(dir.join("key.pem"), "also not a pem").unwrap();
    let web = WebConfig {
        auth_token: None,
        tls_cert_file: Some(dir.join("cert.pem")),
        tls_key_file: Some(dir.join("key.pem")),
    };
    assert!(redfish_exporter::http::load_tls(&web).await.is_err());
    cleanup(&dir);
}

#[tokio::test]
async fn tls_end_to_end_with_self_signed_cert() {
    let dir = temp_config_dir();
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    let web = WebConfig {
        auth_token: None,
        tls_cert_file: Some(cert_path),
        tls_key_file: Some(key_path),
    };
    let tls = redfish_exporter::http::load_tls(&web)
        .await
        .unwrap()
        .expect("tls loaded");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move {
        redfish_exporter::http::serve_on(listener, app, Some(tls), Duration::from_secs(10), rx)
            .await
    });
    // 等待监听就绪（TCP 连接探测）
    let mut connected = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(addr).is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(connected, "server did not start");
    // 信任证书的客户端成功
    let trusting = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(cert.cert.pem().as_bytes()).unwrap())
        .build()
        .unwrap();
    let resp = trusting
        .get(format!("https://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ok");
    // 不信任证书的客户端失败（TLS 实际生效）
    let plain = reqwest::Client::new();
    assert!(
        plain
            .get(format!("https://{addr}/healthz"))
            .send()
            .await
            .is_err()
    );
    let _ = tx.send(true);
    handle.await.unwrap().unwrap();
    cleanup(&dir);
}

// 多线程 runtime：测试线程会阻塞在 std TcpStream::read() 上，
// 单线程 runtime 下服务端任务无法推进，header 超时永远不会触发。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn header_read_timeout_closes_idle_connection() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move {
        redfish_exporter::http::serve_on(listener, app, None, Duration::from_millis(300), rx).await
    });
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0u8; 16];
    let started = std::time::Instant::now();
    let n = stream.read(&mut buf);
    let elapsed = started.elapsed();
    assert!(
        matches!(n, Ok(0)),
        "expected clean EOF from server, got {n:?}"
    );
    assert!(elapsed < Duration::from_secs(2), "timeout took {elapsed:?}");
    let _ = tx.send(true);
    handle.await.unwrap().unwrap();
}

// TLS 模式下的零字节慢连接：完成 TLS 握手后不发送任何字节，
// header 读超时同样必须在 ~300ms 内关闭连接（与纯 HTTP 分支语义一致）。
#[tokio::test]
async fn tls_header_read_timeout_closes_idle_connection() {
    let dir = temp_config_dir();
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    let web = WebConfig {
        auth_token: None,
        tls_cert_file: Some(cert_path),
        tls_key_file: Some(key_path),
    };
    let tls = redfish_exporter::http::load_tls(&web)
        .await
        .unwrap()
        .expect("tls loaded");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move {
        redfish_exporter::http::serve_on(listener, app, Some(tls), Duration::from_millis(300), rx)
            .await
    });
    // 信任自签证书的 TLS 客户端
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(cert.cert.der().to_vec()))
        .unwrap();
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut stream = connector
        .connect(ServerName::try_from("127.0.0.1".to_string()).unwrap(), tcp)
        .await
        .unwrap();
    // 握手完成后不发送任何字节，等待服务端关闭
    let mut buf = [0u8; 16];
    let started = std::time::Instant::now();
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await;
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(2), "timeout took {elapsed:?}");
    let closed = match &n {
        // 干净 EOF（close_notify）或 rustls 的 unexpected EOF（对端直接断开、无 close_notify）
        // 视为服务端已关闭；外层超时或其他 I/O 错误不允许。
        Ok(Ok(0)) => true,
        Ok(Err(e)) => e.kind() == std::io::ErrorKind::UnexpectedEof,
        _ => false,
    };
    assert!(
        closed,
        "expected connection closed by server (EOF), got {n:?}"
    );
    let _ = tx.send(true);
    handle.await.unwrap().unwrap();
    cleanup(&dir);
}
