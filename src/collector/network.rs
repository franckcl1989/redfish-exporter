use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{LINK_SPEED, LINK_STATUS, Metric};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::chassis::NetworkAdapter;
use nv_redfish::ethernet_interface::{EthernetInterface, LinkStatus};
use nv_redfish::pcie_device::PcieDevice;
use std::sync::Arc;

const PCIE_DEVICE_LANES_IN_USE: (&str, &str) = (
    "redfish_pcie_device_lanes_in_use",
    "Number of PCIe lanes in use by the device",
);
const PCIE_DEVICE_MAX_LANES: (&str, &str) = (
    "redfish_pcie_device_max_lanes",
    "Maximum number of PCIe lanes supported by the device",
);

pub async fn collect_network<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    collect_ethernet(root, bmc_name, &mut out).await?;
    collect_pcie(root, bmc_name, &mut out).await?;
    Ok(out)
}

async fn collect_ethernet<B: Bmc>(
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(systems) = root.systems().await.map_err(|e| format!("systems: {e}"))? else {
        return Ok(());
    };
    let systems = systems
        .members()
        .await
        .map_err(|e| format!("systems members: {e}"))?;
    for system in systems {
        let system_id = system.id().to_string();
        let Ok(Some(interfaces)) = system.ethernet_interfaces().await else {
            continue;
        };
        let Ok(interfaces) = interfaces.members().await else {
            continue;
        };
        for interface in interfaces {
            collect_ethernet_interface(bmc_name, &system_id, &interface, out);
        }
    }
    Ok(())
}

fn collect_ethernet_interface<B: Bmc>(
    bmc_name: &str,
    system_id: &str,
    interface: &EthernetInterface<B>,
    out: &mut Vec<Metric>,
) {
    let raw = interface.raw();
    let id = interface.id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "ethernet_interface", &id, &health, &state);
    if let Some(mac) = interface.mac_address() {
        push_info(out, bmc_name, "mac_address", &mac.to_string());
    }
    match interface.link_status() {
        Some(LinkStatus::LinkUp) => {
            push_link_value(out, bmc_name, system_id, &id, LINK_STATUS, Some(1.0));
        }
        Some(LinkStatus::NoLink)
        | Some(LinkStatus::LinkDown)
        | Some(LinkStatus::UnsupportedValue) => {
            push_link_value(out, bmc_name, system_id, &id, LINK_STATUS, Some(0.0));
        }
        None => {}
    }
    push_link_value(
        out,
        bmc_name,
        system_id,
        &id,
        LINK_SPEED,
        raw.speed_mbps.flatten().map(|v| v as f64),
    );
}

async fn collect_pcie<B: Bmc>(
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(chassis_collection) = root.chassis().await.map_err(|e| format!("chassis: {e}"))?
    else {
        return Ok(());
    };
    let chassis_members = chassis_collection
        .members()
        .await
        .map_err(|e| format!("chassis members: {e}"))?;
    for chassis in chassis_members {
        if let Ok(Some(adapters)) = chassis.network_adapters().await {
            for adapter in adapters {
                collect_network_adapter(bmc_name, &adapter, out).await;
            }
        }
        let Ok(Some(devices)) = chassis.pcie_devices().await else {
            continue;
        };
        let Ok(devices) = devices.members().await else {
            continue;
        };
        for device in devices {
            collect_pcie_device(bmc_name, &device, out);
        }
    }
    Ok(())
}

async fn collect_network_adapter<B: Bmc>(
    bmc_name: &str,
    adapter: &NetworkAdapter<B>,
    out: &mut Vec<Metric>,
) {
    let raw = adapter.raw();
    let id = adapter.id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "network_adapter", &id, &health, &state);
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(out, bmc_name, "manufacturer", &value);
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(out, bmc_name, "model", &value);
    }
    if let Some(value) = raw
        .controllers
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.firmware_package_version.clone().flatten())
    {
        push_info(out, bmc_name, "firmware_version", &value);
    }
    let Ok(Some(ports)) = adapter.ports().await else {
        return;
    };
    let Ok(ports) = ports.members().await else {
        return;
    };
    for port in ports {
        let raw = port.raw();
        let port_id = port.id().to_string();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(out, bmc_name, "port", &port_id, &health, &state);
    }
}

fn collect_pcie_device<B: Bmc>(bmc_name: &str, device: &PcieDevice<B>, out: &mut Vec<Metric>) {
    let raw = device.raw();
    let id = device.id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "pcie_device", &id, &health, &state);
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(out, bmc_name, "manufacturer", &value);
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(out, bmc_name, "model", &value);
    }
    if let Some(value) = raw.serial_number.clone().flatten() {
        push_info(out, bmc_name, "serial_number", &value);
    }
    if let Some(value) = raw.firmware_version.clone().flatten() {
        push_info(out, bmc_name, "firmware_version", &value);
    }
    let Some(pcie_interface) = &raw.pcie_interface else {
        return;
    };
    push_device_value(
        out,
        bmc_name,
        &id,
        PCIE_DEVICE_LANES_IN_USE,
        pcie_interface.lanes_in_use.flatten().map(|v| v as f64),
    );
    push_device_value(
        out,
        bmc_name,
        &id,
        PCIE_DEVICE_MAX_LANES,
        pcie_interface.max_lanes.flatten().map(|v| v as f64),
    );
}

fn push_link_value(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    system_id: &str,
    id: &str,
    name_help: (&'static str, &'static str),
    value: Option<f64>,
) {
    let Some(value) = value else {
        return;
    };
    out.push(
        Metric::gauge(name_help.0, name_help.1)
            .label("bmc", bmc_name.to_string())
            .label("system", system_id.to_string())
            .label("id", id.to_string())
            .build(value),
    );
}

fn push_device_value(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    id: &str,
    name_help: (&'static str, &'static str),
    value: Option<f64>,
) {
    let Some(value) = value else {
        return;
    };
    out.push(
        Metric::gauge(name_help.0, name_help.1)
            .label("bmc", bmc_name.to_string())
            .label("id", id.to_string())
            .build(value),
    );
}
