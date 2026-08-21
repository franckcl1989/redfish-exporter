//! 集成测试：完整采集周期、BMC 失败隔离、session 认证流程。
//!
//! 全部通过 nv-redfish-bmc-mock 模拟真实 BMC 的 HTTP 行为，验证
//! collect_fast/collect_slow 的分组采集链路与 establish_session 的会话流程。

use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish::schema::session::SessionCreate;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::bmc::establish_session;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::metrics::encode;
use redfish_exporter::registry::build_registry;
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

fn expect_service_root(bmc: &Mock, links: &[&str]) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1",
        "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
        "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
    });
    for link in links {
        match *link {
            "Chassis" => {
                payload["Chassis"] = json!({ "@odata.id": "/redfish/v1/Chassis" });
            }
            "Systems" => {
                payload["Systems"] = json!({ "@odata.id": "/redfish/v1/Systems" });
            }
            "UpdateService" => {
                payload["UpdateService"] = json!({ "@odata.id": "/redfish/v1/UpdateService" });
            }
            _ => unreachable!("unexpected root link {link}"),
        }
    }
    bmc.expect(Expect::get(ODataId::service_root(), payload));
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

/// 一个 chassis 枚举轮次（集合 GET + chassis GET）。
/// 每个 collector 独立枚举 chassis（设计文档记录），
/// sensors/power/network/chassis_health/assembly 共 5 个 collector。
fn expect_chassis_round(bmc: &Mock, with_links: bool) {
    expect_chassis_collection(bmc);
    let mut chassis = json!({
        "@odata.id": "/redfish/v1/Chassis/1",
        "Id": "1", "Name": "Chassis 1", "ChassisType": "RackMount",
        "Status": { "Health": "OK", "State": "Enabled" },
        "Manufacturer": "Acme", "Model": "Server", "SerialNumber": "SN-C1", "PartNumber": "PN-C1",
    });
    if with_links {
        chassis["Sensors"] = json!({ "@odata.id": "/redfish/v1/Chassis/1/Sensors" });
        chassis["Thermal"] = json!({ "@odata.id": "/redfish/v1/Chassis/1/Thermal" });
        chassis["Power"] = json!({ "@odata.id": "/redfish/v1/Chassis/1/Power" });
        chassis["PCIeDevices"] = json!({ "@odata.id": "/redfish/v1/Chassis/1/PCIeDevices" });
        chassis["Assembly"] = json!({ "@odata.id": "/redfish/v1/Chassis/1/Assembly" });
    }
    bmc.expect(Expect::get("/redfish/v1/Chassis/1", chassis));
}

fn expect_sensor_payloads(bmc: &Mock) {
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

fn expect_thermal_power_payloads(bmc: &Mock) {
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

fn expect_system(bmc: &Mock, links: &[&str]) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1",
        "Id": "1", "Name": "System 1", "SystemType": "Physical",
        "PowerState": "On",
        "Manufacturer": "Dell", "Model": "R750",
        "SerialNumber": "SN1", "SKU": "SKU1",
        "Status": { "Health": "OK", "State": "Enabled" },
    });
    for link in links {
        match *link {
            "Processors" => {
                payload["Processors"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Processors" });
            }
            "Memory" => {
                payload["Memory"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Memory" });
            }
            "Storage" => {
                payload["Storage"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Storage" });
            }
            "EthernetInterfaces" => {
                payload["EthernetInterfaces"] =
                    json!({ "@odata.id": "/redfish/v1/Systems/1/EthernetInterfaces" });
            }
            "Bios" => {
                payload["Bios"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Bios" });
            }
            _ => unreachable!("unexpected system link {link}"),
        }
    }
    bmc.expect(Expect::get("/redfish/v1/Systems/1", payload));
}

fn expect_processor_payloads(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Processors",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Processors",
            "@odata.type": "#ProcessorCollection.ProcessorCollection",
            "Name": "Processor Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Processors/CPU1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1",
            "Id": "CPU1", "Name": "CPU 1", "ProcessorType": "CPU",
            "Status": { "Health": "OK", "State": "Enabled" },
            "Manufacturer": "Intel", "Model": "Xeon Gold 6338",
            "Metrics": { "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1/Metrics" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Processors/CPU1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "TemperatureCelsius": 55.0,
            "ConsumedPowerWatt": 60.0,
            "BandwidthPercent": 45.0,
        }),
    ));
}

fn expect_memory_payloads(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory",
            "@odata.type": "#MemoryCollection.MemoryCollection",
            "Name": "Memory Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1" }],
            "Members@odata.count": 1,
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory/DIMM1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1",
            "Id": "DIMM1", "Name": "DIMM 1", "MemoryType": "DRAM",
            "Status": { "Health": "OK", "State": "Enabled" },
            "Manufacturer": "Samsung", "PartNumber": "M393A2K43DB3-CWE",
            "CapacityMiB": 16384,
            "Metrics": { "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1/Metrics" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "BandwidthPercent": 30.0,
        }),
    ));
}

