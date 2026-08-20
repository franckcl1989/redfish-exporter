pub mod memory;
pub mod network;
pub mod power;
pub mod processors;
pub mod sensors;
pub mod storage;
pub mod systems;

use crate::metrics::{HEALTH_STATUS, INFO, Metric, SCRAPE_DURATION, UP, health_state_labels};
use nv_redfish::Bmc;
use nv_redfish::schema::resource::{Health, Status};
use std::sync::Arc;

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

pub(crate) fn push_info(out: &mut Vec<Metric>, bmc: &str, key: &str, value: &str) {
    out.push(
        Metric::gauge(INFO.0, INFO.1)
            .label("bmc", bmc.to_string())
            .label("key", key.to_string())
            .label("value", value.to_string())
            .build(1.0),
    );
}

pub struct ScrapeReport {
    pub metrics: Vec<Metric>,
    pub failed_resources: Vec<String>,
}

pub async fn collect_all<B: Bmc>(
    bmc: Arc<B>,
    bmc_name: &str,
) -> Result<ScrapeReport, nv_redfish::Error<B>> {
    let started = std::time::Instant::now();
    let root = nv_redfish::ServiceRoot::new(Arc::clone(&bmc)).await?;
    let mut metrics = Vec::new();
    let mut failed_resources = Vec::new();

    match sensors::collect_chassis_sensors(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match power::collect_power_metrics(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match processors::collect_processors(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match memory::collect_memory(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match storage::collect_storage(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match network::collect_network(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match systems::collect_systems(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match systems::collect_chassis_health(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match systems::collect_managers(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match systems::collect_assembly(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    match systems::collect_firmware(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }

    let up = if failed_resources.is_empty() {
        1.0
    } else {
        0.0
    };
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
    Ok(ScrapeReport {
        metrics,
        failed_resources,
    })
}
