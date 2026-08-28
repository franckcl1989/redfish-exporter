use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{INDICATOR_LED, Metric, POWER_STATE};
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
        let resource_id = system.odata_id().to_string();
        if let Some(power_state) = system.power_state() {
            out.push(
                Metric::gauge(POWER_STATE.0, POWER_STATE.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.clone())
                    .build(f64::from(matches!(power_state, PowerState::On))),
            );
        }
        let raw = system.raw();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(&mut out, bmc_name, "system", &resource_id, &health, &state);
        // IndicatorLED：nv-redfish 0.15 编译 schema 为 Option<Option<IndicatorLed>> 枚举
        // （Unknown/Lit/Blinking/Off/UnsupportedValue），无 Display 实现，用 Debug 输出变体名。
        if let Some(led) = raw.indicator_led.flatten() {
            out.push(
                Metric::gauge(INDICATOR_LED.0, INDICATOR_LED.1)
                    .label("bmc", bmc_name.to_string())
                    .label("resource_type", "system".to_string())
                    .label("id", system_id.clone())
                    .label("state", format!("{led:?}"))
                    .build(1.0),
            );
        }
        if let Some(value) = raw.manufacturer.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "system",
                &resource_id,
                "manufacturer",
                &value,
            );
        }
        if let Some(value) = raw.model.clone().flatten() {
            push_info(&mut out, bmc_name, "system", &resource_id, "model", &value);
        }
        if let Some(value) = raw.serial_number.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "system",
                &resource_id,
                "serial_number",
                &value,
            );
        }
        if let Some(value) = raw.sku.clone().flatten() {
            push_info(&mut out, bmc_name, "system", &resource_id, "sku", &value);
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
        let resource_id = chassis.odata_id().to_string();
        let raw = chassis.raw();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(&mut out, bmc_name, "chassis", &resource_id, &health, &state);
        // IndicatorLED：同 systems 遍历，无该字段的机箱（如 Dell 背板机箱）不产出。
        if let Some(led) = raw.indicator_led.flatten() {
            out.push(
                Metric::gauge(INDICATOR_LED.0, INDICATOR_LED.1)
                    .label("bmc", bmc_name.to_string())
                    .label("resource_type", "chassis".to_string())
                    .label("id", chassis_id.clone())
                    .label("state", format!("{led:?}"))
                    .build(1.0),
            );
        }
        if let Some(value) = raw.manufacturer.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "chassis",
                &resource_id,
                "manufacturer",
                &value,
            );
        }
        if let Some(value) = raw.model.clone().flatten() {
            push_info(&mut out, bmc_name, "chassis", &resource_id, "model", &value);
        }
        if let Some(value) = raw.serial_number.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "chassis",
                &resource_id,
                "serial_number",
                &value,
            );
        }
        if let Some(value) = raw.part_number.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "chassis",
                &resource_id,
                "part_number",
                &value,
            );
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
        let resource_id = manager.odata_id().to_string();
        let raw = manager.raw();
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(&mut out, bmc_name, "manager", &resource_id, &health, &state);
        if let Some(value) = raw.manufacturer.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "manager",
                &resource_id,
                "manufacturer",
                &value,
            );
        }
        if let Some(value) = raw.model.clone().flatten() {
            push_info(&mut out, bmc_name, "manager", &resource_id, "model", &value);
        }
        if let Some(value) = raw.firmware_version.clone().flatten() {
            push_info(
                &mut out,
                bmc_name,
                "manager",
                &resource_id,
                "firmware_version",
                &value,
            );
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
    let mut attempted = 0usize;
    let mut failed = 0usize;
    for chassis in chassis_members {
        let chassis_id = chassis.id().to_string();
        let assembly = match chassis.assembly().await {
            Ok(Some(assembly)) => {
                attempted += 1;
                assembly
            }
            Ok(None) => continue,
            Err(error) => {
                attempted += 1;
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "assembly resource fetch failed"
                );
                continue;
            }
        };
        let assemblies = match assembly.assemblies().await {
            Ok(assemblies) => assemblies,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "assembly members fetch failed"
                );
                continue;
            }
        };
        for assembly_data in assemblies {
            let raw = assembly_data.raw();
            let assembly_id = raw.odata_id().to_string();
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
                push_info(
                    &mut out,
                    bmc_name,
                    "assembly",
                    &assembly_id,
                    "producer",
                    &value,
                );
            }
            if let Some(value) = raw.model.clone().flatten() {
                push_info(
                    &mut out,
                    bmc_name,
                    "assembly",
                    &assembly_id,
                    "model",
                    &value,
                );
            }
            if let Some(value) = raw.part_number.clone().flatten() {
                push_info(
                    &mut out,
                    bmc_name,
                    "assembly",
                    &assembly_id,
                    "part_number",
                    &value,
                );
            }
            if let Some(value) = raw.serial_number.clone().flatten() {
                push_info(
                    &mut out,
                    bmc_name,
                    "assembly",
                    &assembly_id,
                    "serial_number",
                    &value,
                );
            }
        }
    }
    if attempted > 0 && failed == attempted {
        return Err(format!(
            "assembly: all {failed}/{attempted} declared resources failed"
        ));
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
        let id = item.odata_id().to_string();
        if let Some(version) = item.version() {
            push_info(
                out,
                bmc_name,
                "software_inventory",
                &id,
                version_key,
                &format!("{version}"),
            );
        }
        let (health, state) = status_labels(raw.status.as_ref());
        push_health(out, bmc_name, "software_inventory", &id, &health, &state);
    }
}
