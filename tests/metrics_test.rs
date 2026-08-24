use redfish_exporter::metrics::{Metric, SENSOR_READING, encode, register_into, unbox_reading};

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
fn duplicate_labels_are_merged() {
    // 同名+同 label 集的指标合并：结果只有一个指标序列
    let reg = prometheus::Registry::new();
    let m1 = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "A".into())
        .build(1.0);
    let m2 = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "A".into())
        .build(2.0);
    register_into(&[m1, m2], &reg).unwrap();
    let out = encode(&reg);
    assert_eq!(out.matches("redfish_sensor_reading{").count(), 1);
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
