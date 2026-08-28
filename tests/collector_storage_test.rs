use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::storage::collect_storage;
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

fn expect_system(bmc: &Mock, with_storage: bool) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1",
        "Id": "1", "Name": "System 1", "SystemType": "Physical",
        "Status": { "Health": "OK", "State": "Enabled" },
    });
    if with_storage {
        payload["Storage"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Storage" });
    }
    bmc.expect(Expect::get("/redfish/v1/Systems/1", payload));
}

fn expect_storage_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage",
            "@odata.type": "#StorageCollection.StorageCollection",
            "Name": "Storage Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_storage(bmc: &Mock, with_drives: bool, with_volumes: bool) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1",
        "Id": "SATA1", "Name": "SATA 1",
        "Status": { "Health": "OK", "State": "Enabled" },
        "StorageControllers": [{
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1#/StorageControllers/0",
            "MemberId": "0",
            "Model": "PERC H755 Adapter",
            "FirmwareVersion": "52.16.1-4405",
            "Status": { "Health": "OK", "State": "Enabled" },
        }],
    });
    if with_drives {
        payload["Drives"] =
            json!([{ "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1" }]);
    }
    if with_volumes {
        payload["Volumes"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Volumes" });
    }
    // 控制器明细经 raw JSON 重取同一 URI，故该 GET 期望注册两次（FIFO 顺序）。
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1",
        payload.clone(),
    ));
    bmc.expect(Expect::get("/redfish/v1/Systems/1/Storage/SATA1", payload));
}

fn expect_drive(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1",
            "Id": "HDD1", "Name": "HDD 1",
            "Status": { "Health": "OK", "State": "Enabled" },
            "Manufacturer": "Seagate", "Model": "ST1000",
            "SerialNumber": "SN123", "Revision": "A1",
            "CapacityBytes": 1024,
            "PredictedMediaLifeLeftPercent": 80,
            "FailurePredicted": false,
            "Metrics": { "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1/Metrics" },
        }),
    ));
}

fn expect_drive_metrics(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "CorrectableIOReadErrorCount": 3,
            "CorrectableIOWriteErrorCount": 0,
            "UncorrectableIOReadErrorCount": 0,
            "UncorrectableIOWriteErrorCount": 1,
        }),
    ));
}

