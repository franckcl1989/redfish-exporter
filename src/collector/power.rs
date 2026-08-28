use crate::collector::status_labels;
use crate::metrics::{
    Metric, POWER_CONSUMPTION, POWER_CONSUMPTION_AVG, POWER_CONSUMPTION_INTERVAL,
    POWER_CONSUMPTION_MAX, POWER_CONSUMPTION_MIN, POWER_SUPPLY_CAPACITY, POWER_SUPPLY_EFFICIENCY,
    POWER_SUPPLY_INPUT_VOLTAGE, POWER_SUPPLY_INPUT_WATTS, SENSOR_READING, unbox_reading,
};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::chassis::Chassis;
use nv_redfish::core::EntityTypeRef as _;
use nv_redfish::schema::sensor::Sensor;
use nv_redfish::schema::thermal::Temperature;
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

pub async fn collect_power_metrics<B: Bmc>(
    bmc: Arc<B>,
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
        collect_legacy_thermal(bmc.as_ref(), bmc_name, &chassis_id, &chassis, &mut out).await?;
        collect_legacy_power(bmc.as_ref(), bmc_name, &chassis_id, &chassis, &mut out).await?;
        collect_power_supplies(bmc_name, &chassis_id, &chassis, &mut out).await?;
        collect_environment_metrics(bmc_name, &chassis_id, &chassis, &mut out).await?;
        collect_controls(bmc_name, &chassis_id, &chassis, &mut out).await?;
    }
    Ok(out)
}

async fn collect_legacy_thermal<B: Bmc>(
    bmc: &B,
    bmc_name: &str,
    chassis_id: &str,
    chassis: &Chassis<B>,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(thermal) = chassis
        .thermal()
        .await
        .map_err(|e| format!("thermal: {e}"))?
    else {
        return Ok(());
    };
    let Some(temperatures) = &thermal.raw().temperatures else {
        return Ok(());
    };
    let total = temperatures.len();
    let mut failed = 0usize;
    for nav in temperatures {
        let temperature = match nav.get(bmc).await {
            Ok(temperature) => temperature,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "legacy temperature fetch failed"
                );
                continue;
            }
        };
        let Some(reading) = unbox_reading(temperature.reading_celsius) else {
            continue;
        };
        let labels = temperature_labels(&temperature);
        push_sensor_reading(out, bmc_name, chassis_id, &labels, reading);
        push_threshold(
            out,
            bmc_name,
            chassis_id,
            &labels,
            THRESHOLD_UPPER_CRITICAL,
            unbox_reading(temperature.upper_threshold_critical),
        );
        push_threshold(
            out,
            bmc_name,
            chassis_id,
            &labels,
            THRESHOLD_UPPER_WARNING,
            unbox_reading(temperature.upper_threshold_non_critical),
        );
        push_threshold(
            out,
            bmc_name,
            chassis_id,
            &labels,
            THRESHOLD_LOWER_WARNING,
            unbox_reading(temperature.lower_threshold_non_critical),
        );
        push_threshold(
            out,
            bmc_name,
            chassis_id,
            &labels,
            THRESHOLD_LOWER_CRITICAL,
            unbox_reading(temperature.lower_threshold_critical),
        );
    }
    if total > 0 && failed == total {
        return Err(format!(
            "thermal: all {failed}/{total} temperature fetches failed"
        ));
    }
    Ok(())
}

async fn collect_legacy_power<B: Bmc>(
    bmc: &B,
    bmc_name: &str,
    chassis_id: &str,
    chassis: &Chassis<B>,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(power) = chassis.power().await.map_err(|e| format!("power: {e}"))? else {
        return Ok(());
    };
    let power = power.raw();
    let Some(controls) = &power.power_control else {
        return legacy_power_supplies(bmc, bmc_name, chassis_id, &power, out).await;
    };
    let Some(first) = controls.first() else {
        return legacy_power_supplies(bmc, bmc_name, chassis_id, &power, out).await;
    };
    let power_control = match first.get(bmc).await {
        Ok(power_control) => power_control,
        Err(error) => {
            tracing::warn!(
                bmc = %bmc_name,
                chassis = %chassis_id,
                error = %error,
                "legacy power control fetch failed; trying legacy power supplies"
            );
            let has_supplies = power
                .power_supplies
                .as_ref()
                .is_some_and(|supplies| !supplies.is_empty());
            legacy_power_supplies(bmc, bmc_name, chassis_id, &power, out).await?;
            if !has_supplies {
                return Err(format!("power control: {error}"));
            }
            return Ok(());
        }
    };
    let Some(consumed) = unbox_reading(power_control.power_consumed_watts) else {
        return legacy_power_supplies(bmc, bmc_name, chassis_id, &power, out).await;
    };
    out.push(
        Metric::gauge(POWER_CONSUMPTION.0, POWER_CONSUMPTION.1)
            .label("bmc", bmc_name.to_string())
            .label("chassis", chassis_id.to_string())
            .build(consumed),
    );
    if let Some(pm) = &power_control.power_metrics {
        push_power_stat(
            out,
            bmc_name,
            chassis_id,
            POWER_CONSUMPTION_MIN,
            unbox_reading(pm.min_consumed_watts),
        );
        push_power_stat(
            out,
            bmc_name,
            chassis_id,
            POWER_CONSUMPTION_MAX,
            unbox_reading(pm.max_consumed_watts),
        );
        push_power_stat(
            out,
            bmc_name,
            chassis_id,
            POWER_CONSUMPTION_AVG,
            unbox_reading(pm.average_consumed_watts),
        );
        push_power_stat(
            out,
            bmc_name,
            chassis_id,
            POWER_CONSUMPTION_INTERVAL,
            pm.interval_in_min.flatten().map(|v| v as f64),
        );
    }
    legacy_power_supplies(bmc, bmc_name, chassis_id, &power, out).await
}

