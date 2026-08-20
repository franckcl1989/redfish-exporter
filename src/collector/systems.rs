use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{Metric, POWER_STATE};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::core::EntityTypeRef as _;
use nv_redfish::schema::resource::PowerState;
use std::sync::Arc;

pub async fn collect_systems<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(systems) = root.systems().await.map_err(|e| format!("systems: {e}"))? else {
        return Ok(out);
    };
    let systems = systems
        .members()
        .await
        .map_err(|e| format!("systems members: {e}"))?;
    for system in systems {
        let system_id = system.id().to_string();
        if matches!(system.power_state(), Some(PowerState::On)) {
            out.push(
                Metric::gauge(POWER_STATE.0, POWER_STATE.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.clone())
                    .build(1.0),
            );
        }
        let raw = system.raw();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(&mut out, bmc_name, "system", &system_id, &health, &state);
        if let Some(value) = raw.manufacturer.clone().flatten() {
            push_info(&mut out, bmc_name, "manufacturer", &value);
        }
        if let Some(value) = raw.model.clone().flatten() {
            push_info(&mut out, bmc_name, "model", &value);
        }
        if let Some(value) = raw.serial_number.clone().flatten() {
            push_info(&mut out, bmc_name, "serial_number", &value);
        }
        if let Some(value) = raw.sku.clone().flatten() {
            push_info(&mut out, bmc_name, "sku", &value);
        }
    }
    Ok(out)
}

pub async fn collect_chassis_health<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(chassis_collection) = root.chassis().await.map_err(|e| format!("chassis: {e}"))?
    else {
        return Ok(out);
    };
    let chassis_members = chassis_collection
        .members()
        .await
        .map_err(|e| format!("chassis members: {e}"))?;
    for chassis in chassis_members {
        let chassis_id = chassis.id().to_string();
        let raw = chassis.raw();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(&mut out, bmc_name, "chassis", &chassis_id, &health, &state);
        if let Some(value) = raw.manufacturer.clone().flatten() {
            push_info(&mut out, bmc_name, "manufacturer", &value);
        }
        if let Some(value) = raw.model.clone().flatten() {
            push_info(&mut out, bmc_name, "model", &value);
        }
        if let Some(value) = raw.serial_number.clone().flatten() {
            push_info(&mut out, bmc_name, "serial_number", &value);
        }
        if let Some(value) = raw.part_number.clone().flatten() {
            push_info(&mut out, bmc_name, "part_number", &value);
        }
    }
    Ok(out)
}

pub async fn collect_managers<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(managers) = root
        .managers()
        .await
        .map_err(|e| format!("managers: {e}"))?
    else {
        return Ok(out);
    };
    let managers = managers
        .members()
        .await
        .map_err(|e| format!("managers members: {e}"))?;
    for manager in managers {
        let manager_id = manager.id().to_string();
        let raw = manager.raw();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(&mut out, bmc_name, "manager", &manager_id, &health, &state);
        if let Some(value) = raw.manufacturer.clone().flatten() {
            push_info(&mut out, bmc_name, "manufacturer", &value);
        }
        if let Some(value) = raw.model.clone().flatten() {
            push_info(&mut out, bmc_name, "model", &value);
        }
        if let Some(value) = raw.firmware_version.clone().flatten() {
            push_info(&mut out, bmc_name, "firmware_version", &value);
        }
    }
    Ok(out)
}

pub async fn collect_assembly<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(chassis_collection) = root.chassis().await.map_err(|e| format!("chassis: {e}"))?
    else {
        return Ok(out);
    };
    let chassis_members = chassis_collection
        .members()
        .await
        .map_err(|e| format!("chassis members: {e}"))?;
    for chassis in chassis_members {
        let Ok(Some(assembly)) = chassis.assembly().await else {
            continue;
        };
        let Ok(assemblies) = assembly.assemblies().await else {
            continue;
        };
        for assembly_data in assemblies {
            let raw = assembly_data.raw();
            let assembly_id = raw
                .odata_id()
                .last_segment()
                .unwrap_or_default()
                .to_string();
            let (health, state) = status_labels(raw.status.as_ref());
            push_health(
                &mut out,
                bmc_name,
                "assembly",
                &assembly_id,
                &health,
                &state,
            );
            if let Some(value) = raw.producer.clone().flatten() {
                push_info(&mut out, bmc_name, "producer", &value);
            }
            if let Some(value) = raw.model.clone().flatten() {
                push_info(&mut out, bmc_name, "model", &value);
            }
            if let Some(value) = raw.part_number.clone().flatten() {
                push_info(&mut out, bmc_name, "part_number", &value);
            }
            if let Some(value) = raw.serial_number.clone().flatten() {
                push_info(&mut out, bmc_name, "serial_number", &value);
            }
        }
    }
    Ok(out)
}

pub async fn collect_firmware<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(update_service) = root
        .update_service()
        .await
        .map_err(|e| format!("update service: {e}"))?
    else {
        return Ok(out);
    };
    if let Some(firmwares) = update_service
        .firmware_inventories()
        .await
        .map_err(|e| format!("firmware inventories: {e}"))?
    {
        collect_inventory_items(&mut out, bmc_name, "firmware_version", &firmwares);
    }
    let Some(softwares) = update_service
        .software_inventories()
        .await
        .map_err(|e| format!("software inventories: {e}"))?
    else {
        return Ok(out);
    };
    collect_inventory_items(&mut out, bmc_name, "software_version", &softwares);
    Ok(out)
}

fn collect_inventory_items<B: Bmc>(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    version_key: &str,
    items: &[nv_redfish::update_service::SoftwareInventory<B>],
) {
    for item in items {
        let raw = item.raw();
        let id = raw.base.id.clone();
        if let Some(version) = item.version() {
            push_info(out, bmc_name, version_key, &format!("{version}"));
        }
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(out, bmc_name, "software_inventory", &id, &health, &state);
    }
}