fn expect_volume_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1/Volumes",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Volumes",
            "@odata.type": "#VolumeCollection.VolumeCollection",
            "Name": "Volume Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Volumes/1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_volume(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1/Volumes/1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Volumes/1",
            "Id": "1", "Name": "Volume 1",
            "Status": { "Health": "OK", "State": "Enabled" },
            "CapacityBytes": 2048,
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
async fn collects_drive_metrics() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_storage_collection(&bmc);
    expect_storage(&bmc, true, true);
    expect_drive(&bmc);
    expect_drive_metrics(&bmc);
    expect_volume_collection(&bmc);
    expect_volume(&bmc);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_storage(bmc, &root, "bmc1").await.unwrap();

    let capacity: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_capacity_bytes")
        .collect();
    assert_eq!(capacity.len(), 1);
    assert_eq!(capacity[0].value, 1024.0);
    let labels = labels_of(capacity[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("storage"), Some(&"SATA1"));
    assert_eq!(labels.get("id"), Some(&"HDD1"));

    let life_left: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_life_left_percent")
        .collect();
    assert_eq!(life_left.len(), 1);
    assert_eq!(life_left[0].value, 80.0);

    let failure: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_predictive_failure")
        .collect();
    assert_eq!(failure.len(), 1);
    assert_eq!(failure[0].value, 0.0);
    let labels = labels_of(failure[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("storage"), Some(&"SATA1"));
    assert_eq!(labels.get("id"), Some(&"HDD1"));

    let read_correctable: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_io_read_correctable_errors_total")
        .collect();
    assert_eq!(read_correctable.len(), 1);
    assert_eq!(read_correctable[0].value, 3.0);
    let write_correctable: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_io_write_correctable_errors_total")
        .collect();
    assert_eq!(write_correctable.len(), 1);
    assert_eq!(write_correctable[0].value, 0.0);
    let read_uncorrectable: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_io_read_uncorrectable_errors_total")
        .collect();
    assert_eq!(read_uncorrectable.len(), 1);
    assert_eq!(read_uncorrectable[0].value, 0.0);
    let write_uncorrectable: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_io_write_uncorrectable_errors_total")
        .collect();
    assert_eq!(write_uncorrectable.len(), 1);
    assert_eq!(write_uncorrectable[0].value, 1.0);

    let volume_capacity: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_volume_capacity_bytes")
        .collect();
    assert_eq!(volume_capacity.len(), 1);
    assert_eq!(volume_capacity[0].value, 2048.0);
    let labels = labels_of(volume_capacity[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("storage"), Some(&"SATA1"));
    assert_eq!(labels.get("id"), Some(&"1"));

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 2);
    for m in &health {
        assert_eq!(m.value, 1.0);
    }
    let drive_health: Vec<_> = health
        .iter()
        .filter(|m| {
            let labels = labels_of(m);
            labels.get("resource_type") == Some(&"drive")
                && labels.get("id") == Some(&"/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1")
        })
        .collect();
    assert_eq!(drive_health.len(), 1);
    let labels = labels_of(drive_health[0]);
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));
    let volume_health: Vec<_> = health
        .iter()
        .filter(|m| {
            let labels = labels_of(m);
            labels.get("resource_type") == Some(&"volume")
                && labels.get("id") == Some(&"/redfish/v1/Systems/1/Storage/SATA1/Volumes/1")
        })
        .collect();
    assert_eq!(volume_health.len(), 1);

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
    assert_eq!(info_values.get("manufacturer"), Some(&"Seagate"));
    assert_eq!(info_values.get("model"), Some(&"ST1000"));
    assert_eq!(info_values.get("serial_number"), Some(&"SN123"));
    assert_eq!(info_values.get("revision"), Some(&"A1"));
    assert_eq!(info_values.get("name"), Some(&"Volume 1"));

    // 无 Oem 块（浪潮式驱动器：明细全 null 仅 Status）不得产出 OEM 指标。
    assert!(
        !metrics.iter().any(|m| m.name == "redfish_drive_info"),
        "drive without Oem must not emit drive_info"
    );
    assert!(
        !metrics.iter().any(|m| m.name == "redfish_drive_oem_status"),
        "drive without Oem must not emit drive_oem_status"
    );
}

#[tokio::test]
async fn drive_oem_life_left_null_emits_oem_only() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_storage_collection(&bmc);
    expect_storage(&bmc, true, false);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1",
            "Id": "HDD1", "Name": "HDD 1",
            "Status": { "Health": "OK", "State": "Enabled" },
            // 真机探测：Dell/浪潮全部驱动器 PredictedMediaLifeLeftPercent
            // 均 null（NOT MET backlog）——寿命字段不采集。
            "PredictedMediaLifeLeftPercent": null,
            "Oem": { "Dell": { "DellPhysicalDisk": {
                "WWN": "3F4EE0803B522508",
                "RaidStatus": "Online",
                "PowerStatus": "On",
            } } },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_storage(bmc, &root, "bmc1").await.unwrap();

    assert!(
        !metrics
            .iter()
            .any(|m| m.name == "redfish_drive_life_left_percent"),
        "null PredictedMediaLifeLeftPercent must not emit life-left metric"
    );
    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_info")
        .collect();
    assert_eq!(info.len(), 1);
    let labels = labels_of(info[0]);
    assert_eq!(labels.get("wwn"), Some(&"3F4EE0803B522508"));
    let oem: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_oem_status")
        .collect();
    assert_eq!(oem.len(), 1);
    let labels = labels_of(oem[0]);
    assert_eq!(labels.get("raid_status"), Some(&"Online"));
    assert_eq!(labels.get("power_status"), Some(&"On"));
}

