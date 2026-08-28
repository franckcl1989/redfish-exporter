use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::network::collect_network;
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
            "Systems": { "@odata.id": "/redfish/v1/Systems" },
        }),
    ));
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

fn expect_empty_systems_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems",
        json!({
            "@odata.id": "/redfish/v1/Systems",
            "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
            "Name": "Systems",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
}

fn expect_empty_chassis_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis",
        json!({
            "@odata.id": "/redfish/v1/Chassis",
            "@odata.type": "#ChassisCollection.ChassisCollection",
            "Name": "Chassis Collection",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
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

fn labels_of(metric: &redfish_exporter::metrics::Metric) -> HashMap<&'static str, &str> {
    metric
        .labels
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect()
}

#[tokio::test]
async fn collects_ethernet_link_status() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1",
            "Id": "1", "Name": "System 1", "SystemType": "Physical",
            "Status": { "Health": "OK", "State": "Enabled" },
            "EthernetInterfaces": { "@odata.id": "/redfish/v1/Systems/1/EthernetInterfaces" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/EthernetInterfaces",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/EthernetInterfaces",
            "@odata.type": "#EthernetInterfaceCollection.EthernetInterfaceCollection",
            "Name": "Ethernet Interfaces",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/EthernetInterfaces/eth0" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/EthernetInterfaces/eth0",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/EthernetInterfaces/eth0",
            "Id": "eth0", "Name": "Ethernet Interface 0",
            "LinkStatus": "LinkUp",
            "SpeedMbps": 1000,
            "MACAddress": "00:11:22:33:44:55",
            "Status": { "Health": "OK", "State": "Enabled" },
        }),
    ));
    expect_empty_chassis_collection(&bmc);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_network(bmc, &root, "bmc1").await.unwrap();

    let link_status: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_ethernet_interface_link_status")
        .collect();
    assert_eq!(link_status.len(), 1);
    assert_eq!(link_status[0].value, 1.0);
    let labels = labels_of(link_status[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"eth0"));

    let speed: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_ethernet_interface_speed_mbps")
        .collect();
    assert_eq!(speed.len(), 1);
    assert_eq!(speed[0].value, 1000.0);
    let labels = labels_of(speed[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"eth0"));

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 1);
    let labels = labels_of(health[0]);
    assert_eq!(labels.get("resource_type"), Some(&"ethernet_interface"));
    assert_eq!(
        labels.get("id"),
        Some(&"/redfish/v1/Systems/1/EthernetInterfaces/eth0")
    );
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
    assert_eq!(info_values.get("mac_address"), Some(&"00:11:22:33:44:55"));
}

#[tokio::test]
async fn collects_pcie_health_and_lanes() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_empty_systems_collection(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "PCIeDevices": { "@odata.id": "/redfish/v1/Chassis/1/PCIeDevices" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/PCIeDevices",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/PCIeDevices",
            "@odata.type": "#PCIeDeviceCollection.PCIeDeviceCollection",
            "Name": "PCIe Devices",
            "Members": [{ "@odata.id": "/redfish/v1/Chassis/1/PCIeDevices/GPU0" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/PCIeDevices/GPU0",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/PCIeDevices/GPU0",
            "Id": "GPU0", "Name": "GPU 0",
            "Status": { "Health": "OK", "State": "Enabled" },
            "PCIeInterface": { "LanesInUse": 8, "MaxLanes": 16 },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_network(bmc, &root, "bmc1").await.unwrap();

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 1);
    let labels = labels_of(health[0]);
    assert_eq!(labels.get("resource_type"), Some(&"pcie_device"));
    assert_eq!(
        labels.get("id"),
        Some(&"/redfish/v1/Chassis/1/PCIeDevices/GPU0")
    );
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));
    assert_eq!(health[0].value, 1.0);

    let lanes_in_use: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_pcie_device_lanes_in_use")
        .collect();
    assert_eq!(lanes_in_use.len(), 1);
    assert_eq!(lanes_in_use[0].value, 8.0);
    let labels = labels_of(lanes_in_use[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("chassis"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"GPU0"));

    let max_lanes: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_pcie_device_max_lanes")
        .collect();
    assert_eq!(max_lanes.len(), 1);
    assert_eq!(max_lanes[0].value, 16.0);
    let labels = labels_of(max_lanes[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("chassis"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"GPU0"));
}

#[tokio::test]
async fn falls_back_when_network_adapter_collection_members_lack_inline_id() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_empty_systems_collection(&bmc);
    expect_chassis_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1",
            "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
            "NetworkAdapters": {
                "@odata.id": "/redfish/v1/Chassis/1/NetworkAdapters"
            },
        }),
    ));
    let malformed_collection = json!({
        "@odata.id": "/redfish/v1/Chassis/1/NetworkAdapters",
        "Name": "Network Adapters",
        "Members": [{
            "@odata.id": "/redfish/v1/Chassis/1/NetworkAdapters/outboardPCIeCard0",
            "@odata.type": "#NetworkAdapter.v1_9_0.NetworkAdapter"
        }],
        "Members@odata.count": 1,
    });
    // Typed collection fetch fails because the annotation makes the member
    // look expanded while `Id` is absent; the raw fallback fetches it again.
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/NetworkAdapters",
        malformed_collection.clone(),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/NetworkAdapters",
        malformed_collection,
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/NetworkAdapters/outboardPCIeCard0",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/NetworkAdapters/outboardPCIeCard0",
            "Id": "OutboardPCIeCard0",
            "Name": "OutboardPCIeCard0",
            "Manufacturer": "Intel",
            "Model": "Ethernet Controller X710",
            "Status": { "Health": "OK", "State": "Enabled" },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_network(bmc, &root, "bmc1").await.unwrap();

    let adapter_health = metrics
        .iter()
        .find(|metric| {
            metric.name == "redfish_health_status"
                && labels_of(metric).get("resource_type") == Some(&"network_adapter")
        })
        .expect("fallback must retain adapter health");
    assert_eq!(labels_of(adapter_health).get("health"), Some(&"OK"));
    assert!(metrics.iter().any(|metric| {
        let labels = labels_of(metric);
        metric.name == "redfish_info"
            && labels.get("resource_type") == Some(&"network_adapter")
            && labels.get("key") == Some(&"model")
            && labels.get("value") == Some(&"Ethernet Controller X710")
    }));
}

#[tokio::test]
async fn all_declared_ethernet_collections_failing_is_an_error() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1",
            "Id": "1", "Name": "System 1", "SystemType": "Physical",
            "EthernetInterfaces": {
                "@odata.id": "/redfish/v1/Systems/1/EthernetInterfaces"
            },
        }),
    ));
    // No EthernetInterfaces response: the only declared collection fails.
    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let error = collect_network(bmc, &root, "bmc1")
        .await
        .expect_err("declared Ethernet collection must not fail silently");
    assert!(
        error.contains("all 1/1 declared collections failed"),
        "{error}"
    );
}
