pub mod bios;
pub mod error;
pub mod logs;
pub mod memory;
pub mod network;
pub mod power;
pub mod processors;
pub mod sensors;
pub mod storage;
pub mod systems;

use crate::config::CollectorsConfig;
use crate::metrics::{HEALTH_STATUS, INFO, Metric, SCRAPE_DURATION, UP, health_state_labels};
use nv_redfish::schema::resource::{Health, Status};
use nv_redfish::{Bmc, ServiceRoot};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

pub(crate) fn health_wire(health: &Health) -> &'static str {
    match health {
        Health::Ok => "OK",
        Health::Warning => "Warning",
        Health::Critical => "Critical",
        Health::UnsupportedValue => "UnsupportedValue",
    }
}

pub(crate) fn status_labels(status: Option<&Status>) -> (String, String) {
    let health = status
        .and_then(|s| s.health.as_ref())
        .and_then(|h| h.as_ref())
        .map(health_wire);
    let state = status
        .and_then(|s| s.state.as_ref())
        .and_then(|h| h.as_ref())
        .map(|s| format!("{s:?}"));
    health_state_labels(health, state.as_deref())
}

pub(crate) fn push_health(
    out: &mut Vec<Metric>,
    bmc: &str,
    resource_type: &str,
    id: &str,
    health: &str,
    state: &str,
) {
    let health = if health.is_empty() { "unknown" } else { health };
    let state = if state.is_empty() { "unknown" } else { state };
    out.push(
        Metric::gauge(HEALTH_STATUS.0, HEALTH_STATUS.1)
            .label("bmc", bmc.to_string())
            .label("resource_type", resource_type.to_string())
            .label("id", id.to_string())
            .label("health", health.to_string())
            .label("state", state.to_string())
            .build(1.0),
    );
}

pub(crate) fn push_info(
    out: &mut Vec<Metric>,
    bmc: &str,
    resource_type: &str,
    id: &str,
    key: &str,
    value: &str,
) {
    out.push(
        Metric::gauge(INFO.0, INFO.1)
            .label("bmc", bmc.to_string())
            .label("resource_type", resource_type.to_string())
            .label("id", id.to_string())
            .label("key", key.to_string())
            .label("value", value.to_string())
            .build(1.0),
    );
}

#[derive(Clone, Debug)]
pub struct ScrapeReport {
    pub metrics: Vec<Metric>,
    pub failed_resources: Vec<String>,
}

/// 快组：每轮采集。返回 report 不含 up/duration（finalize_report 统一添加）。
pub async fn collect_fast<B: Bmc>(
    bmc: Arc<B>,
    root: &ServiceRoot<B>,
    bmc_name: &str,
) -> Result<ScrapeReport, String> {
    let mut metrics = Vec::new();
    let mut failed_resources = Vec::new();

    match timed(
        "sensors",
        bmc_name,
        sensors::collect_chassis_sensors(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "power",
        bmc_name,
        power::collect_power_metrics(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "processors",
        bmc_name,
        processors::collect_processors(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "memory",
        bmc_name,
        memory::collect_memory(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "systems",
        bmc_name,
        systems::collect_systems(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "chassis_health",
        bmc_name,
        systems::collect_chassis_health(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "managers",
        bmc_name,
        systems::collect_managers(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }

    Ok(ScrapeReport {
        metrics,
        failed_resources,
    })
}

/// 慢组：按 slow_interval 分频采集。返回 report 不含 up/duration。
pub async fn collect_slow<B: Bmc>(
    bmc: Arc<B>,
    root: &ServiceRoot<B>,
    bmc_name: &str,
    collectors: CollectorsConfig,
) -> Result<ScrapeReport, String> {
    let mut metrics = Vec::new();
    let mut failed_resources = Vec::new();

    match timed(
        "storage",
        bmc_name,
        storage::collect_storage(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "network",
        bmc_name,
        network::collect_network(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "firmware",
        bmc_name,
        systems::collect_firmware(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match timed(
        "assembly",
        bmc_name,
        systems::collect_assembly(Arc::clone(&bmc), root, bmc_name),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    if collectors.event_logs {
        match timed(
            "event_logs",
            bmc_name,
            logs::collect_event_logs_with_limit(
                Arc::clone(&bmc),
                root,
                bmc_name,
                collectors.event_log_limit,
            ),
        )
        .await
        {
            Ok(m) => metrics.extend(m),
            Err(resource) => failed_resources.push(resource),
        }
    }
    match timed(
        "bios",
        bmc_name,
        bios::collect_bios_configured(
            Arc::clone(&bmc),
            root,
            bmc_name,
            collectors.bios_attributes,
            collectors.bios_attribute_limit,
        ),
    )
    .await
    {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }

    Ok(ScrapeReport {
        metrics,
        failed_resources,
    })
}

/// 记录单个 collector 的耗时（debug 级），用于定位慢资源。
async fn timed<F, T>(label: &'static str, bmc_name: &str, fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    let started = std::time::Instant::now();
    let result = fut.await;
    tracing::debug!(
        bmc = bmc_name,
        collector = label,
        duration_ms = started.elapsed().as_millis(),
        "collector done"
    );
    result
}

pub fn finalize_report(
    bmc_name: &str,
    metrics: Vec<Metric>,
    failed: Vec<String>,
    started: Instant,
) -> ScrapeReport {
    // Redfish allows the same Sensor resource to be reachable through more than one
    // navigation path (for example Chassis/Sensors and EnvironmentMetrics). Keep one
    // sample for an exact descriptor+label identity. Descriptor/type conflicts and
    // duplicate label keys are deliberately left intact for register_into to reject.
    let mut metrics = deduplicate_exact_series(metrics);
    let up = if failed.is_empty() { 1.0 } else { 0.0 };
    metrics.push(
        Metric::gauge(UP.0, UP.1)
            .label("bmc", bmc_name.to_string())
            .build(up),
    );
    metrics.push(
        Metric::gauge(SCRAPE_DURATION.0, SCRAPE_DURATION.1)
            .label("bmc", bmc_name.to_string())
            .build(started.elapsed().as_secs_f64()),
    );
    ScrapeReport {
        metrics,
        failed_resources: failed,
    }
}

fn deduplicate_exact_series(metrics: Vec<Metric>) -> Vec<Metric> {
    let mut seen = HashMap::with_capacity(metrics.len());
    let mut unique = Vec::with_capacity(metrics.len());
    for metric in metrics {
        let mut labels = metric.labels.clone();
        labels.sort_unstable();
        let key = (metric.name, metric.help, metric.kind, labels);
        match seen.entry(key) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(metric.value);
                unique.push(metric);
            }
            std::collections::hash_map::Entry::Occupied(entry) => {
                if entry.get().to_bits() != metric.value.to_bits() {
                    tracing::warn!(
                        metric = metric.name,
                        first_value = *entry.get(),
                        duplicate_value = metric.value,
                        "duplicate Redfish series observed through multiple navigation paths; keeping first value"
                    );
                }
            }
        }
    }
    unique
}

pub fn merge_reports(fast: ScrapeReport, slow: Option<&ScrapeReport>) -> ScrapeReport {
    let mut metrics = fast.metrics;
    let mut failed = fast.failed_resources;
    if let Some(s) = slow {
        metrics.extend(s.metrics.iter().cloned());
        for r in &s.failed_resources {
            if !failed.contains(r) {
                failed.push(r.clone());
            }
        }
    }
    ScrapeReport {
        metrics,
        failed_resources: failed,
    }
}
