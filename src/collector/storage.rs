use crate::collector::{push_health, push_info, status_labels};
use crate::metrics::{
    DRIVE_CAPACITY, DRIVE_INFO, DRIVE_OEM_STATUS, Metric, STORAGE_CONTROLLER_INFO,
    STORAGE_CONTROLLER_STATUS, VOLUME_CAPACITY,
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
    let mut collection_attempts = 0usize;
    let mut collection_failures = 0usize;
    let mut controller_attempts = 0usize;
    let mut controller_failures = 0usize;
    for system in systems {
        let system_id = system.id().to_string();
        let storages = match system.storage_controllers().await {
            Ok(Some(storages)) => {
                collection_attempts += 1;
                storages
            }
            Ok(None) => continue,
            // IEIT/Inspur NF5280M6 firmware can advertise a synthetic
            // `PCIE*_RAID` member and then return HTTP 500/vendor code 17034
            // for it when no RAID controller is installed. Treat only this
            // exact vendor response as an absent optional collection; other
            // declared collection failures remain observable.
            Err(error) if is_absent_raid_controller_error(&error.to_string()) => {
                tracing::debug!(
                    bmc = %bmc_name,
                    system = %system_id,
                    "firmware advertised an empty RAID controller collection"
                );
                continue;
            }
            Err(error) => {
                collection_attempts += 1;
                collection_failures += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    error = %error,
                    "storage collection fetch failed"
                );
                continue;
            }
        };
        for storage in storages {
            controller_attempts += 1;
            if let Err(error) =
                collect_storage_controller(bmc.as_ref(), bmc_name, &system_id, &storage, &mut out)
                    .await
            {
                controller_failures += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    storage = %storage.id(),
                    error = %error,
                    "storage controller subresources failed"
                );
            }
        }
    }
    if collection_attempts > 0 && collection_failures == collection_attempts {
        return Err(format!(
            "storage: all {collection_failures}/{collection_attempts} declared collections failed"
        ));
    }
    if controller_attempts > 0 && controller_failures == controller_attempts {
        return Err(format!(
            "storage: all {controller_failures}/{controller_attempts} controllers failed"
        ));
    }
    Ok(out)
}

fn is_absent_raid_controller_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("500 internal server error")
        && normalized.contains("17034")
        && normalized.contains("no raid controller available")
}

