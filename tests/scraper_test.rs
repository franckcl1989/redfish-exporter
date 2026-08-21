use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::collector::merge_reports;
use redfish_exporter::metrics::{Metric, encode};
use redfish_exporter::registry::{Snapshot, build_registry};
use redfish_exporter::scraper::{apply_slow_result, merge_round, slow_due};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn registry_includes_build_info_and_error_total() {
    let reg = build_registry(
        "bmc1",
        &ScrapeReport {
            metrics: vec![],
            failed_resources: vec!["sensors".into()],
        },
        3,
    )
    .await
    .unwrap();
    let out = encode(&reg);
    assert!(out.contains("redfish_build_info"));
    assert!(out.contains("redfish_scrape_errors_total{bmc=\"bmc1\"} 3"));
}

#[tokio::test]
async fn snapshot_tracks_scrape_errors() {
    let snap = Snapshot::new();
    snap.record_scrape_errors("bmc1", 2);
    snap.record_scrape_errors("bmc1", 1);
    assert_eq!(snap.scrape_errors("bmc1"), 3);
}

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
        0,
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
        0,
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

fn slow_report(value: f64) -> ScrapeReport {
    ScrapeReport {
        metrics: vec![Metric::gauge("slow_metric", "s").build(value)],
        failed_resources: vec![],
    }
}

#[test]
fn slow_due_never_without_interval() {
    assert!(!slow_due(None, None));
    assert!(!slow_due(Some(Instant::now()), None));
}

#[test]
fn slow_due_true_on_first_round_without_subtraction() {
    assert!(slow_due(None, Some(Duration::from_secs(48 * 3600))));
}

#[test]
fn slow_due_true_after_interval_elapsed() {
    let last = Instant::now() - Duration::from_millis(50);
    assert!(slow_due(Some(last), Some(Duration::from_millis(10))));
}

#[test]
fn slow_due_false_before_interval_elapsed() {
    assert!(!slow_due(
        Some(Instant::now()),
        Some(Duration::from_millis(10))
    ));
}

#[test]
fn apply_slow_result_success_caches_report() {
    let (cache, failed) = apply_slow_result(None, Ok(slow_report(1.0)), Instant::now());
    assert!(failed.is_empty());
    let (_, report) = cache.expect("cached");
    assert_eq!(report.expect("report").metrics[0].value, 1.0);
}

#[test]
fn apply_slow_result_failure_keeps_last_good_cache() {
    let cache = Some((Instant::now(), Some(slow_report(1.0))));
    let (new_cache, failed) = apply_slow_result(cache, Err("storage".into()), Instant::now());
    assert_eq!(failed, vec!["storage"]);
    let (_, report) = new_cache.expect("last good kept");
    assert_eq!(report.expect("report").metrics[0].value, 1.0);
}

#[test]
fn apply_slow_result_first_round_failure_caches_nothing() {
    let (new_cache, failed) = apply_slow_result(None, Err("storage".into()), Instant::now());
    assert_eq!(failed, vec!["storage"]);
    assert!(new_cache.is_none());
}

#[test]
fn slow_failure_keeps_last_good_metrics_in_merged_output() {
    let fast = ScrapeReport {
        metrics: vec![Metric::gauge("fast_metric", "f").build(3.0)],
        failed_resources: vec![],
    };
    let cache = Some((Instant::now(), Some(slow_report(1.0))));
    let (new_cache, failed) = apply_slow_result(cache, Err("storage".into()), Instant::now());
    let merged = merge_round(
        fast,
        new_cache.as_ref().and_then(|(_, r)| r.as_ref()),
        failed,
    );
    assert!(merged.metrics.iter().any(|m| m.name == "fast_metric"));
    assert!(merged.metrics.iter().any(|m| m.name == "slow_metric"));
    assert_eq!(merged.failed_resources, vec!["storage"]);
}

#[test]
fn merge_round_dedupes_slow_failure_against_merged_failures() {
    let fast = ScrapeReport {
        metrics: vec![],
        failed_resources: vec!["storage".into()],
    };
    let merged = merge_round(fast, None, vec!["storage".into()]);
    assert_eq!(merged.failed_resources, vec!["storage"]);
}
