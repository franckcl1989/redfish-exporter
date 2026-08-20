use crate::metrics::{
    DRIVE_CAPACITY, HEALTH_STATUS, INFO, Metric, VOLUME_CAPACITY, health_state_labels,
};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::schema::resource::{Health, Status};
use nv_redfish::schema::volume::Volume as VolumeSchema;
use std::sync::Arc;

const DRIVE_LIFE_LEFT: (&str, &str) = (
    "redfish_drive_life_left_percent",
    "Drive remaining media life percentage",
);
const DRIVE_PREDICTIVE_FAILURE: (&str, &str) = (
    "redfish_drive_predictive_failure",
    "Whether the drive predicts a failure in the near future, 1 = failure predicted",
);
const DRIVE_IO_READ_CORRECTABLE: (&str, &str) = (
    "redfish_drive_io_read_correctable_errors_total",
    "Lifetime number of correctable read errors reported by the drive",
);
const DRIVE_IO_WRITE_CORRECTABLE: (&str, &str) = (
    "redfish_drive_io_write_correctable_errors_total",
    "Lifetime number of correctable write errors reported by the drive",
);
const DRIVE_IO_READ_UNCORRECTABLE: (&str, &str) = (
    "redfish_drive_io_read_uncorrectable_errors_total",
    "Lifetime number of uncorrectable read errors reported by the drive",
);
const DRIVE_IO_WRITE_UNCORRECTABLE: (&str, &str) = (
    "redfish_drive_io_write_uncorrectable_errors_total",
    "Lifetime number of uncorrectable write errors reported by the drive",
);

pub async fn collect_storage<B: Bmc>(
    bmc: Arc<B>,
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
        let Ok(Some(storages)) = system.storage_controllers().await else {
            continue;
        };
        for storage in storages {
            collect_storage_controller(bmc.as_ref(), bmc_name, &system_id, &storage, &mut out)
                .await;
        }
    }
    Ok(out)
}

async fn collect_storage_controller<B: Bmc>(
    bmc: &B,
    bmc_name: &str,
    system_id: &str,
    storage: &nv_redfish::computer_system::Storage<B>,
    out: &mut Vec<Metric>,
) {
    let raw = storage.raw();
    let storage_id = storage.id().to_string();

    let Ok(Some(drives)) = storage.drives().await else {
        return;
    };
    for drive in drives {
        collect_drive(bmc, bmc_name, system_id, &storage_id, &drive, out).await;
    }

    let Some(volumes_nav) = &raw.volumes else {
        return;
    };
    let Ok(collection) = volumes_nav.get(bmc).await else {
        return;
    };
    for volume_ref in &collection.members {
        let Ok(volume) = volume_ref.get(bmc).await else {
            continue;
        };
        collect_volume(bmc_name, system_id, &storage_id, &volume, out);
    }
}

async fn collect_drive<B: Bmc>(
    _bmc: &B,
    bmc_name: &str,
    system_id: &str,
    storage_id: &str,
    drive: &nv_redfish::computer_system::Drive<B>,
    out: &mut Vec<Metric>,
) {
    let raw = drive.raw();
    let id = drive.id().to_string();
    push_health(out, bmc_name, "drive", &id, raw.status.as_ref());
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(out, bmc_name, "manufacturer", &value);
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(out, bmc_name, "model", &value);
    }
    if let Some(value) = raw.serial_number.clone().flatten() {
        push_info(out, bmc_name, "serial_number", &value);
    }
    if let Some(value) = raw.revision.clone().flatten() {
        push_info(out, bmc_name, "revision", &value);
    }
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        DRIVE_CAPACITY,
        raw.capacity_bytes.flatten().map(|v| v as f64),
    );
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        DRIVE_LIFE_LEFT,
        raw.predicted_media_life_left_percent.flatten(),
    );
    if let Some(failure) = raw.failure_predicted.flatten() {
        push_value(
            out,
            bmc_name,
            system_id,
            storage_id,
            &id,
            DRIVE_PREDICTIVE_FAILURE,
            Some(if failure { 1.0 } else { 0.0 }),
        );
    }
    let Ok(Some(metrics)) = drive.metrics().await else {
        return;
    };
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        DRIVE_IO_READ_CORRECTABLE,
        metrics
            .correctable_io_read_error_count
            .flatten()
            .map(|v| v as f64),
    );
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        DRIVE_IO_WRITE_CORRECTABLE,
        metrics
            .correctable_io_write_error_count
            .flatten()
            .map(|v| v as f64),
    );
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        DRIVE_IO_READ_UNCORRECTABLE,
        metrics
            .uncorrectable_io_read_error_count
            .flatten()
            .map(|v| v as f64),
    );
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        DRIVE_IO_WRITE_UNCORRECTABLE,
        metrics
            .uncorrectable_io_write_error_count
            .flatten()
            .map(|v| v as f64),
    );
}

fn collect_volume(
    bmc_name: &str,
    system_id: &str,
    storage_id: &str,
    volume: &VolumeSchema,
    out: &mut Vec<Metric>,
) {
    let id = volume.base.id.clone();
    push_health(out, bmc_name, "volume", &id, volume.status.as_ref());
    push_info(out, bmc_name, "name", &volume.base.name.clone());
    push_value(
        out,
        bmc_name,
        system_id,
        storage_id,
        &id,
        VOLUME_CAPACITY,
        volume.capacity_bytes.flatten().map(|v| v as f64),
    );
}

fn push_value(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    system_id: &str,
    storage_id: &str,
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
            .label("storage", storage_id.to_string())
            .label("id", id.to_string())
            .build(value),
    );
}

fn push_health(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    resource_type: &str,
    id: &str,
    status: Option<&Status>,
) {
    let (health, state) = status_labels(status);
    out.push(
        Metric::gauge(HEALTH_STATUS.0, HEALTH_STATUS.1)
            .label("bmc", bmc_name.to_string())
            .label("resource_type", resource_type.to_string())
            .label("id", id.to_string())
            .label("health", health)
            .label("state", state)
            .build(1.0),
    );
}

fn push_info(out: &mut Vec<Metric>, bmc_name: &str, key: &str, value: &str) {
    out.push(
        Metric::gauge(INFO.0, INFO.1)
            .label("bmc", bmc_name.to_string())
            .label("key", key.to_string())
            .label("value", value.to_string())
            .build(1.0),
    );
}

fn status_labels(status: Option<&Status>) -> (String, String) {
    let health = status
        .and_then(|s| s.health.as_ref())
        .and_then(|h| h.as_ref())
        .map(health_str);
    let state = status
        .and_then(|s| s.state.as_ref())
        .and_then(|h| h.as_ref())
        .map(|s| format!("{s:?}"));
    health_state_labels(health, state.as_deref())
}

fn health_str(health: &Health) -> &'static str {
    match health {
        Health::Ok => "OK",
        Health::Warning => "Warning",
        Health::Critical => "Critical",
        Health::UnsupportedValue => "UnsupportedValue",
    }
}
