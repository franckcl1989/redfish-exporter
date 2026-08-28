use crate::config::MAX_EVENT_LOG_LIMIT;
use crate::metrics::Metric;
use crate::pagination::fetch_pages_up_to;
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
    collect_event_logs_with_limit(bmc, root, bmc_name, MAX_EVENT_LOG_LIMIT).await
}

pub async fn collect_event_logs_with_limit<B: Bmc>(
    bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
    max_entries: usize,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let mut manager_attempts = 0usize;
    let mut manager_failures = 0usize;
    let mut attempted_services = 0usize;
    let mut failed_services = 0usize;
    let mut attempted_entries = 0usize;
    let mut failed_entries = 0usize;
    let mut remaining_entries = max_entries;
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
    'managers: for manager in managers {
        let manager_id = manager.id().to_string();
        let services = match manager.log_services().await {
            Ok(Some(services)) => {
                manager_attempts += 1;
                services
            }
            Ok(None) => continue,
            Err(error) => {
                manager_attempts += 1;
                manager_failures += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    manager = %manager_id,
                    error = %error,
                    "log service collection fetch failed"
                );
                continue;
            }
        };
        for service in services {
            if remaining_entries == 0 {
                break 'managers;
            }
            let service_id = service.id().to_string();
            let raw = service.raw();
            let Some(entries_ref) = &raw.entries else {
                continue;
            };
            attempted_services += 1;
            let entries = match fetch_pages_up_to(&bmc, entries_ref.id(), remaining_entries).await {
                Ok(entries) => entries,
                Err(e) => {
                    failed_services += 1;
                    tracing::warn!(
                        bmc = %bmc_name,
                        manager = %manager_id,
                        service = %service_id,
                        error = %e,
                        "event log pagination failed, skipping log service"
                    );
                    continue;
                }
            };
            remaining_entries = remaining_entries.saturating_sub(entries.len());
            for value in entries {
                attempted_entries += 1;
                let entry = match serde_json::from_value::<nv_redfish::schema::log_entry::LogEntry>(
                    value,
                ) {
                    Ok(entry) => entry,
                    Err(error) => {
                        failed_entries += 1;
                        tracing::warn!(
                            bmc = %bmc_name,
                            manager = %manager_id,
                            service = %service_id,
                            error = %error,
                            "event log entry parse failed"
                        );
                        continue;
                    }
                };
                push_entry(&mut out, bmc_name, &manager_id, &service_id, &entry);
            }
        }
    }
    if remaining_entries == 0 {
        tracing::warn!(
            bmc = %bmc_name,
            max_entries,
            "event log metric limit reached; remaining entries omitted"
        );
    }
    if manager_attempts > 0 && manager_failures == manager_attempts {
        return Err(format!(
            "event logs: all {manager_failures}/{manager_attempts} manager log-service collections failed"
        ));
    }
    if attempted_services > 0 && failed_services == attempted_services {
        return Err(format!(
            "event log pagination: all {failed_services}/{attempted_services} log services failed"
        ));
    }
    if attempted_entries > 0 && failed_entries == attempted_entries {
        return Err(format!(
            "event logs: all {failed_entries}/{attempted_entries} entries failed to parse"
        ));
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
