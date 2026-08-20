use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::power::collect_power_metrics;
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

#[tokio::test]
async fn collects_legacy_thermal_readings() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Thermal": { "@odata.id": "/redfish/v1/Chassis/1/Thermal" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Thermal",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Thermal",
            "Id": "Thermal", "Name": "Thermal",
            "Temperatures": [{
                "@odata.id": "/redfish/v1/Chassis/1/Thermal#/Temperatures/1",
                "MemberId": "1", "Name": "CPU1",
                "ReadingCelsius": 42.0,
                "UpperThresholdCritical": 80.0,
                "Status": { "Health": "OK", "State": "Enabled" },
            }],
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_power_metrics(bmc, &root, "bmc1").await.unwrap();
    let readings: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_sensor_reading")
        .collect();
    assert_eq!(readings.len(), 1);
    let r = readings[0];
    assert_eq!(r.value, 42.0);
    let labels: HashMap<_, _> = r.labels.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("chassis"), Some(&"1"));
    assert_eq!(labels.get("name"), Some(&"CPU1"));
    assert_eq!(labels.get("units"), Some(&"Cel"));
    assert_eq!(labels.get("sensor_type"), Some(&"Temperature"));
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_sensor_threshold_upper_critical" && m.value == 80.0)
    );
}

#[tokio::test]
async fn collects_legacy_power_consumption() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Power": { "@odata.id": "/redfish/v1/Chassis/1/Power" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Power",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Power",
            "Id": "Power", "Name": "Power",
            "PowerControl": [{
                "@odata.id": "/redfish/v1/Chassis/1/Power#/PowerControl/1",
                "MemberId": "1", "Name": "Total",
                "PowerConsumedWatts": 320.0,
            }],
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_power_metrics(bmc, &root, "bmc1").await.unwrap();
    let consumption: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_power_consumption_watts")
        .collect();
    assert_eq!(consumption.len(), 1);
    let c = consumption[0];
    assert_eq!(c.value, 320.0);
    assert_eq!(c.labels.len(), 2);
    let labels: HashMap<_, _> = c.labels.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("chassis"), Some(&"1"));
}

#[tokio::test]
async fn no_thermal_power_links_is_ok() {
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
    let metrics = collect_power_metrics(bmc, &root, "bmc1").await.unwrap();
    assert!(metrics.is_empty());
}