async fn legacy_power_supplies<B: Bmc>(
    bmc: &B,
    bmc_name: &str,
    chassis_id: &str,
    power: &nv_redfish::schema::power::Power,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(supplies) = &power.power_supplies else {
        return Ok(());
    };
    let total = supplies.len();
    let mut failed = 0usize;
    for nav in supplies {
        let supply = match nav.get(bmc).await {
            Ok(supply) => supply,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "legacy power supply fetch failed"
                );
                continue;
            }
        };
        let id = supply.base.member_id.clone();
        push_psu_metric(
            out,
            bmc_name,
            chassis_id,
            &id,
            POWER_SUPPLY_EFFICIENCY,
            unbox_reading(supply.efficiency_percent),
        );
        push_psu_metric(
            out,
            bmc_name,
            chassis_id,
            &id,
            POWER_SUPPLY_INPUT_WATTS,
            unbox_reading(supply.power_input_watts),
        );
        push_psu_metric(
            out,
            bmc_name,
            chassis_id,
            &id,
            POWER_SUPPLY_CAPACITY,
            unbox_reading(supply.power_capacity_watts),
        );
        push_psu_metric(
            out,
            bmc_name,
            chassis_id,
            &id,
            POWER_SUPPLY_INPUT_VOLTAGE,
            unbox_reading(supply.line_input_voltage),
        );
        let Some(reading) = unbox_reading(supply.power_output_watts) else {
            continue;
        };
        let (health, state) = status_labels(supply.status.as_ref());
        let labels = ReadingLabels {
            id: supply.base.odata_id().to_string(),
            name: supply
                .name
                .clone()
                .flatten()
                .unwrap_or_else(|| supply.base.member_id.clone()),
            units: "W".to_string(),
            sensor_type: "Power".to_string(),
            health,
            state,
        };
        push_sensor_reading(out, bmc_name, chassis_id, &labels, reading);
    }
    if total > 0 && failed == total {
        return Err(format!(
            "power supplies: all {failed}/{total} legacy supply fetches failed"
        ));
    }
    Ok(())
}

async fn collect_power_supplies<B: Bmc>(
    bmc_name: &str,
    chassis_id: &str,
    chassis: &Chassis<B>,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let supplies = chassis
        .power_supplies()
        .await
        .map_err(|e| format!("power supplies: {e}"))?;
    let mut attempted = 0usize;
    let mut supply_failures = 0usize;
    for supply in supplies {
        if supply.raw().metrics.is_none() {
            continue;
        };
        attempted += 1;
        let links = match supply.metrics_sensor_links().await {
            Ok(links) => links,
            Err(error) => {
                supply_failures += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    supply = %supply.id(),
                    error = %error,
                    "power supply metrics fetch failed"
                );
                continue;
            }
        };
        let total = links.len();
        let mut failed = 0usize;
        for link in links {
            let sensor = match link.fetch().await {
                Ok(sensor) => sensor,
                Err(error) => {
                    failed += 1;
                    tracing::warn!(
                        bmc = %bmc_name,
                        chassis = %chassis_id,
                        supply = %supply.id(),
                        error = %error,
                        "power supply metric sensor fetch failed"
                    );
                    continue;
                }
            };
            push_sensor_metric(out, bmc_name, chassis_id, &sensor);
        }
        if total > 0 && failed == total {
            supply_failures += 1;
            tracing::warn!(
                bmc = %bmc_name,
                chassis = %chassis_id,
                supply = %supply.id(),
                failed,
                total,
                "all power supply metric sensors failed"
            );
        }
    }
    if attempted > 0 && supply_failures == attempted {
        return Err(format!(
            "power supplies: all {supply_failures}/{attempted} declared metrics resources failed"
        ));
    }
    Ok(())
}