fn expect_storage_payloads(bmc: &Mock) {
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
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Storage/SATA1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1",
            "Id": "SATA1", "Name": "SATA 1",
            "Status": { "Health": "OK", "State": "Enabled" },
            "Drives": [{ "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Drives/HDD1" }],
            "Volumes": { "@odata.id": "/redfish/v1/Systems/1/Storage/SATA1/Volumes" },
        }),
    ));
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

fn expect_bios_payloads(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Bios",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Bios",
            "Id": "Bios", "Name": "BIOS",
            "Attributes": {
                "BootMode": "Uefi",
                "MemTest": "Disabled",
                "LegacyBoot": true,
                "MaxCores": 16,
            },
            "@Redfish.Settings": {
                "@odata.id": "/redfish/v1/Systems/1/Bios/Settings",
                "SettingsObject": { "@odata.id": "/redfish/v1/Systems/1/Bios/Settings" },
            },
        }),
    ));
}

fn expect_ethernet_payloads(bmc: &Mock) {
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
}

fn expect_pcie_payloads(bmc: &Mock) {
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
}

fn expect_assembly_payloads(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Chassis/1/Assembly",
        json!({
            "@odata.id": "/redfish/v1/Chassis/1/Assembly",
            "Id": "Assembly", "Name": "Assembly",
            "Assemblies": [{
                "@odata.id": "/redfish/v1/Chassis/1/Assembly#/Assemblies/0",
                "MemberId": "0", "Name": "PSU Assembly",
                "Producer": "Acme", "Model": "PSU-800", "PartNumber": "PN-A1",
                "SerialNumber": "SN-A1",
                "Status": { "Health": "OK", "State": "Enabled" },
            }],
        }),
    ));
}

fn expect_firmware_payloads(bmc: &Mock) {
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
}

/// 测试 1：一个 MockBmc 覆盖快组（7 个 collector）+ 慢组（4 个 collector）
/// 的全部调用路径。
///
/// mock expectation 顺序必须与实际请求顺序一致：快组
/// sensors → power → processors → memory → systems → chassis_health → managers，
/// 慢组 storage → network → firmware → assembly → bios。
/// managers 因 root 无 Managers 链接而跳过（Ok(None)，无网络请求）。
#[tokio::test]
async fn full_scrape_cycle_with_mock_bmc() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["Chassis", "Systems", "UpdateService"]);

    // 快组：sensors（chassis 枚举）
    expect_chassis_round(&bmc, true);
    expect_sensor_payloads(&bmc);

    // 快组：power（chassis 枚举）
    expect_chassis_round(&bmc, true);
    expect_thermal_power_payloads(&bmc);

    // 快组：processors（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Processors"]);
    expect_processor_payloads(&bmc);

    // 快组：memory（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Memory"]);
    expect_memory_payloads(&bmc);

    // 快组：systems（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &[]);

    // 快组：chassis_health（chassis 枚举）
    expect_chassis_round(&bmc, true);

    // 快组：managers：root 无 Managers 链接 → 无网络请求

    // 慢组：storage（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Storage"]);
    expect_storage_payloads(&bmc);

    // 慢组：network：ethernet（systems 枚举）+ pcie（chassis 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["EthernetInterfaces"]);
    expect_ethernet_payloads(&bmc);
    expect_chassis_round(&bmc, true);
    expect_pcie_payloads(&bmc);

    // 慢组：firmware（UpdateService）
    expect_firmware_payloads(&bmc);

    // 慢组：assembly（chassis 枚举）
    expect_chassis_round(&bmc, true);
    expect_assembly_payloads(&bmc);

    // 慢组：bios（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Bios"]);
    expect_bios_payloads(&bmc);

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
    assert!(
        report.failed_resources.is_empty(),
        "failed_resources: {:?}",
        report.failed_resources
    );
    let metrics = &report.metrics;
    assert!(!metrics.is_empty());

    assert!(metrics.iter().any(|m| m.name == "redfish_up"
        && m.value == 1.0
        && labels_of(m).get("bmc") == Some(&"bmc1")));
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_scrape_duration_seconds")
    );

    let sensor_readings: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_sensor_reading")
        .collect();
    assert!(
        sensor_readings.len() >= 2,
        "expected ambient + thermal sensor readings, got {}",
        sensor_readings.len()
    );

    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_processor_temperature_celsius" && m.value == 55.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_processor_power_watts" && m.value == 60.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_power_consumption_watts" && m.value == 320.0)
    );
    assert!(metrics.iter().any(|m| {
        m.name == "redfish_health_status"
            && m.value == 1.0
            && labels_of(m).get("resource_type") == Some(&"system")
    }));
    assert!(metrics.iter().any(|m| m.name == "redfish_info"));
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_power_state" && m.value == 1.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_ethernet_interface_link_status")
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_bios_pending_changes" && m.value == 1.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_bios_attribute" && m.value == 16.0)
    );

    // 端到端：ScrapeReport → prometheus Registry → 文本编码
    let registry = build_registry("bmc1", &report, 0).await.unwrap();
    let out = encode(&registry);
    assert!(out.contains("redfish_up{bmc=\"bmc1\"} 1"));
    assert!(out.contains("redfish_power_consumption_watts"));
    assert!(out.contains("redfish_health_status"));
}

