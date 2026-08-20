use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{Metric, PROCESSOR_POWER, PROCESSOR_TEMPERATURE, unbox_reading};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use std::sync::Arc;

const PROCESSOR_BANDWIDTH: (&str, &str) = (
    "redfish_processor_bandwidth_percent",
    "Processor bandwidth utilization percentage",
);

pub async fn collect_processors<B: Bmc>(
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
        let Ok(Some(processors)) = system.processors().await else {
            continue;
        };
        for processor in processors {
            collect_processor(bmc_name, &system_id, &processor, &mut out).await;
        }
    }
    Ok(out)
}

async fn collect_processor<B: Bmc>(
    bmc_name: &str,
    system_id: &str,
    processor: &nv_redfish::computer_system::Processor<B>,
    out: &mut Vec<Metric>,
) {
    let raw = processor.raw();
    let id = processor.id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "processor", &id, &health, &state);
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(out, bmc_name, "manufacturer", &value);
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(out, bmc_name, "model", &value);
    }
    if let Some(value) = raw.processor_type.flatten() {
        push_info(out, bmc_name, "processor_type", &format!("{value:?}"));
    }
    let Ok(Some(metrics)) = processor.metrics().await else {
        return;
    };
    let metrics = metrics.raw();
    push_value(
        out,
        bmc_name,
        system_id,
        &id,
        PROCESSOR_TEMPERATURE,
        unbox_reading(metrics.temperature_celsius),
    );
    push_value(
        out,
        bmc_name,
        system_id,
        &id,
        PROCESSOR_POWER,
        unbox_reading(metrics.consumed_power_watt),
    );
    push_value(
        out,
        bmc_name,
        system_id,
        &id,
        PROCESSOR_BANDWIDTH,
        unbox_reading(metrics.bandwidth_percent),
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
