use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::config::{AuthMethod, BmcConfig, Config, SecretString, load_config};
use redfish_exporter::http::router;
use redfish_exporter::metrics::Metric;
use redfish_exporter::registry::Snapshot;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
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
    }
}

fn test_router(snapshot: Arc<Snapshot>, cfg: Config) -> Router {
    router(
        snapshot,
        Arc::new(RwLock::new(cfg)),
        PathBuf::from("config.yaml"),
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
    let app = router(snap, cfg, path.clone());

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
    let app = router(snap, cfg, path.clone());

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
