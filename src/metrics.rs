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
pub const INFO: (&str, &str) = (
    "redfish_info",
    "Static key-value information about a Redfish resource",
);
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
pub const POWER_STATE: (&str, &str) = (
    "redfish_power_state",
    "Power state of a system, 1 = On and 0 = any other known state",
);
pub const INDICATOR_LED: (&str, &str) = (
    "redfish_indicator_led",
    "Indicator LED state, 1 = present with state label",
);
pub const PROCESSOR_TEMPERATURE: (&str, &str) = (
    "redfish_processor_temperature_celsius",
    "Processor temperature in Celsius",
);
pub const PROCESSOR_POWER: (&str, &str) =
    ("redfish_processor_power_watts", "Processor power in watts");
pub const PROCESSOR_FREQUENCY: (&str, &str) = (
    "redfish_processor_frequency_mhz",
    "Current operating frequency of the processor in MHz",
);
pub const PROCESSOR_MAX_FREQUENCY: (&str, &str) = (
    "redfish_processor_max_frequency_mhz",
    "Maximum rated frequency of the processor in MHz",
);
pub const PROCESSOR_VOLTAGE: (&str, &str) = (
    "redfish_processor_voltage_volts",
    "Processor input voltage in volts (vendor OEM field when present)",
);
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
pub const DRIVE_INFO: (&str, &str) = (
    "redfish_drive_info",
    "Drive vendor identifier information, 1 = present",
);
pub const DRIVE_OEM_STATUS: (&str, &str) = (
    "redfish_drive_oem_status",
    "Drive vendor OEM status, 1 = present with status labels",
);
pub const STORAGE_CONTROLLER_INFO: (&str, &str) = (
    "redfish_storage_controller_info",
    "Storage controller model and firmware information, 1 = present",
);
pub const STORAGE_CONTROLLER_STATUS: (&str, &str) = (
    "redfish_storage_controller_status",
    "Storage controller status, 1 = present with status label",
);
pub const LINK_STATUS: (&str, &str) = (
    "redfish_ethernet_interface_link_status",
    "Ethernet link status, 1 = up",
);
pub const LINK_SPEED: (&str, &str) = (
    "redfish_ethernet_interface_speed_mbps",
    "Ethernet link speed in Mbps",
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetricKind {
    Gauge,
    Counter,
}

#[derive(Clone, Debug)]
pub struct Metric {
    pub name: &'static str,
    pub help: &'static str,
    pub labels: Vec<(&'static str, String)>,
    pub value: f64,
    pub kind: MetricKind,
}

pub struct MetricBuilder {
    name: &'static str,
    help: &'static str,
    labels: Vec<(&'static str, String)>,
    kind: MetricKind,
}

impl Metric {
    pub fn gauge(name: &'static str, help: &'static str) -> MetricBuilder {
        MetricBuilder {
            name,
            help,
            labels: Vec::new(),
            kind: MetricKind::Gauge,
        }
    }

    pub fn counter(name: &'static str, help: &'static str) -> MetricBuilder {
        MetricBuilder {
            name,
            help,
            labels: Vec::new(),
            kind: MetricKind::Counter,
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
            kind: self.kind,
        }
    }
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("prometheus error: {0}")]
    Prometheus(#[from] prometheus::Error),
    #[error("metric '{name}' has conflicting descriptors: {details}")]
    ConflictingDescriptor {
        name: &'static str,
        details: &'static str,
    },
    #[error("metric '{name}' contains duplicate label '{label}'")]
    DuplicateLabel {
        name: &'static str,
        label: &'static str,
    },
    #[error("metric '{name}' contains more than one sample with the same label values")]
    DuplicateSeries { name: &'static str },
    #[error("counter metric '{name}' has invalid value {value}")]
    InvalidCounterValue { name: &'static str, value: f64 },
}

/// 把所有指标注册进 registry。同名（name+help）指标合并为同一个 GaugeVec（不同 label 集时为不同序列）。
pub fn register_into(
    metrics: &[Metric],
    registry: &prometheus::Registry,
) -> Result<(), MetricsError> {
    use std::collections::HashMap;

    #[derive(Clone)]
    struct Descriptor {
        help: &'static str,
        kind: MetricKind,
        label_names: Vec<&'static str>,
    }

    // 第一阶段：按名称收集 descriptor 与 label 名并集。相同名称如果 help/type
    // 不一致必须失败，不能依赖 HashMap 迭代顺序随机丢弃其中一组。
    // GaugeVec 需预先声明 label 名集合；不同 label 集的指标共享同一 GaugeVec，
    // 缺失 label 以空值兜底（等价守护测试钉住的行为）。
    let mut descriptors: HashMap<&'static str, Descriptor> = HashMap::with_capacity(metrics.len());
    let mut series = std::collections::HashSet::with_capacity(metrics.len());
    for m in metrics {
        let mut seen = std::collections::HashSet::with_capacity(m.labels.len());
        for (key, _) in &m.labels {
            if !seen.insert(*key) {
                return Err(MetricsError::DuplicateLabel {
                    name: m.name,
                    label: key,
                });
            }
        }
        match descriptors.entry(m.name) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let descriptor = entry.get_mut();
                if descriptor.help != m.help {
                    return Err(MetricsError::ConflictingDescriptor {
                        name: m.name,
                        details: "help text differs",
                    });
                }
                if descriptor.kind != m.kind {
                    return Err(MetricsError::ConflictingDescriptor {
                        name: m.name,
                        details: "metric type differs",
                    });
                }
                descriptor
                    .label_names
                    .extend(m.labels.iter().map(|(key, _)| *key));
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Descriptor {
                    help: m.help,
                    kind: m.kind,
                    label_names: m.labels.iter().map(|(key, _)| *key).collect(),
                });
            }
        }
    }
    for descriptor in descriptors.values_mut() {
        descriptor.label_names.sort_unstable();
        descriptor.label_names.dedup();
    }

    enum MetricVec {
        Gauge(prometheus::GaugeVec),
        Counter(prometheus::CounterVec),
    }

    let mut vecs: HashMap<&'static str, MetricVec> = HashMap::with_capacity(descriptors.len());
    for (name, descriptor) in &descriptors {
        let opts = prometheus::Opts::new(*name, descriptor.help);
        let metric_vec = match descriptor.kind {
            MetricKind::Gauge => {
                MetricVec::Gauge(prometheus::GaugeVec::new(opts, &descriptor.label_names)?)
            }
            MetricKind::Counter => {
                MetricVec::Counter(prometheus::CounterVec::new(opts, &descriptor.label_names)?)
            }
        };
        vecs.insert(*name, metric_vec);
    }

    // 第三阶段：逐指标 set。
    // 微观优化：每 metric 先建 label 映射，取值 O(L)（原实现每 label 线性查找 O(L²)）。
    for m in metrics {
        let names = &descriptors[m.name].label_names;
        let mut lookup: HashMap<&'static str, &str> = HashMap::with_capacity(m.labels.len());
        for (k, v) in &m.labels {
            lookup.insert(*k, v.as_str());
        }
        let label_values: Vec<&str> = names
            .iter()
            .map(|n| lookup.get(n).copied().unwrap_or_default())
            .collect();
        if !series.insert((m.name, label_values.clone())) {
            return Err(MetricsError::DuplicateSeries { name: m.name });
        }
        match &vecs[m.name] {
            MetricVec::Gauge(gauge) => gauge.with_label_values(&label_values).set(m.value),
            MetricVec::Counter(counter) => {
                if !m.value.is_finite() || m.value < 0.0 {
                    return Err(MetricsError::InvalidCounterValue {
                        name: m.name,
                        value: m.value,
                    });
                }
                counter.with_label_values(&label_values).inc_by(m.value);
            }
        }
    }
    for (_, metric_vec) in vecs {
        match metric_vec {
            MetricVec::Gauge(gauge) => registry.register(Box::new(gauge))?,
            MetricVec::Counter(counter) => registry.register(Box::new(counter))?,
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
/// 契约：本函数只追加写入、不负责清空——调用方复用缓冲前必须自行 `clear()`。
pub fn encode_bytes_into(registry: &prometheus::Registry, out: &mut Vec<u8>) {
    encode_metric_families_into(&registry.gather(), out);
}

/// 编码已经合并并排序的指标族。Snapshot 用它把多个 BMC registry 合成为一份
/// 合法 exposition，确保每个指标名只出现一组 HELP/TYPE 元数据。
pub fn encode_metric_families_into(
    families: &[prometheus::proto::MetricFamily],
    out: &mut Vec<u8>,
) {
    use prometheus::{Encoder, TextEncoder};
    // 不变量：encode 写入 Vec<u8> 不会失败；失败即程序缺陷，panic 合理。
    TextEncoder::new()
        .encode(families, out)
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
