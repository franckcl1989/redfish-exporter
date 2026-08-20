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
