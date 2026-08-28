use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::systems::{
    collect_assembly, collect_firmware, collect_managers, collect_systems,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

fn labels_of(metric: &redfish_exporter::metrics::Metric) -> HashMap<&'static str, &str> {
    metric
        .labels
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect()
}

fn expect_service_root(bmc: &Mock, services: &[&str]) -> ODataId {
    let root_id = ODataId::service_root();
    let mut payload = json!({
        "@odata.id": "/redfish/v1",
        "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
        "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
    });
    for service in services {
        match *service {
            "Systems" => {
                payload["Systems"] = json!({ "@odata.id": "/redfish/v1/Systems" });
            }
            "Managers" => {
                payload["Managers"] = json!({ "@odata.id": "/redfish/v1/Managers" });
            }
            "UpdateService" => {
                payload["UpdateService"] = json!({ "@odata.id": "/redfish/v1/UpdateService" });
            }
            "Chassis" => {
                payload["Chassis"] = json!({ "@odata.id": "/redfish/v1/Chassis" });
            }
            _ => unreachable!("unexpected service {service}"),
        }
    }
    bmc.expect(Expect::get(root_id.clone(), payload));
    root_id
}

fn expect_systems_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems",
        json!({
            "@odata.id": "/redfish/v1/Systems",
            "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
            "Name": "Systems",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_system_with_power(bmc: &Mock, power_state: &str) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1",
            "Id": "1", "Name": "System 1", "SystemType": "Physical",
            "PowerState": power_state,
            "Manufacturer": "Dell", "Model": "R750",
            "SerialNumber": "SN1", "SKU": "SKU1",
            "Status": { "Health": "OK", "State": "Enabled" },
        }),
    ));
}

#[tokio::test]
async fn collects_system_power_and_health() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["Systems"]);
    expect_systems_collection(&bmc);
    expect_system_with_power(&bmc, "On");

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_systems(bmc, &root, "bmc1").await.unwrap();

    let power_state: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_power_state")
        .collect();
    assert_eq!(power_state.len(), 1);
    assert_eq!(power_state[0].value, 1.0);
    let labels = labels_of(power_state[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].value, 1.0);
    let labels = labels_of(health[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("resource_type"), Some(&"system"));
    assert_eq!(labels.get("id"), Some(&"/redfish/v1/Systems/1"));
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));

    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_info")
        .collect();
    let info_values: HashMap<_, _> = info
        .iter()
        .map(|m| {
            let labels = labels_of(m);
            (
                labels.get("key").copied().unwrap(),
                labels.get("value").copied().unwrap(),
            )
        })
        .collect();
    assert_eq!(info_values.get("manufacturer"), Some(&"Dell"));
    assert_eq!(info_values.get("model"), Some(&"R750"));
    assert_eq!(info_values.get("serial_number"), Some(&"SN1"));
    assert_eq!(info_values.get("sku"), Some(&"SKU1"));
}

#[tokio::test]
async fn known_non_on_power_state_is_exported_as_zero() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["Systems"]);
    expect_systems_collection(&bmc);
    expect_system_with_power(&bmc, "Off");

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_systems(bmc, &root, "bmc1").await.unwrap();
    let power = metrics
        .iter()
        .find(|metric| metric.name == "redfish_power_state")
        .expect("known power state must emit a time series");
    assert_eq!(power.value, 0.0);
}

#[tokio::test]
async fn collects_firmware_inventory() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["UpdateService"]);
    bmc.expect(Expect::get(
        "/redfish/v1/UpdateService",
        json!({
            "@odata.id": "/redfish/v1/UpdateService",
            "Id": "UpdateService", "Name": "Update Service",
            "FirmwareInventory": { "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/UpdateService/FirmwareInventory",
        json!({
            "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory",
            "@odata.type": "#SoftwareInventoryCollection.SoftwareInventoryCollection",
            "Name": "Firmware Inventory",
            "Members": [{ "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/UpdateService/FirmwareInventory/1",
        json!({
            "@odata.id": "/redfish/v1/UpdateService/FirmwareInventory/1",
            "Id": "1", "Name": "BMC Firmware",
            "Version": "1.2.3",
            "Status": { "Health": "OK", "State": "Enabled" },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_firmware(bmc, &root, "bmc1").await.unwrap();

    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_info")
        .collect();
    let info_values: HashMap<_, _> = info
        .iter()
        .map(|m| {
            let labels = labels_of(m);
            (
                labels.get("key").copied().unwrap(),
                labels.get("value").copied().unwrap(),
            )
        })
        .collect();
    assert_eq!(info_values.get("firmware_version"), Some(&"1.2.3"));

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].value, 1.0);
    let labels = labels_of(health[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("resource_type"), Some(&"software_inventory"));
    assert_eq!(
        labels.get("id"),
        Some(&"/redfish/v1/UpdateService/FirmwareInventory/1")
    );
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));
}

#[tokio::test]
async fn missing_services_are_ok() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &[]);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let firmware = collect_firmware(Arc::clone(&bmc), &root, "bmc1")
        .await
        .unwrap();
    assert!(firmware.is_empty());
    let managers = collect_managers(bmc, &root, "bmc1").await.unwrap();
    assert!(managers.is_empty());
}

#[tokio::test]
async fn all_declared_assembly_resources_failing_is_an_error() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["Chassis"]);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "Assembly": { "@odata.id": "/redfish/v1/Chassis/1/Assembly" },
        }),
    ));
    // No Assembly response: the only declared resource fails.
    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let error = collect_assembly(bmc, &root, "bmc1")
        .await
        .expect_err("declared Assembly resource must not fail silently");
    assert!(
        error.contains("all 1/1 declared resources failed"),
        "{error}"
    );
}
