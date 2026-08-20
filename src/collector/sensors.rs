use crate::collector::status_labels;
use crate::metrics::{Metric, SENSOR_READING, unbox_reading};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::schema::sensor::Sensor;
use std::sync::Arc;

const THRESHOLD_UPPER_CRITICAL: (&str, &str) = (
    "redfish_sensor_threshold_upper_critical",
    "Sensor threshold",
);
const THRESHOLD_UPPER_WARNING: (&str, &str) =
    ("redfish_sensor_threshold_upper_warning", "Sensor threshold");
const THRESHOLD_LOWER_WARNING: (&str, &str) =
    ("redfish_sensor_threshold_lower_warning", "Sensor threshold");
const THRESHOLD_LOWER_CRITICAL: (&str, &str) = (
    "redfish_sensor_threshold_lower_critical",
    "Sensor threshold",
);

pub async fn collect_chassis_sensors<B: Bmc>(
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
        let Some(links) = chassis
            .sensor_links()
            .await
            .map_err(|e| format!("sensor links: {e}"))?
        else {
            continue;
        };
        let total = links.len();
        let mut failed = 0usize;
        for link in links {
            let sensor = match link.fetch().await {
                Ok(sensor) => sensor,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };
            let labels = collect_labels(&sensor);
            let Some(reading) = unbox_reading(sensor.reading) else {
                continue;
            };
            out.push(
                Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
                    .label("bmc", bmc_name.to_string())
                    .label("chassis", chassis_id.clone())
                    .label("name", labels.name.clone())
                    .label("units", labels.units.clone())
                    .label("sensor_type", labels.sensor_type.clone())
                    .label("health", labels.health.clone())
                    .label("state", labels.state.clone())
                    .build(reading),
            );
            push_threshold(
                &mut out,
                bmc_name,
                &chassis_id,
                &sensor,
                &labels,
                THRESHOLD_UPPER_CRITICAL,
                "upper_critical",
            );
            push_threshold(
                &mut out,
                bmc_name,
                &chassis_id,
                &sensor,
                &labels,
                THRESHOLD_UPPER_WARNING,
                "upper_warning",
            );
            push_threshold(
                &mut out,
                bmc_name,
                &chassis_id,
                &sensor,
                &labels,
                THRESHOLD_LOWER_WARNING,
                "lower_warning",
            );
            push_threshold(
                &mut out,
                bmc_name,
                &chassis_id,
                &sensor,
                &labels,
                THRESHOLD_LOWER_CRITICAL,
                "lower_critical",
            );
        }
        if failed > 0 && failed == total {
            return Err(format!(
                "sensor fetch: all {failed}/{total} sensor fetches failed"
            ));
        }
    }
    Ok(out)
}

struct SensorLabels {
    name: String,
    units: String,
    sensor_type: String,
    health: String,
    state: String,
}

fn collect_labels(sensor: &Sensor) -> SensorLabels {
    let (health, state) = status_labels(sensor.status.as_ref());
    SensorLabels {
        name: sensor.base.id.to_string(),
        units: sensor.reading_units.clone().flatten().unwrap_or_default(),
        sensor_type: sensor
            .reading_type
            .as_ref()
            .and_then(|t| t.as_ref())
            .map(|t| format!("{t:?}"))
            .unwrap_or_default(),
        health,
        state,
    }
}

fn push_threshold(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    chassis_id: &str,
    sensor: &Sensor,
    labels: &SensorLabels,
    name_help: (&'static str, &'static str),
    kind: &str,
) {
    let Some(value) = threshold_value(sensor, kind) else {
        return;
    };
    out.push(
        Metric::gauge(name_help.0, name_help.1)
            .label("bmc", bmc_name.to_string())
            .label("chassis", chassis_id.to_string())
            .label("name", labels.name.clone())
            .label("units", labels.units.clone())
            .label("sensor_type", labels.sensor_type.clone())
            .label("health", labels.health.clone())
            .label("state", labels.state.clone())
            .build(value),
    );
}

fn threshold_value(sensor: &Sensor, kind: &str) -> Option<f64> {
    let thresholds = sensor.thresholds.as_ref()?;
    let threshold = match kind {
        "upper_critical" => thresholds.upper_critical.as_ref(),
        "upper_warning" => thresholds.upper_caution.as_ref(),
        "lower_warning" => thresholds.lower_caution.as_ref(),
        "lower_critical" => thresholds.lower_critical.as_ref(),
        _ => return None,
    };
    threshold.and_then(|t| unbox_reading(t.reading))
}
