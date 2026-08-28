use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{LINK_SPEED, LINK_STATUS, Metric};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::chassis::NetworkAdapter;
use nv_redfish::core::{EntityTypeRef, ODataETag, ODataId};
use nv_redfish::ethernet_interface::{EthernetInterface, LinkStatus};
use nv_redfish::pcie_device::PcieDevice;
use nv_redfish::schema::network_adapter::NetworkAdapter as NetworkAdapterSchema;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
struct RawNetworkAdapterCollection {
    #[serde(rename = "@odata.id")]
    id: ODataId,
    #[serde(rename = "Members", default)]
    members: Vec<RawNetworkAdapterReference>,
}

impl EntityTypeRef for RawNetworkAdapterCollection {
    fn odata_id(&self) -> &ODataId {
        &self.id
    }

    fn etag(&self) -> Option<&ODataETag> {
        None
    }
}

#[derive(Deserialize)]
struct RawNetworkAdapterReference {
    #[serde(rename = "@odata.id")]
    id: ODataId,
}

const PCIE_DEVICE_LANES_IN_USE: (&str, &str) = (
    "redfish_pcie_device_lanes_in_use",
    "Number of PCIe lanes in use by the device",
);
const PCIE_DEVICE_MAX_LANES: (&str, &str) = (
    "redfish_pcie_device_max_lanes",
    "Maximum number of PCIe lanes supported by the device",
);

pub async fn collect_network<B: Bmc>(
    bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    collect_ethernet(root, bmc_name, &mut out).await?;
    collect_pcie(bmc.as_ref(), root, bmc_name, &mut out).await?;
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
    let mut attempted = 0usize;
    let mut failed = 0usize;
    for system in systems {
        let system_id = system.id().to_string();
        let interfaces = match system.ethernet_interfaces().await {
            Ok(Some(interfaces)) => {
                attempted += 1;
                interfaces
            }
            Ok(None) => continue,
            Err(error) => {
                attempted += 1;
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    error = %error,
                    "Ethernet interface collection fetch failed"
                );
                continue;
            }
        };
        let interfaces = match interfaces.members().await {
            Ok(interfaces) => interfaces,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    error = %error,
                    "Ethernet interface members fetch failed"
                );
                continue;
            }
        };
        for interface in interfaces {
            collect_ethernet_interface(bmc_name, &system_id, &interface, out);
        }
    }
    if attempted > 0 && failed == attempted {
        return Err(format!(
            "ethernet interfaces: all {failed}/{attempted} declared collections failed"
        ));
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
    let resource_id = interface.odata_id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(
        out,
        bmc_name,
        "ethernet_interface",
        &resource_id,
        &health,
        &state,
    );
    if let Some(mac) = interface.mac_address() {
        push_info(
            out,
            bmc_name,
            "ethernet_interface",
            &resource_id,
            "mac_address",
            &mac.to_string(),
        );
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
    bmc: &B,
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
    let mut attempted = 0usize;
    let mut failed = 0usize;
    for chassis in chassis_members {
        let chassis_id = chassis.id().to_string();
        match chassis.network_adapters().await {
            Ok(Some(adapters)) => {
                attempted += 1;
                for adapter in adapters {
                    collect_network_adapter(bmc_name, &adapter, out).await;
                }
            }
            Ok(None) => {}
            Err(error) => {
                attempted += 1;
                match collect_network_adapters_fallback(bmc, &chassis, bmc_name, out).await {
                    Ok(()) => tracing::debug!(
                        bmc = %bmc_name,
                        chassis = %chassis_id,
                        typed_error = %error,
                        "used raw network adapter collection fallback"
                    ),
                    Err(fallback_error) => {
                        failed += 1;
                        tracing::warn!(
                            bmc = %bmc_name,
                            chassis = %chassis_id,
                            error = %error,
                            fallback_error = %fallback_error,
                            "network adapter collection fetch failed"
                        );
                    }
                }
            }
        }
        let devices = match chassis.pcie_devices().await {
            Ok(Some(devices)) => {
                attempted += 1;
                devices
            }
            Ok(None) => continue,
            Err(error) => {
                attempted += 1;
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "PCIe device collection fetch failed"
                );
                continue;
            }
        };
        let devices = match devices.members().await {
            Ok(devices) => devices,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "PCIe device members fetch failed"
                );
                continue;
            }
        };
        for device in devices {
            collect_pcie_device(bmc_name, &chassis_id, &device, out);
        }
    }
    if attempted > 0 && failed == attempted {
        return Err(format!(
            "PCIe/network adapters: all {failed}/{attempted} declared collections failed"
        ));
    }
    Ok(())
}

/// Some IEIT/Inspur firmware returns collection members containing
/// `@odata.id` plus annotation fields but no inline `Id`. nv-redfish treats
/// those as expanded resources and rejects the collection. Re-read the
/// collection as references, then fetch each member as a complete resource.
async fn collect_network_adapters_fallback<B: Bmc>(
    bmc: &B,
    chassis: &nv_redfish::chassis::Chassis<B>,
    bmc_name: &str,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let raw_chassis = chassis.raw();
    let Some(nav) = raw_chassis.network_adapters.as_ref() else {
        return Ok(());
    };
    let collection = bmc
        .get::<RawNetworkAdapterCollection>(nav.odata_id())
        .await
        .map_err(|error| format!("raw collection: {error}"))?;
    let total = collection.members.len();
    let mut failures = 0usize;
    for member in &collection.members {
        match bmc.get::<NetworkAdapterSchema>(&member.id).await {
            Ok(adapter) => collect_raw_network_adapter(bmc, bmc_name, &adapter, out).await,
            Err(error) => {
                failures += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    adapter = %member.id,
                    error = %error,
                    "raw network adapter member fetch failed"
                );
            }
        }
    }
    if total > 0 && failures == total {
        return Err(format!(
            "all {failures}/{total} raw network adapter members failed"
        ));
    }
    Ok(())
}