async fn collect_storage_controller<B: Bmc>(
    bmc: &B,
    bmc_name: &str,
    system_id: &str,
    storage: &nv_redfish::computer_system::Storage<B>,
    out: &mut Vec<Metric>,
) -> Result<(), String> {
    let raw = storage.raw();
    let storage_id = storage.id().to_string();
    let mut attempted = 0usize;
    let mut failed = 0usize;

    // 存储控制器明细（标准 StorageControllers 数组；真机仅 Dell 有，
    // 浪潮存储子树固件损坏不产出）：类型化 schema 无该字段（仅 ReferenceLeaf），
    // 经 raw JSON 重取同一 URI 导航。重取失败仅跳过控制器指标，
    // 不阻止 drives/volumes 采集。
    if raw
        .storage_controllers
        .as_ref()
        .is_some_and(|v| !v.is_empty())
    {
        attempted += 1;
        match bmc.get::<RawStoragePayload>(storage.odata_id()).await {
            Ok(raw_payload) => {
                collect_storage_controllers(bmc_name, system_id, &storage_id, &raw_payload, out);
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    storage = %storage_id,
                    error = %error,
                    "raw storage controller payload fetch failed"
                );
            }
        }
    }

    // A failed drive subtree does not block volumes, but remains observable if
    // every declared subtree of this controller fails.
    match storage.drives().await {
        Ok(Some(drives)) => {
            attempted += 1;
            for drive in drives {
                collect_drive(bmc, bmc_name, system_id, &storage_id, &drive, out).await;
            }
        }
        Ok(None) => {}
        Err(error) => {
            attempted += 1;
            failed += 1;
            tracing::warn!(
                bmc = %bmc_name,
                system = %system_id,
                storage = %storage_id,
                error = %error,
                "drive subtree fetch failed"
            );
        }
    }

    if let Some(volumes_nav) = &raw.volumes {
        attempted += 1;
        match volumes_nav.get(bmc).await {
            Ok(collection) => {
                let total = collection.members.len();
                let mut member_failures = 0usize;
                for volume_ref in &collection.members {
                    match volume_ref.get(bmc).await {
                        Ok(volume) => {
                            collect_volume(bmc_name, system_id, &storage_id, &volume, out);
                        }
                        Err(error) => {
                            member_failures += 1;
                            tracing::warn!(
                                bmc = %bmc_name,
                                system = %system_id,
                                storage = %storage_id,
                                error = %error,
                                "volume fetch failed"
                            );
                        }
                    }
                }
                if total > 0 && member_failures == total {
                    failed += 1;
                }
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    storage = %storage_id,
                    error = %error,
                    "volume collection fetch failed"
                );
            }
        }
    }

    if attempted > 0 && failed == attempted {
        return Err(format!(
            "all {failed}/{attempted} declared storage subresources failed"
        ));
    }
    Ok(())
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
    let resource_id = drive.odata_id().to_string();
    let (health, state) = status_labels(raw.status.as_ref());
    push_health(out, bmc_name, "drive", &resource_id, &health, &state);
    // 驱动器 OEM 字段（Dell DellPhysicalDisk；真机 Dell 有，浪潮驱动器明细
    // 全 null 仅 Status 不产出）：Drive facade 的 raw 保留 Oem 块
    // （Item.oem，与 Processor 同层），经类型化 OEM 直接导航，无需 raw JSON
    // 重取。寿命字段 PredictedMediaLifeLeftPercent 真机为 null（NOT MET
    // backlog），不采集；浪潮无 Oem 块，自然不产出。
    if let Some(oem) = raw.base.base.oem.as_ref()
        && let Some(dell) =
            nv_redfish::oem::oem_value(oem, "Dell").and_then(|d| d.get("DellPhysicalDisk"))
    {
        if let Some(wwn) = dell.get("WWN").and_then(|v| v.as_str()).map(str::to_string) {
            out.push(
                Metric::gauge(DRIVE_INFO.0, DRIVE_INFO.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.to_string())
                    .label("storage", storage_id.to_string())
                    .label("id", id.clone())
                    .label("wwn", wwn)
                    .build(1.0),
            );
        }
        let raid = dell
            .get("RaidStatus")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let power = dell
            .get("PowerStatus")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !raid.is_empty() || !power.is_empty() {
            out.push(
                Metric::gauge(DRIVE_OEM_STATUS.0, DRIVE_OEM_STATUS.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.to_string())
                    .label("storage", storage_id.to_string())
                    .label("id", id.clone())
                    .label("raid_status", raid)
                    .label("power_status", power)
                    .build(1.0),
            );
        }
    }
    if let Some(value) = raw.manufacturer.clone().flatten() {
        push_info(out, bmc_name, "drive", &resource_id, "manufacturer", &value);
    }
    if let Some(value) = raw.model.clone().flatten() {
        push_info(out, bmc_name, "drive", &resource_id, "model", &value);
    }
    if let Some(value) = raw.serial_number.clone().flatten() {
        push_info(
            out,
            bmc_name,
            "drive",
            &resource_id,
            "serial_number",
            &value,
        );
    }
    if let Some(value) = raw.revision.clone().flatten() {
        push_info(out, bmc_name, "drive", &resource_id, "revision", &value);
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
    let metrics = match drive.metrics().await {
        Ok(Some(metrics)) => metrics,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                bmc = %bmc_name,
                system = %system_id,
                storage = %storage_id,
                drive = %id,
                error = %error,
                "drive metrics fetch failed"
            );
            return;
        }
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
    let resource_id = volume.odata_id().to_string();
    let (health, state) = status_labels(volume.status.as_ref());
    push_health(out, bmc_name, "volume", &resource_id, &health, &state);
    push_info(
        out,
        bmc_name,
        "volume",
        &resource_id,
        "name",
        &volume.base.name,
    );
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
        if name_help.0.ends_with("_total") {
            Metric::counter(name_help.0, name_help.1)
        } else {
            Metric::gauge(name_help.0, name_help.1)
        }
        .label("bmc", bmc_name.to_string())
        .label("system", system_id.to_string())
        .label("storage", storage_id.to_string())
        .label("id", id.to_string())
        .build(value),
    );
}

#[cfg(test)]
mod tests {
    use super::is_absent_raid_controller_error;

    #[test]
    fn recognizes_only_the_ieit_empty_raid_response() {
        let expected = "BMC error: Invalid HTTP response - status: 500 Internal Server Error \
            text: { \"error\": \"There are no RAID Controller Available\", \"code\": 17034 }";
        assert!(is_absent_raid_controller_error(expected));

        assert!(!is_absent_raid_controller_error(
            "status: 500 Internal Server Error: controller temporarily unavailable"
        ));
        assert!(!is_absent_raid_controller_error(
            "status: 404 Not Found: code 17034, There are no RAID Controller Available"
        ));
        assert!(!is_absent_raid_controller_error(
            "status: 500 Internal Server Error: code 17035, There are no RAID Controller Available"
        ));
    }
}
