use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::metrics::{Metric, encode};
use redfish_exporter::registry::{Snapshot, build_registry};
use std::sync::Arc;

#[tokio::test]
async fn snapshot_atomic_update_and_read() {
    let snap = Snapshot::new();
    assert!(snap.is_empty());
    assert!(snap.registry("bmc1").is_none());
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
    snap.update("bmc1", reg.clone());
    assert!(!snap.is_empty());
    assert!(Arc::ptr_eq(&snap.registry("bmc1").unwrap(), &reg));
    assert!(snap.registry("bmc2").is_none());
    assert_eq!(snap.registries().len(), 1);
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
