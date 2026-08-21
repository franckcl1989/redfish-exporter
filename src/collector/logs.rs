use crate::metrics::Metric;
use crate::pagination::fetch_all_pages;
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const EVENT_LOG_ENTRY: (&str, &str) = (
    "redfish_event_log_entry",
    "Event log entry, value is the entry creation time as a Unix timestamp",
);

pub async fn collect_event_logs<B: Bmc>(
    bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(managers) = root
        .managers()
        .await
        .map_err(|e| format!("managers: {e}"))?
    else {
        return Ok(out);
    };
    let managers = managers
        .members()
        .await
        .map_err(|e| format!("manager members: {e}"))?;
    for manager in managers {
        let manager_id = manager.id().to_string();
        let Ok(Some(services)) = manager.log_services().await else {
            continue;
        };
        for service in services {
            let service_id = service.id().to_string();
            let raw = service.raw();
            let Some(entries_ref) = &raw.entries else {
                continue;
            };
            let Ok(entries) = fetch_all_pages(&bmc, entries_ref.id()).await else {
                continue;
            };
            for value in entries {
                let Ok(entry) =
                    serde_json::from_value::<nv_redfish::schema::log_entry::LogEntry>(value)
                else {
                    continue;
                };
                push_entry(&mut out, bmc_name, &manager_id, &service_id, &entry);
            }
        }
    }
    Ok(out)
}

fn severity_label(severity: &nv_redfish::schema::log_entry::EventSeverity) -> &'static str {
    match severity {
        nv_redfish::schema::log_entry::EventSeverity::Ok => "OK",
        nv_redfish::schema::log_entry::EventSeverity::Warning => "Warning",
        nv_redfish::schema::log_entry::EventSeverity::Critical => "Critical",
        nv_redfish::schema::log_entry::EventSeverity::UnsupportedValue => "UnsupportedValue",
    }
}

fn push_entry(
    out: &mut Vec<Metric>,
    bmc_name: &str,
    manager: &str,
    service: &str,
    entry: &nv_redfish::schema::log_entry::LogEntry,
) {
    let id = entry.base.id.to_string();
    let message = entry.message.clone().flatten().unwrap_or_default();
    let severity = entry
        .severity
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(severity_label)
        .unwrap_or_default();
    let ts = entry
        .created
        .as_ref()
        .and_then(|c| SystemTime::try_from(*c).ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as f64);
    let Some(ts) = ts else { return };
    out.push(
        Metric::gauge(EVENT_LOG_ENTRY.0, EVENT_LOG_ENTRY.1)
            .label("bmc", bmc_name.to_string())
            .label("manager", manager.to_string())
            .label("service", service.to_string())
            .label("severity", severity.to_string())
            .label("message", message)
            .label("id", id)
            .build(ts),
    );
}
