pub mod memory;
pub mod power;
pub mod processors;
pub mod sensors;

use crate::metrics::{Metric, SCRAPE_DURATION, UP};
use nv_redfish::Bmc;
use std::sync::Arc;

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
    // Task 7-9 在此挂接其余 collector

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
