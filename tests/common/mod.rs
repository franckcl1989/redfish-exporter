//! MockBmc 共享测试助手：integration_test 与 soak_test 复用同一套
//! 期望定义（expect_*）与 Mock 类型别名，保证两处行为一致。
//!
//! 自 integration_test.rs 原样搬移（签名不变），仅加 pub 以便
//! `use common::*;` 通配导入。

use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use serde_json::json;
use std::collections::HashMap;

pub type Mock = MockBmc<serde_json::Error>;

// labels_of 仅 integration_test 调用；soak_test 经 `use common::*;` 引入但不使用，
// 各测试 crate 独立编译，故需放宽 dead_code。
#[allow(dead_code)]
pub fn labels_of(metric: &redfish_exporter::metrics::Metric) -> HashMap<&'static str, &str> {
    metric
        .labels
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect()
}

pub fn expect_service_root(bmc: &Mock, links: &[&str]) {
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

pub fn expect_chassis_collection(bmc: &Mock) {
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
pub fn expect_chassis_round(bmc: &Mock, with_links: bool) {
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

pub fn expect_sensor_payloads(bmc: &Mock) {
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

pub fn expect_thermal_power_payloads(bmc: &Mock) {
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

pub fn expect_systems_collection(bmc: &Mock) {
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

pub fn expect_system(bmc: &Mock, links: &[&str]) {
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

pub fn expect_processor_payloads(bmc: &Mock) {
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

pub fn expect_memory_payloads(bmc: &Mock) {
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

pub fn expect_storage_payloads(bmc: &Mock) {
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

pub fn expect_bios_payloads(bmc: &Mock) {
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

pub fn expect_ethernet_payloads(bmc: &Mock) {
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

pub fn expect_pcie_payloads(bmc: &Mock) {
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

pub fn expect_assembly_payloads(bmc: &Mock) {
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

pub fn expect_firmware_payloads(bmc: &Mock) {
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
