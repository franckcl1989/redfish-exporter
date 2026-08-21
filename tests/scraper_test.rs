use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::collector::merge_reports;
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

#[tokio::test(start_paused = true)]
async fn with_deadline_times_out() {
    use redfish_exporter::scraper::with_deadline;
    use std::time::Duration;
    let never = async {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        42
    };
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        with_deadline(Duration::from_secs(1), never),
    )
    .await;
    assert!(result.is_ok()); // 内部 deadline 先触发
    assert_eq!(result.unwrap(), None);
}

#[tokio::test]
async fn with_deadline_completes_in_time() {
    use redfish_exporter::scraper::with_deadline;
    use std::time::Duration;
    let ok = async { 7 };
    assert_eq!(with_deadline(Duration::from_secs(1), ok).await, Some(7));
}

#[tokio::test]
async fn merge_reports_combines_metrics_and_failed() {
    let fast = ScrapeReport {
        metrics: vec![Metric::gauge("a", "a").label("bmc", "b".into()).build(1.0)],
        failed_resources: vec!["sensors".into()],
    };
    let slow = ScrapeReport {
        metrics: vec![Metric::gauge("b", "b").label("bmc", "b".into()).build(2.0)],
        failed_resources: vec!["storage".into(), "sensors".into()],
    };
    let merged = merge_reports(fast, Some(&slow));
    assert_eq!(merged.metrics.len(), 2);
    assert_eq!(merged.failed_resources, vec!["sensors", "storage"]);
}
