use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{
    MEMORY_BANDWIDTH, MEMORY_CAPACITY, MEMORY_CORRECTABLE, MEMORY_UNCORRECTABLE, Metric,
    unbox_reading,
};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use std::sync::Arc;

pub async fn collect_memory<B: Bmc>(
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
    let mut attempted = 0usize;
    let mut failed = 0usize;
    for system in systems {
        let system_id = system.id().to_string();
        let modules = match system.memory_modules().await {
            Ok(Some(modules)) => {
                attempted += 1;
                modules
            }
            Ok(None) => continue,
            Err(error) => {
                attempted += 1;
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    error = %error,
                    "memory collection fetch failed"
                );
                continue;
            }
        };
        for module in modules {
            collect_module(bmc_name, &system_id, &module, &mut out).await;
        }
    }
    if attempted > 0 && failed == attempted {
        return Err(format!(
            "memory: all {failed}/{attempted} declared collections failed"
        ));
    }
    Ok(out)
}

async fn collect_module<B: Bmc>(
    bmc_name: &str,
    system_id: &str,
    module: &nv_redfish::computer_system::Memory<B>,
    out: &mut Vec<Metric>,
) {
    let raw = module.raw();
    let id = module.id().to_string();
    let resource_id = module.odata_id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "memory", &resource_id, &health, &state);
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "memory",
            &resource_id,
            "manufacturer",
            &value,
        );
    }
    if let Some(value) = raw.part_number.clone().flatten() {
        push_info(out, bmc_name, "memory", &resource_id, "part_number", &value);
    }
    if let Some(value) = raw.memory_type.flatten() {
        push_info(
            out,
            bmc_name,
            "memory",
            &resource_id,
            "memory_type",
            &format!("{value:?}"),
        );
    }
    push_capacity(out, bmc_name, system_id, &id, raw.capacity_mi_b.flatten());
    let metrics = match module.metrics().await {
        Ok(Some(metrics)) => metrics,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                bmc = %bmc_name,
                system = %system_id,
                memory = %id,
                error = %error,
                "memory metrics fetch failed"
            );
            return;
        }
    };
    let raw_metrics = metrics.raw();
    push_value(
        out,
        bmc_name,
        system_id,
        &id,
        MEMORY_BANDWIDTH,
        unbox_reading(raw_metrics.bandwidth_percent),
    );
    if let Some(health_data) = &raw_metrics.health_data
        && let Some(trips) = &health_data.alarm_trips
    {
        if let Some(Some(v)) = trips.correctable_ecc_error {
            push_bool(out, bmc_name, system_id, &id, MEMORY_CORRECTABLE, v);
        }
        if let Some(Some(v)) = trips.uncorrectable_ecc_error {
            push_bool(out, bmc_name, system_id, &id, MEMORY_UNCORRECTABLE, v);
        }
    }
}

fn push_bool(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    system_id: &str,
    id: &str,
    name_help: (&'static str, &'static str),
    value: bool,
) {
    push_value(
        out,
        bmc_name,
        system_id,
        id,
        name_help,
        Some(if value { 1.0 } else { 0.0 }),
    );
}

fn push_value(
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

fn push_capacity(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    system_id: &str,
    id: &str,
    capacity_mib: Option<i64>,
) {
    let Some(capacity_mib) = capacity_mib else {
        return;
    };
    out.push(
        Metric::gauge(MEMORY_CAPACITY.0, MEMORY_CAPACITY.1)
            .label("bmc", bmc_name.to_string())
            .label("system", system_id.to_string())
            .label("id", id.to_string())
            .build(capacity_mib as f64 * 1048576.0),
    );
}
