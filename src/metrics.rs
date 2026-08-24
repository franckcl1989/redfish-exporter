use thiserror::Error;

pub const UP: (&str, &str) = (
    "redfish_up",
    "Whether the last scrape of this BMC succeeded",
);
pub const BUILD_INFO: (&str, &str) = ("redfish_build_info", "Build information");
pub const SCRAPE_ERRORS_TOTAL: (&str, &str) = (
    "redfish_scrape_errors_total",
    "Total number of failed resources across all scrape rounds",
);
pub const SCRAPE_DURATION: (&str, &str) = (
    "redfish_scrape_duration_seconds",
    "Duration of the last scrape of this BMC",
);
pub const SCRAPE_ERROR: (&str, &str) = (
    "redfish_scrape_error",
    "Set to 1 when the last scrape of a resource failed",
);
pub const HEALTH_STATUS: (&str, &str) = ("redfish_health_status", "Health and state of a resource");
pub const INFO: (&str, &str) = ("redfish_info", "Static key-value information about a BMC");
pub const SENSOR_READING: (&str, &str) = ("redfish_sensor_reading", "Sensor reading");
pub const THRESHOLD_PREFIX: &str = "redfish_sensor_threshold_";
pub const POWER_CONSUMPTION: (&str, &str) = (
    "redfish_power_consumption_watts",
    "Total chassis power consumption in watts",
);
pub const POWER_CONSUMPTION_MIN: (&str, &str) = (
    "redfish_power_consumption_min_watts",
    "Minimum chassis power consumption in watts over the measurement window",
);
pub const POWER_CONSUMPTION_MAX: (&str, &str) = (
    "redfish_power_consumption_max_watts",
    "Maximum chassis power consumption in watts over the measurement window",
);
pub const POWER_CONSUMPTION_AVG: (&str, &str) = (
    "redfish_power_consumption_avg_watts",
    "Average chassis power consumption in watts over the measurement window",
);
pub const POWER_CONSUMPTION_INTERVAL: (&str, &str) = (
    "redfish_power_consumption_interval_minutes",
    "Power consumption measurement window in minutes",
);
pub const POWER_SUPPLY_EFFICIENCY: (&str, &str) = (
    "redfish_power_supply_efficiency_percent",
    "Power supply efficiency in percent",
);
pub const POWER_SUPPLY_INPUT_WATTS: (&str, &str) = (
    "redfish_power_supply_input_watts",
    "Power supply input power in watts",
);
pub const POWER_SUPPLY_CAPACITY: (&str, &str) = (
    "redfish_power_supply_capacity_watts",
    "Power supply rated capacity in watts",
);
pub const POWER_SUPPLY_INPUT_VOLTAGE: (&str, &str) = (
    "redfish_power_supply_input_voltage",
    "Power supply line input voltage in volts",
);
pub const POWER_INPUT: (&str, &str) = ("redfish_power_input_watts", "Chassis power input in watts");
pub const POWER_STATE: (&str, &str) = ("redfish_power_state", "Power state of a system, 1 = On");
pub const PROCESSOR_TEMPERATURE: (&str, &str) = (
    "redfish_processor_temperature_celsius",
    "Processor temperature in Celsius",
);
pub const PROCESSOR_POWER: (&str, &str) =
    ("redfish_processor_power_watts", "Processor power in watts");
pub const MEMORY_CAPACITY: (&str, &str) =
    ("redfish_memory_capacity_bytes", "Memory capacity in bytes");
pub const MEMORY_BANDWIDTH: (&str, &str) = (
    "redfish_memory_bandwidth_percent",
    "Memory bandwidth utilization percentage",
);
pub const MEMORY_CORRECTABLE: (&str, &str) = (
    "redfish_memory_correctable_errors",
    "Memory correctable ECC alarm trip, 1 = tripped",
);
pub const MEMORY_UNCORRECTABLE: (&str, &str) = (
    "redfish_memory_uncorrectable_errors",
    "Memory uncorrectable ECC alarm trip, 1 = tripped",
);
pub const VOLUME_CAPACITY: (&str, &str) =
    ("redfish_volume_capacity_bytes", "Volume capacity in bytes");
pub const DRIVE_CAPACITY: (&str, &str) =
    ("redfish_drive_capacity_bytes", "Drive capacity in bytes");
pub const LINK_STATUS: (&str, &str) = (
    "redfish_ethernet_interface_link_status",
    "Ethernet link status, 1 = up",
);
pub const LINK_SPEED: (&str, &str) = (
    "redfish_ethernet_interface_speed_mbps",
    "Ethernet link speed in Mbps",
);