async fn collect_environment_metrics<B: Bmc>(
    bmc_name: &str,
    chassis_id: &str,
    chassis: &Chassis<B>,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(metrics) = chassis
        .environment_metrics()
        .await
        .map_err(|e| format!("environment metrics: {e}"))?
    else {
        return Ok(());
    };
    let links = metrics.sensor_links();
    let total = links.len();
    let mut failed = 0usize;
    for link in links {
        let sensor = match link.fetch().await {
            Ok(sensor) => sensor,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    chassis = %chassis_id,
                    error = %error,
                    "environment metric sensor fetch failed"
                );
                continue;
            }
        };
        push_sensor_metric(out, bmc_name, chassis_id, &sensor);
    }
    if total > 0 && failed == total {
        return Err(format!(
            "environment metrics: all {failed}/{total} sensor fetches failed"
        ));
    }
    Ok(())
}

async fn collect_controls<B: Bmc>(
    bmc_name: &str,
    chassis_id: &str,
    chassis: &Chassis<B>,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let Some(controls) = chassis
        .controls()
        .await
        .map_err(|e| format!("controls: {e}"))?
    else {
        return Ok(());
    };
    for control in controls {
        let control = control.raw();
        let Some(reading) = unbox_reading(control.set_point) else {
            continue;
        };
        let (health, state) = status_labels(control.status.as_ref());
        let labels = ReadingLabels {
            id: control.base.odata_id().to_string(),
            name: control.base.id.to_string(),
            units: control
                .set_point_units
                .clone()
                .flatten()
                .unwrap_or_default(),
            sensor_type: "Control".to_string(),
            health,
            state,
        };
        push_sensor_reading(out, bmc_name, chassis_id, &labels, reading);
    }
    Ok(())
}

struct ReadingLabels {
    id: String,
    name: String,
    units: String,
    sensor_type: String,
    health: String,
    state: String,
}

fn temperature_labels(temperature: &Temperature) -> ReadingLabels {
    let (health, state) = status_labels(temperature.status.as_ref());
    ReadingLabels {
        id: temperature.base.odata_id().to_string(),
        name: temperature
            .name
            .clone()
            .flatten()
            .unwrap_or_else(|| temperature.base.member_id.clone()),
        units: "Cel".to_string(),
        sensor_type: "Temperature".to_string(),
        health,
        state,
    }
}

fn push_sensor_metric(out: &mut Vec<Metric>, bmc_name: &str, chassis_id: &str, sensor: &Sensor) {
    let labels = sensor_labels(sensor);
    let Some(reading) = unbox_reading(sensor.reading) else {
        return;
    };
    push_sensor_reading(out, bmc_name, chassis_id, &labels, reading);
}

fn sensor_labels(sensor: &Sensor) -> ReadingLabels {
    let (health, state) = status_labels(sensor.status.as_ref());
    ReadingLabels {
        id: sensor.base.odata_id().to_string(),
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

fn push_sensor_reading(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    chassis_id: &str,
    labels: &ReadingLabels,
    value: f64,
) {
    out.push(
        Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
            .label("bmc", bmc_name.to_string())
            .label("chassis", chassis_id.to_string())
            .label("id", labels.id.clone())
            .label("name", labels.name.clone())
            .label("units", labels.units.clone())
            .label("sensor_type", labels.sensor_type.clone())
            .label("health", labels.health.clone())
            .label("state", labels.state.clone())
            .build(value),
    );
}

fn push_threshold(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    chassis_id: &str,
    labels: &ReadingLabels,
    name_help: (&'static str, &'static str),
    value: Option<f64>,
) {
    let Some(value) = value else {
        return;
    };
    out.push(
        Metric::gauge(name_help.0, name_help.1)
            .label("bmc", bmc_name.to_string())
            .label("chassis", chassis_id.to_string())
            .label("id", labels.id.clone())
            .label("name", labels.name.clone())
            .label("units", labels.units.clone())
            .label("sensor_type", labels.sensor_type.clone())
            .label("health", labels.health.clone())
            .label("state", labels.state.clone())
            .build(value),
    );
}

fn push_power_stat(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    chassis_id: &str,
    name_help: (&'static str, &'static str),
    value: Option<f64>,
) {
    let Some(value) = value else {
        return;
    };
    out.push(
        Metric::gauge(name_help.0, name_help.1)
            .label("bmc", bmc_name.to_string())
            .label("chassis", chassis_id.to_string())
            .build(value),
    );
}

fn push_psu_metric(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    chassis_id: &str,
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
            .label("chassis", chassis_id.to_string())
            .label("id", id.to_string())
            .build(value),
    );
}
