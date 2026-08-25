use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{
    Metric, PROCESSOR_FREQUENCY, PROCESSOR_MAX_FREQUENCY, PROCESSOR_POWER, PROCESSOR_TEMPERATURE,
    PROCESSOR_VOLTAGE, unbox_reading,
};
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
    // 频率：MaxSpeedMHz 为标准字段；当前频率取厂商 OEM 字段
    // （Dell CurrentClockSpeedMhz / 浪潮 Public.FrequencyMHz），
    // 缺失时该指标不产出（真机探测：Dell 2100 / 浪潮 2100）。
    push_value(
        out,
        bmc_name,
        system_id,
        &id,
        PROCESSOR_MAX_FREQUENCY,
        raw.max_speed_mhz.flatten().map(|v| v as f64),
    );
    // OEM 扩展经 serde flatten 落在 base 链的 Item.oem（ProcessorSchema 无顶层 oem 字段）。
    if let Some(oem) = raw.base.base.oem.as_ref() {
        // Dell DellProcessor 子对象：当前频率与电压共用同一路径前缀，提取一次复用。
        let dell_processor =
            nv_redfish::oem::oem_value(oem, "Dell").and_then(|d| d.get("DellProcessor"));
        // 当前频率：Dell 优先，浪潮 Public 兜底（以真机探测路径为准）。
        let freq = dell_processor
            .and_then(|p| p.get("CurrentClockSpeedMhz"))
            .and_then(|v| v.as_f64())
            .or_else(|| {
                nv_redfish::oem::oem_value(oem, "Public")
                    .and_then(|p| p.get("FrequencyMHz"))
                    .and_then(|v| v.as_f64())
            });
        push_value(out, bmc_name, system_id, &id, PROCESSOR_FREQUENCY, freq);
        // 电压：Dell DellProcessor.Volts 为字符串，解析失败则跳过（不报错、不产出）。
        let volts = dell_processor
            .and_then(|p| p.get("Volts"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok());
        push_value(out, bmc_name, system_id, &id, PROCESSOR_VOLTAGE, volts);
    }
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
