use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::sensors::collect_chassis_sensors;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

fn expect_service_root(bmc: &Mock) -> ODataId {
    let root_id = ODataId::service_root();
    bmc.expect(Expect::get(
        root_id.clone(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
        }),
    ));
    root_id
}

fn expect_chassis_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_sensor_payloads(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Sensors": { "@odata.id": "/redfish/v1/Chassis/1/Sensors" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Sensors",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Sensors",
            "@odata.type": "#SensorCollection.SensorCollection",
            "Name": "Sensors",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1/Sensors/Ambient" }],
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Sensors/Ambient",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Sensors/Ambient",
            "Id": "Ambient", "Name": "Ambient Temp",
            "Reading": 25.5,
            "ReadingType": "Temperature",
            "ReadingUnits": "Cel",
            "Status": { "Health": "OK", "State": "Enabled" },
            "Thresholds": {
                "UpperCritical": { "Reading": 60.0 },
                "UpperCaution": { "Reading": 50.0 },
            },
        }),
    ));
}

#[tokio::test]
async fn collects_sensor_readings_and_thresholds() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    expect_sensor_payloads(&bmc);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_chassis_sensors(bmc, &root, "bmc1").await.unwrap();
    let readings: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_sensor_reading")
        .collect();
    assert_eq!(readings.len(), 1);
    let r = &readings[0];
    assert_eq!(r.value, 25.5);
    let labels: HashMap<_, _> = r.labels.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("chassis"), Some(&"1"));
    assert_eq!(
        labels.get("id"),
        Some(&"/redfish/v1/Chassis/1/Sensors/Ambient")
    );
    assert_eq!(labels.get("name"), Some(&"Ambient"));
    assert_eq!(labels.get("units"), Some(&"Cel"));
    assert_eq!(labels.get("sensor_type"), Some(&"Temperature"));
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_sensor_threshold_upper_critical" && m.value == 60.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_sensor_threshold_upper_warning" && m.value == 50.0)
    );
}

#[tokio::test]
async fn missing_sensor_collection_is_ok() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_chassis_sensors(bmc, &root, "bmc1").await.unwrap();
    assert!(metrics.is_empty());
}

#[tokio::test]
async fn partial_sensor_fetch_failure_keeps_other_sensors() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Sensors": { "@odata.id": "/redfish/v1/Chassis/1/Sensors" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Sensors",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Sensors",
            "@odata.type": "#SensorCollection.SensorCollection",
            "Name": "Sensors",
            "Members": [
                { "@odata.id": "/redfish/v1/Chassis/1/Sensors/Ambient" },
                { "@odata.id": "/redfish/v1/Chassis/1/Sensors/Missing" },
            ],
            "Members@odata.count": 2,
        }),
    ));
    // 只为第一个 sensor 配置 GET；第二个 sensor fetch 时 mock 队列耗尽
    // （UnexpectedGet）→ 该 sensor 跳过，不影响第一个。
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Sensors/Ambient",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Sensors/Ambient",
            "Id": "Ambient", "Name": "Ambient Temp",
            "Reading": 25.5,
            "ReadingType": "Temperature",
            "ReadingUnits": "Cel",
            "Status": { "Health": "OK", "State": "Enabled" },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_chassis_sensors(bmc, &root, "bmc1").await.unwrap();
    let readings: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_sensor_reading")
        .collect();
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].value, 25.5);
    let labels: HashMap<_, _> = readings[0]
        .labels
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    assert_eq!(labels.get("name"), Some(&"Ambient"));
}

#[tokio::test]
async fn all_sensor_fetches_failing_returns_err() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Sensors": { "@odata.id": "/redfish/v1/Chassis/1/Sensors" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Sensors",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Sensors",
            "@odata.type": "#SensorCollection.SensorCollection",
            "Name": "Sensors",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1/Sensors/Ambient" }],
            "Members@odata.count": 1,
        }),
    ));
    // 不配置任何 sensor 的 GET：全部 fetch 失败 → 返回 Err 保持可见性。

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    match collect_chassis_sensors(bmc, &root, "bmc1").await {
        Err(err) => assert!(err.contains("sensor fetch"), "{err}"),
        Ok(_) => panic!("expected all sensor fetches to fail"),
    }
}

#[tokio::test]
async fn collect_groups_reports_up_duration_and_sensor_metrics() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    expect_sensor_payloads(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Sensors": { "@odata.id": "/redfish/v1/Chassis/1/Sensors" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let fast = collect_fast(Arc::clone(&bmc), &root, "bmc1").await.unwrap();
    let slow = collect_slow(Arc::clone(&bmc), &root, "bmc1").await.unwrap();
    let merged = merge_reports(fast, Some(&slow));
    let report = finalize_report(
        "bmc1",
        merged.metrics,
        merged.failed_resources,
        std::time::Instant::now(),
    );
    assert!(report.failed_resources.is_empty());
    assert!(
        report
            .metrics
            .iter()
            .any(|m| m.name == "redfish_up" && m.value == 1.0)
    );
    assert!(
        report
            .metrics
            .iter()
            .any(|m| m.name == "redfish_scrape_duration_seconds")
    );
    assert!(
        report
            .metrics
            .iter()
            .any(|m| m.name == "redfish_sensor_reading" && m.value == 25.5)
    );
}