#[derive(Clone, Debug)]
pub struct Metric {
    pub name: &'static str,
    pub help: &'static str,
    pub labels: Vec<(&'static str, String)>,
    pub value: f64,
}

pub struct MetricBuilder {
    name: &'static str,
    help: &'static str,
    labels: Vec<(&'static str, String)>,
}

impl Metric {
    pub fn gauge(name: &'static str, help: &'static str) -> MetricBuilder {
        MetricBuilder {
            name,
            help,
            labels: Vec::new(),
        }
    }
}

impl MetricBuilder {
    pub fn label(mut self, key: &'static str, value: String) -> Self {
        self.labels.push((key, value));
        self
    }
    pub fn build(self, value: f64) -> Metric {
        Metric {
            name: self.name,
            help: self.help,
            labels: self.labels,
            value,
        }
    }
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("prometheus error: {0}")]
    Prometheus(#[from] prometheus::Error),
}

/// 把所有指标注册进 registry。同名（name+help）指标合并为同一个 GaugeVec（不同 label 集时为不同序列）。
pub fn register_into(
    metrics: &[Metric],
    registry: &prometheus::Registry,
) -> Result<(), MetricsError> {
    use std::collections::HashMap;

    // 第一阶段：收集每个 (name, help) 组的 label 名并集（排序去重）。
    // GaugeVec 需预先声明 label 名集合；不同 label 集的指标共享同一 GaugeVec，
    // 缺失 label 以空值兜底（等价守护测试钉住的行为）。
    let mut group_names: HashMap<(&'static str, &'static str), Vec<&'static str>> =
        HashMap::with_capacity(metrics.len());
    for m in metrics {
        group_names
            .entry((m.name, m.help))
            .or_default()
            .extend(m.labels.iter().map(|(k, _)| *k));
    }
    for names in group_names.values_mut() {
        names.sort();
        names.dedup();
    }

    // 第二阶段：每组一个 GaugeVec。
    // 不变量：GaugeVec 以 'static 字面量名构造，不可能失败；失败即程序缺陷，panic 合理。
    let mut vecs: HashMap<(&'static str, &'static str), prometheus::GaugeVec> =
        HashMap::with_capacity(group_names.len());
    for ((name, help), names) in &group_names {
        vecs.insert(
            (*name, *help),
            prometheus::GaugeVec::new(prometheus::Opts::new(*name, *help), names)
                .expect("GaugeVec construction with &'static str names cannot fail"),
        );
    }

    // 第三阶段：逐指标 set。
    // 微观优化：每 metric 先建 label 映射，取值 O(L)（原实现每 label 线性查找 O(L²)）。
    for m in metrics {
        let names = &group_names[&(m.name, m.help)];
        let mut lookup: HashMap<&'static str, &str> = HashMap::with_capacity(m.labels.len());
        for (k, v) in &m.labels {
            lookup.entry(*k).or_insert(v.as_str());
        }
        let label_values: Vec<String> = names
            .iter()
            .map(|n| lookup.get(n).copied().unwrap_or_default().to_string())
            .collect();
        debug_assert_eq!(
            label_values.len(),
            names.len(),
            "label cardinality mismatch for '{}'",
            m.name
        );
        let gv = &vecs[&(m.name, m.help)];
        gv.with_label_values(&label_values).set(m.value);
    }
    for (_, gv) in vecs {
        match registry.register(Box::new(gv)) {
            Ok(()) => {}
            // 同一 registry 中同名指标重复注册视为合并（collect 阶段已分组），忽略
            Err(prometheus::Error::AlreadyReg) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// 编码 registry 为 Prometheus 文本格式字节（快照预编码缓存使用）。
pub fn encode_bytes(registry: &prometheus::Registry) -> Vec<u8> {
    let mut buf = Vec::new();
    encode_bytes_into(registry, &mut buf);
    buf
}

/// 编码写入调用方缓冲（可复用：clear 后重复调用避免反复增长分配）。
pub fn encode_bytes_into(registry: &prometheus::Registry, out: &mut Vec<u8>) {
    use prometheus::{Encoder, TextEncoder};
    // 不变量：encode 写入 Vec<u8> 不会失败；失败即程序缺陷，panic 合理。
    TextEncoder::new()
        .encode(&registry.gather(), out)
        .expect("TextEncoder::encode writes to Vec<u8> and cannot fail");
}

pub fn encode(registry: &prometheus::Registry) -> String {
    String::from_utf8(encode_bytes(registry)).expect("TextEncoder output is always valid UTF-8")
}

pub fn unbox_reading(v: Option<Option<f64>>) -> Option<f64> {
    v.flatten()
}

pub fn health_state_labels(health: Option<&str>, state: Option<&str>) -> (String, String) {
    (
        health.unwrap_or("unknown").to_string(),
        state.unwrap_or("unknown").to_string(),
    )
}
