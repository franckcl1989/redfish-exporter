use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::bios::collect_bios;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

fn expect_service_root(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1",
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "Systems": { "@odata.id": "/redfish/v1/Systems" },
        }),
    ));
}

fn expect_systems_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems",
        json!({
            "@odata.id": "/redfish/v1/Systems",
            "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
            "Name": "Systems",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1" }],
        }),
    ));
}

fn expect_system(bmc: &Mock, with_bios: bool) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1",
        "Id": "1", "Name": "System 1",
    });
    if with_bios {
        payload["Bios"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Bios" });
    }
    bmc.expect(Expect::get("/redfish/v1/Systems/1", payload));
}

fn expect_bios(bmc: &Mock, with_settings: bool, attributes: serde_json::Value) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1/Bios",
        "Id": "Bios", "Name": "BIOS",
        "Attributes": attributes,
    });
    if with_settings {
        payload["@Redfish.Settings"] = json!({
            "@odata.id": "/redfish/v1/Systems/1/Bios/Settings",
            "SettingsObject": { "@odata.id": "/redfish/v1/Systems/1/Bios/Settings" },
        });
    }
    bmc.expect(Expect::get("/redfish/v1/Systems/1/Bios", payload));
}

fn labels_of(metric: &redfish_exporter::metrics::Metric) -> HashMap<&'static str, &str> {
    metric
        .labels
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect()
}

#[tokio::test]
async fn collects_bios_attributes() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_bios(
        &bmc,
        true,
        json!({
            "BootMode": "Uefi",
            "MemTest": "Disabled",
            "LegacyBoot": true,
            "MaxCores": 16,
            "SerialNumber": "SN123",
            "AdminPwd": null,
        }),
    );

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_bios(bmc, &root, "bmc1").await.unwrap();

    let pending = metrics
        .iter()
        .find(|m| m.name == "redfish_bios_pending_changes")
        .unwrap();
    assert_eq!(pending.value, 1.0);
    let labels = labels_of(pending);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));

    let attrs: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_bios_attribute")
        .collect();
    assert_eq!(attrs.len(), 3);
    let by_attr: HashMap<_, _> = attrs
        .iter()
        .map(|m| (labels_of(m).get("attribute").copied().unwrap(), m.value))
        .collect();
    assert_eq!(by_attr.get("MemTest"), Some(&0.0));
    assert_eq!(by_attr.get("LegacyBoot"), Some(&1.0));
    assert_eq!(by_attr.get("MaxCores"), Some(&16.0));
    for m in &attrs {
        let labels = labels_of(m);
        assert_eq!(labels.get("bmc"), Some(&"bmc1"));
        assert_eq!(labels.get("system"), Some(&"1"));
    }

    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_bios_attribute_info")
        .collect();
    assert_eq!(info.len(), 2);
    let info_by_attr: HashMap<_, _> = info
        .iter()
        .map(|m| {
            let labels = labels_of(m);
            (
                labels.get("attribute").copied().unwrap(),
                labels.get("value").copied().unwrap(),
            )
        })
        .collect();
    assert_eq!(info_by_attr.get("BootMode"), Some(&"Uefi"));
    assert_eq!(info_by_attr.get("SerialNumber"), Some(&"SN123"));
    for m in &info {
        assert_eq!(m.value, 1.0);
    }

    assert!(
        !metrics.iter().any(|m| {
            m.name == "redfish_bios_attribute" || m.name == "redfish_bios_attribute_info"
        } && labels_of(m).get("attribute") == Some(&"AdminPwd")),
        "null attribute must be skipped"
    );
}

#[tokio::test]
async fn no_settings_no_pending_changes() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_bios(&bmc, false, json!({ "BootMode": "Uefi" }));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_bios(bmc, &root, "bmc1").await.unwrap();

    let pending = metrics
        .iter()
        .find(|m| m.name == "redfish_bios_pending_changes")
        .unwrap();
    assert_eq!(pending.value, 0.0);
}

#[tokio::test]
async fn no_bios_link_is_ok() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, false);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_bios(bmc, &root, "bmc1").await.unwrap();
    assert!(metrics.is_empty());
}

#[tokio::test]
async fn list_attribute_value_fails_the_only_declared_bios_resource() {
    // dynamic_properties 仅支持 EdmPrimitiveType，属性值为数组时整个 BIOS 资源解析失败，
    // system.bios() 返回 Err；唯一已声明 BIOS 资源失败必须保持可观测，
    // 不能静默返回空成功。
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_bios(&bmc, false, json!({ "ListAttr": ["a", "b"] }));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let error = collect_bios(bmc, &root, "bmc1")
        .await
        .expect_err("the only declared BIOS resource failed to parse");
    assert!(
        error.contains("all 1/1 declared resources failed"),
        "{error}"
    );
}
