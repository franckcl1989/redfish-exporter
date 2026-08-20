use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::metrics::{Metric, encode};
use redfish_exporter::registry::{Snapshot, build_registry};
use std::sync::Arc;

#[tokio::test]
async fn snapshot_atomic_update_and_read() {
    let snap = Snapshot::new();
    assert!(snap.registry().is_none());
    let reg = build_registry(
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
    snap.update(reg.clone());
    assert!(Arc::ptr_eq(&snap.registry().unwrap(), &reg));
}

#[tokio::test]
async fn build_registry_includes_error_metrics() {
    let reg = build_registry(
        "bmc1",
        &ScrapeReport {
            metrics: vec![],
            failed_resources: vec!["sensors".into()],
        },
    )
    .await
    .unwrap();
    let out = encode(&reg);
    assert!(out.contains("redfish_up") && out.contains("redfish_up{bmc=\"bmc1\"} 0"));
    assert!(out.contains("redfish_scrape_error") && out.contains("sensors"));
    assert!(out.contains("bmc=\"bmc1\""));
}