/// 测试 2：BMC 失败隔离。
///
/// - 健康 BMC：完整流程成功（redfish_up=1）。
/// - 期望耗尽：root 成功但后续 GET 无期望 → 各 collector 失败进入
///   failed_resources，快/慢组合并后 up=0（隔离语义）。
/// - 根级失败：root GET 期望不匹配（UnexpectedGet）→ ServiceRoot::new 返回 Err
///   （仅 ServiceRoot 层错误会传播，collector 层错误被隔离）。
#[tokio::test]
async fn bmc_failure_isolation() {
    // 健康 BMC：root 仅含 Chassis 链接，快组 sensors/power/chassis_health 与
    // 慢组 network/assembly 共 5 个 chassis 枚举 collector 各自完成一轮
    // 集合+成员 GET，其余 collector 返回 Ok(空)。
    let ok = Arc::new(Mock::default());
    expect_service_root(&ok, &["Chassis"]);
    for _ in 0..5 {
        expect_chassis_round(&ok, false);
    }
    let root = ServiceRoot::new(Arc::clone(&ok)).await.unwrap();
    let fast = collect_fast(Arc::clone(&ok), &root, "ok-bmc")
        .await
        .unwrap();
    let slow = collect_slow(Arc::clone(&ok), &root, "ok-bmc")
        .await
        .unwrap();
    let merged = merge_reports(fast, Some(&slow));
    let report = finalize_report(
        "ok-bmc",
        merged.metrics,
        merged.failed_resources,
        std::time::Instant::now(),
    );
    assert!(report.failed_resources.is_empty());
    assert!(report.metrics.iter().any(|m| m.name == "redfish_up"
        && m.value == 1.0
        && labels_of(m).get("bmc") == Some(&"ok-bmc")));

    // 期望耗尽：root GET 之后队列为空，每个 collector 枚举失败，
    // 隔离进 failed_resources，redfish_up=0（scraper 层据此降级）。
    let exhausted = Arc::new(Mock::default());
    expect_service_root(&exhausted, &["Chassis"]);
    let root = ServiceRoot::new(Arc::clone(&exhausted)).await.unwrap();
    let fast = collect_fast(Arc::clone(&exhausted), &root, "exhausted-bmc")
        .await
        .unwrap();
    let slow = collect_slow(Arc::clone(&exhausted), &root, "exhausted-bmc")
        .await
        .unwrap();
    let merged = merge_reports(fast, Some(&slow));
    let report = finalize_report(
        "exhausted-bmc",
        merged.metrics,
        merged.failed_resources,
        std::time::Instant::now(),
    );
    assert!(!report.failed_resources.is_empty());
    assert!(report.metrics.iter().any(|m| m.name == "redfish_up"
        && m.value == 0.0
        && labels_of(m).get("bmc") == Some(&"exhausted-bmc")));

    // 根级失败：root GET 命中不匹配的期望（UnexpectedGet），
    // ServiceRoot::new 失败 → 错误传播（collect_round 中返回 Err）。
    let bad = Arc::new(Mock::default());
    bad.expect(Expect::get("/redfish/v1/not-the-real-root", json!({})));
    assert!(ServiceRoot::new(bad).await.is_err());
}

/// 测试 3：session 认证流程（泛型化 establish_session + MockBmc 全流程）。
///
/// - 单元断言：SessionCreate 构造字段正确；BmcCredentials::token 的
///   Debug 输出不泄漏 token。
/// - 全流程：root GET → SessionService GET → Sessions 集合 GET →
///   create_session（POST 负载 UserName/Password）→ 返回会话 token
///   （由调用方应用，scraper.rs 的 set_credentials 是 HttpBmc 特有能力）。
#[tokio::test]
async fn session_auth_flow() {
    let create = SessionCreate::builder("admin".into(), "secret".into()).build();
    assert_eq!(create.user_name, "admin");
    assert_eq!(create.password, "secret");
    assert!(create.token.is_none());
    assert!(create.links.is_none());

    let creds = nv_redfish::bmc_http::BmcCredentials::token("super-secret-token".into());
    let debug = format!("{creds:?}");
    assert!(!debug.contains("super-secret-token"));
    assert!(debug.contains("[REDACTED]"));

    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get(
        ODataId::service_root(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "SessionService": { "@odata.id": "/redfish/v1/SessionService" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService",
        json!({
            "@odata.id": "/redfish/v1/SessionService",
            "Id": "SessionService", "Name": "Session Service",
            "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService/Sessions",
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions",
            "@odata.type": "#SessionCollection.SessionCollection",
            "Name": "Sessions",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
    bmc.expect(Expect::create_session(
        "/redfish/v1/SessionService/Sessions",
        json!({ "UserName": "admin", "Password": "secret" }),
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions/1",
            "Id": "1", "Name": "User Session",
            "UserName": "admin",
            "SessionType": "Redfish",
        }),
        "session-token-123",
        "/redfish/v1/SessionService/Sessions/1",
    ));

    let token = establish_session(&bmc, "admin", "secret").await.unwrap();
    assert_eq!(token, "session-token-123");
}