#[tokio::test]
async fn drive_oem_single_status_emits_with_empty_other() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_storage_collection(&bmc);
    expect_storage(&bmc, true, false);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1",
            "Id": "HDD1", "Name": "HDD 1",
            "Status": { "Health": "OK", "State": "Enabled" },
            // 任一非空门控（either-non-null）：仅 RaidStatus 无 PowerStatus
            // 仍产出 drive_oem_status，缺失标签为空字符串。
            "Oem": { "Dell": { "DellPhysicalDisk": {
                "RaidStatus": "Online",
            } } },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_storage(bmc, &root, "bmc1").await.unwrap();

    let oem: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_drive_oem_status")
        .collect();
    assert_eq!(oem.len(), 1);
    let labels = labels_of(oem[0]);
    assert_eq!(labels.get("raid_status"), Some(&"Online"));
    assert_eq!(labels.get("power_status"), Some(&""));
    assert!(
        !metrics.iter().any(|m| m.name == "redfish_drive_info"),
        "drive without WWN must not emit drive_info"
    );
}

#[tokio::test]
async fn collects_storage_controller_metrics() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_storage_collection(&bmc);
    expect_storage(&bmc, false, false);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_storage(bmc, &root, "bmc1").await.unwrap();

    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_storage_controller_info")
        .collect();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].value, 1.0);
    let labels = labels_of(info[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("storage"), Some(&"SATA1"));
    assert_eq!(labels.get("id"), Some(&"0"));
    assert_eq!(labels.get("model"), Some(&"PERC H755 Adapter"));
    assert_eq!(labels.get("firmware_version"), Some(&"52.16.1-4405"));

    let status: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_storage_controller_status")
        .collect();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].value, 1.0);
    let labels = labels_of(status[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("storage"), Some(&"SATA1"));
    assert_eq!(labels.get("id"), Some(&"0"));
    assert_eq!(labels.get("status"), Some(&"Enabled"));
}

#[tokio::test]
async fn no_storage_is_ok() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, false);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_storage(bmc, &root, "bmc1").await.unwrap();
    assert!(metrics.is_empty());
}

#[tokio::test]
async fn drives_failure_does_not_block_volumes() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_storage_collection(&bmc);
    expect_storage(&bmc, true, true);
    // drive GET 命中但响应为错误 → drives() 返回 Err；
    // volumes 采集不受影响。
    bmc.expect(nv_redfish_bmc_mock::Expect {
        request: nv_redfish_bmc_mock::ExpectedRequest::Get {
            id: "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1"
                .to_string()
                .into(),
        },
        response: Err(serde_json::from_str::<serde_json::Value>("").unwrap_err()),
    });
    expect_volume_collection(&bmc);
    expect_volume(&bmc);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_storage(bmc, &root, "bmc1").await.unwrap();

    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_volume_capacity_bytes" && m.value == 2048.0),
        "volume metrics must still be produced"
    );
    assert!(
        metrics.iter().any(|m| m.name == "redfish_health_status"
            && m.value == 1.0
            && labels_of(m).get("resource_type") == Some(&"volume")),
        "volume health must still be produced"
    );
    assert!(
        !metrics
            .iter()
            .any(|m| m.name == "redfish_drive_capacity_bytes"),
        "drive metrics must be skipped when drives() fails"
    );
}

#[tokio::test]
async fn all_declared_storage_collections_failing_is_an_error() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    // No Storage collection response: the only declared subtree fails.
    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let error = collect_storage(bmc, &root, "bmc1")
        .await
        .expect_err("declared storage collection must not fail silently");
    assert!(
        error.contains("all 1/1 declared collections failed"),
        "{error}"
    );
}

#[tokio::test]
async fn all_declared_storage_controller_subresources_failing_is_an_error() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true);
    expect_storage_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1",
            "Id": "SATA1", "Name": "SATA 1",
            "Drives": [{
                "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1"
            }],
            "Volumes": {
                "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Volumes"
            },
        }),
    ));
    // Neither the drive nor volume collection has a response.
    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let error = collect_storage(bmc, &root, "bmc1")
        .await
        .expect_err("all declared storage subresources must not fail silently");
    assert!(error.contains("all 1/1 controllers failed"), "{error}");
}
