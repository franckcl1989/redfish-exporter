use redfish_exporter::collector::finalize_report;
use redfish_exporter::metrics::{
    Metric, MetricsError, SENSOR_READING, encode, register_into, unbox_reading,
};
use std::time::Instant;

#[test]
fn builds_gauge_metric() {
    let m = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "Ambient".into())
        .build(25.5);
    assert_eq!(m.name, "redfish_sensor_reading");
    assert_eq!(m.value, 25.5);
    assert_eq!(m.labels.len(), 2);
}

#[test]
fn unboxes_double_option() {
    assert_eq!(unbox_reading(Some(Some(12.0))), Some(12.0));
    assert_eq!(unbox_reading(Some(None)), None);
    assert_eq!(unbox_reading(None), None);
}

#[test]
fn register_and_encode_roundtrip() {
    let reg = prometheus::Registry::new();
    let m = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "A".into())
        .build(1.0);
    register_into(&[m], &reg).unwrap();
    let out = encode(&reg);
    assert!(out.contains("redfish_sensor_reading"));
    assert!(out.contains("bmc=\"bmc1\""));
    assert!(out.contains("1"));
}

#[test]
fn same_family_with_distinct_label_values_is_merged() {
    // 同名+同 label 集、不同 label 值的样本合并到同一指标族。
    let reg = prometheus::Registry::new();
    let m1 = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "B".into())
        .build(1.0);
    let m2 = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "A".into())
        .build(2.0);
    register_into(&[m1, m2], &reg).unwrap();
    let out = encode(&reg);
    assert_eq!(out.matches("redfish_sensor_reading{").count(), 2);
}

#[test]
fn counter_metrics_are_exposed_as_counters() {
    let registry = prometheus::Registry::new();
    let metric = Metric::counter("redfish_test_total", "test counter")
        .label("bmc", "one".into())
        .build(7.0);
    register_into(&[metric], &registry).unwrap();
    let text = encode(&registry);
    assert!(text.contains("# TYPE redfish_test_total counter"), "{text}");
    assert!(text.contains("redfish_test_total{bmc=\"one\"} 7"), "{text}");
}

#[test]
fn conflicting_metric_descriptors_are_rejected() {
    let registry = prometheus::Registry::new();
    let result = register_into(
        &[
            Metric::gauge("redfish_conflict", "first").build(1.0),
            Metric::gauge("redfish_conflict", "second").build(2.0),
        ],
        &registry,
    );
    assert!(matches!(
        result,
        Err(MetricsError::ConflictingDescriptor { .. })
    ));
}

#[test]
fn duplicate_label_keys_are_rejected_in_release_too() {
    let registry = prometheus::Registry::new();
    let metric = Metric::gauge("redfish_duplicate_label", "test")
        .label("bmc", "first".into())
        .label("bmc", "second".into())
        .build(1.0);
    assert!(matches!(
        register_into(&[metric], &registry),
        Err(MetricsError::DuplicateLabel { .. })
    ));
}

#[test]
fn duplicate_series_are_rejected_instead_of_silently_overwritten() {
    let registry = prometheus::Registry::new();
    let metrics = vec![
        Metric::gauge("duplicate_series", "help")
            .label("id", "same".into())
            .build(1.0),
        Metric::gauge("duplicate_series", "help")
            .label("id", "same".into())
            .build(2.0),
    ];
    assert!(matches!(
        register_into(&metrics, &registry),
        Err(MetricsError::DuplicateSeries { .. })
    ));
}

#[test]
fn scrape_finalization_deduplicates_the_same_resource_reached_by_two_paths() {
    let labels = |value: f64| {
        Metric::gauge("redfish_sensor_reading", "Sensor reading")
            .label("bmc", "b1".into())
            .label("chassis", "1".into())
            .label("id", "/redfish/v1/Chassis/1/Sensors/Ambient".into())
            .build(value)
    };
    let report = finalize_report(
        "b1",
        vec![labels(25.0), labels(26.0)],
        vec![],
        Instant::now(),
    );
    let readings: Vec<_> = report
        .metrics
        .iter()
        .filter(|metric| metric.name == "redfish_sensor_reading")
        .collect();
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].value, 25.0, "first navigation path wins");
}

#[test]
fn register_into_label_lookup_equivalent_for_mixed_label_sets() {
    // 同一指标名下不同 label 集合（模拟多资源序列）：缺失 label 以空值兜底，注册不冲突。
    let registry = prometheus::Registry::new();
    let m1 = Metric::gauge("redfish_health_status", "h")
        .label("bmc", "b1".into())
        .label("resource_type", "system".into())
        .build(1.0);
    let m2 = Metric::gauge("redfish_health_status", "h")
        .label("bmc", "b2".into())
        .build(1.0);
    redfish_exporter::metrics::register_into(&[m1, m2], &registry).unwrap();
    let out = encode(&registry);
    assert!(
        out.contains("redfish_health_status{bmc=\"b1\",resource_type=\"system\"} 1"),
        "{out}"
    );
    assert!(
        out.contains("redfish_health_status{bmc=\"b2\",resource_type=\"\"} 1"),
        "{out}"
    );
}
