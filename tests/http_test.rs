use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::http::router;
use redfish_exporter::metrics::Metric;
use redfish_exporter::registry::Snapshot;
use std::sync::Arc;
use tower::ServiceExt;

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
    )
    .await
    .unwrap();
    snap.update("bmc1", reg);
    let resp = router(snap)
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
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
    )
    .await
    .unwrap();
    snap.update("bmc1", reg1);
    snap.update("bmc2", reg2);
    let resp = router(snap)
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("redfish_up{bmc=\"bmc1\"} 1"), "{body}");
    assert!(body.contains("redfish_up{bmc=\"bmc2\"} 1"), "{body}");
}

#[tokio::test]
async fn healthz_returns_ok() {
    let snap = Arc::new(Snapshot::new());
    let resp = router(snap)
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn metrics_without_snapshot_returns_empty_with_header() {
    let snap = Arc::new(Snapshot::new());
    let resp = router(snap)
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
