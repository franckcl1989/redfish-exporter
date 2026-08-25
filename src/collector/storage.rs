use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{
    DRIVE_CAPACITY, Metric, STORAGE_CONTROLLER_INFO, STORAGE_CONTROLLER_STATUS, VOLUME_CAPACITY,
};
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::core::{EntityTypeRef, ODataETag, ODataId};
use nv_redfish::schema::volume::Volume as VolumeSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// 存储资源原始负载：nv-redfish 0.15 的类型化 StorageSchema 将标准
/// StorageControllers 数组仅编译为 Vec<ReferenceLeaf>（只保留 @odata.id，
/// 内联 Model/FirmwareVersion/Status 被反序列化丢弃），故按 pagination.rs
/// 的 Page 先例自定义 EntityTypeRef，重新 GET 同一资源 URI 从原始 JSON
/// 提取控制器明细（路径以真机探测 Dell RAID.Slot.2-1 为准）。
#[derive(Deserialize)]
struct RawStoragePayload {
    #[serde(rename = "@odata.id")]
    id: ODataId,
    #[serde(rename = "StorageControllers", default)]
    storage_controllers: Option<Vec<Value>>,
}

impl EntityTypeRef for RawStoragePayload {
    fn odata_id(&self) -> &ODataId {
        &self.id
    }

    fn etag(&self) -> Option<&ODataETag> {
        None
    }
}

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

    // 存储控制器明细（标准 StorageControllers 数组；真机仅 Dell 有，
    // 浪潮存储子树固件损坏不产出）：类型化 schema 无该字段（仅 ReferenceLeaf），
    // 经 raw JSON 重取同一 URI 导航。重取失败仅跳过控制器指标，
    // 不阻止 drives/volumes 采集。
    if raw
        .storage_controllers
        .as_ref()
        .is_some_and(|v| !v.is_empty())
        && let Ok(raw_payload) = bmc.get::<RawStoragePayload>(storage.odata_id()).await
    {
        collect_storage_controllers(bmc_name, system_id, &storage_id, &raw_payload, out);
    }

    // drives 获取失败时跳过该子资源，不阻止 volumes 采集
    if let Ok(Some(drives)) = storage.drives().await {
        for drive in drives {
            collect_drive(bmc, bmc_name, system_id, &storage_id, &drive, out).await;
        }
    }

    let Some(volumes_nav) = &raw.volumes else {
        return;
    };
    if let Ok(collection) = volumes_nav.get(bmc).await {
        for volume_ref in &collection.members {
            let Ok(volume) = volume_ref.get(bmc).await else {
                continue;
            };
            collect_volume(bmc_name, system_id, &storage_id, &volume, out);
        }
    }
}

fn collect_storage_controllers(
    bmc_name: &str,
    system_id: &str,
    storage_id: &str,
    raw: &RawStoragePayload,
    out: &mut Vec<Metric>,
) {
    let Some(controllers) = &raw.storage_controllers else {
        return;
    };
    for (i, c) in controllers.iter().enumerate() {
        // 成员标识：MemberId 优先（真机 Dell 用 MemberId），缺省用 Id，再缺省用下标。
        let id = c
            .get("MemberId")
            .and_then(Value::as_str)
            .or_else(|| c.get("Id").and_then(Value::as_str))
            .map(str::to_owned)
            .unwrap_or_else(|| i.to_string());
        let model = c.get("Model").and_then(Value::as_str).map(str::to_owned);
        let fw = c.get("FirmwareVersion").and_then(Value::as_str);
        if let Some(m) = model {
            out.push(
                Metric::gauge(STORAGE_CONTROLLER_INFO.0, STORAGE_CONTROLLER_INFO.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.to_string())
                    .label("storage", storage_id.to_string())
                    .label("id", id.clone())
                    .label("model", m)
                    .label("firmware_version", fw.unwrap_or_default().to_string())
                    .build(1.0),
            );
        }
        if let Some(st) = c
            .get("Status")
            .and_then(|s| s.get("State"))
            .and_then(Value::as_str)
        {
            out.push(
                Metric::gauge(STORAGE_CONTROLLER_STATUS.0, STORAGE_CONTROLLER_STATUS.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.to_string())
                    .label("storage", storage_id.to_string())
                    .label("id", id)
                    .label("status", st.to_string())
                    .build(1.0),
            );
        }
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
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "drive", &id, &health, &state);
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
    let (health, state) = status_labels(volume.status.as_ref());
    push_health(out, bmc_name, "volume", &id, &health, &state);
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