async fn collect_raw_network_adapter<B: Bmc>(
    bmc: &B,
    bmc_name: &str,
    adapter: &NetworkAdapterSchema,
    out: &mut Vec<Metric>,
) {
    let resource_id = adapter.odata_id().to_string();
    let (health, state) = status_labels(adapter.status.as_ref());
    push_health(
        out,
        bmc_name,
        "network_adapter",
        &resource_id,
        &health,
        &state,
    );
    if let Some(value) = adapter.manufacturer.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "network_adapter",
            &resource_id,
            "manufacturer",
            &value,
        );
    }
    if let Some(value) = adapter.model.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "network_adapter",
            &resource_id,
            "model",
            &value,
        );
    }
    if let Some(value) = adapter
        .controllers
        .as_ref()
        .and_then(|controllers| controllers.first())
        .and_then(|controller| controller.firmware_package_version.clone().flatten())
    {
        push_info(
            out,
            bmc_name,
            "network_adapter",
            &resource_id,
            "firmware_version",
            &value,
        );
    }

    let Some(ports_nav) = adapter.ports.as_ref() else {
        return;
    };
    match ports_nav.get(bmc).await {
        Ok(ports) => {
            for member in &ports.members {
                match member.get(bmc).await {
                    Ok(port) => {
                        let resource_id = port.odata_id().to_string();
                        let (health, state) = status_labels(port.status.as_ref());
                        push_health(out, bmc_name, "port", &resource_id, &health, &state);
                    }
                    Err(error) => tracing::warn!(
                        bmc = %bmc_name,
                        adapter = %adapter.odata_id(),
                        error = %error,
                        "raw network adapter port fetch failed"
                    ),
                }
            }
        }
        Err(error) => tracing::warn!(
            bmc = %bmc_name,
            adapter = %adapter.odata_id(),
            error = %error,
            "raw network adapter port collection fetch failed"
        ),
    }
}

async fn collect_network_adapter<B: Bmc>(
    bmc_name: &str,
    adapter: &NetworkAdapter<B>,
    out: &mut Vec<Metric>,
) {
    let raw = adapter.raw();
    let resource_id = adapter.odata_id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(
        out,
        bmc_name,
        "network_adapter",
        &resource_id,
        &health,
        &state,
    );
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "network_adapter",
            &resource_id,
            "manufacturer",
            &value,
        );
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "network_adapter",
            &resource_id,
            "model",
            &value,
        );
    }
    if let Some(value) = raw
        .controllers
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.firmware_package_version.clone().flatten())
    {
        push_info(
            out,
            bmc_name,
            "network_adapter",
            &resource_id,
            "firmware_version",
            &value,
        );
    }
    match adapter.ports().await {
        Ok(Some(ports)) => match ports.members().await {
            Ok(ports) => {
                for port in ports {
                    let raw = port.raw();
                    let resource_id = port.odata_id().to_string();
                    let (health, state) = status_labels(raw.status.as_ref());
                    push_health(out, bmc_name, "port", &resource_id, &health, &state);
                }
            }
            Err(error) => {
                tracing::warn!(
                    bmc = %bmc_name,
                    adapter = %adapter.id(),
                    error = %error,
                    "network adapter port members fetch failed"
                );
            }
        },
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                bmc = %bmc_name,
                adapter = %adapter.id(),
                error = %error,
                "network adapter ports fetch failed"
            );
        }
    }
}

fn collect_pcie_device<B: Bmc>(
    bmc_name: &str,
    chassis_id: &str,
    device: &PcieDevice<B>,
    out: &mut Vec<Metric>,
) {
    let raw = device.raw();
    let id = device.id().to_string();
    let resource_id = device.odata_id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "pcie_device", &resource_id, &health, &state);
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "pcie_device",
            &resource_id,
            "manufacturer",
            &value,
        );
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(out, bmc_name, "pcie_device", &resource_id, "model", &value);
    }
    if let Some(value) = raw.serial_number.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "pcie_device",
            &resource_id,
            "serial_number",
            &value,
        );
    }
    if let Some(value) = raw.firmware_version.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "pcie_device",
            &resource_id,
            "firmware_version",
            &value,
        );
    }
    let Some(pcie_interface) = &raw.pcie_interface else {
        return;
    };
    push_device_value(
        out,
        bmc_name,
        chassis_id,
        &id,
        PCIE_DEVICE_LANES_IN_USE,
        pcie_interface.lanes_in_use.flatten().map(|v| v as f64),
    );
    push_device_value(
        out,
        bmc_name,
        chassis_id,
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
    chassis_id: &str,
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
            .label("chassis", chassis_id.to_string())
            .label("id", id.to_string())
            .build(value),
    );
}
