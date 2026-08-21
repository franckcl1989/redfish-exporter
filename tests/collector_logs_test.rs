use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::logs::collect_event_logs;
use serde_json::json;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

const ENTRIES: &str = "/redfish/v1/Managers/BMC/LogServices/Sel/Entries";

fn entry_json(id: &str, created: &str, message: &str, severity: &str) -> serde_json::Value {
    json!({
        "@odata.id": format!("{ENTRIES}/{id}"),
        "Id": id, "Name": format!("Entry {id}"), "EntryType": "Event",
        "Created": created,
        "Message": message,
        "Severity": severity,
    })
}

fn common_expects(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1",
        json!({
            "@odata.id": "/redfish/v1", "Id": "Root", "Name": "Root",
            "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "Managers": { "@odata.id": "/redfish/v1/Managers" },
            "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Managers",
        json!({
            "@odata.id": "/redfish/v1/Managers",
            "@odata.type": "#ManagerCollection.ManagerCollection",
            "Name": "Managers", "Members": [{ "@odata.id": "/redfish/v1/Managers/BMC" }],
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Managers/BMC",
        json!({
            "@odata.id": "/redfish/v1/Managers/BMC",
            "Id": "BMC", "Name": "BMC", "ManagerType": "BMC",
            "LogServices": { "@odata.id": "/redfish/v1/Managers/BMC/LogServices" },
        }),
    ));
    bmc.expect(Expect::get("/redfish/v1/Managers/BMC/LogServices", json!({
        "@odata.id": "/redfish/v1/Managers/BMC/LogServices",
        "@odata.type": "#LogServiceCollection.LogServiceCollection",
        "Name": "LogServices", "Members": [{ "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel" }],
    })));
    bmc.expect(Expect::get(
        "/redfish/v1/Managers/BMC/LogServices/Sel",
        json!({
            "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel",
            "Id": "Sel", "Name": "SEL", "LogEntryType": "SEL",
            "Entries": { "@odata.id": ENTRIES },
        }),
    ));
}

#[tokio::test]
async fn collects_event_log_entries() {
    let bmc = Arc::new(Mock::default());
    common_expects(&bmc);
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "@odata.type": "#LogEntryCollection.LogEntryCollection",
            "Name": "Entries",
            "Members": [entry_json("1", "2026-08-01T12:00:00Z", "Fan redundancy lost", "Critical")],
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_event_logs(bmc, &root, "bmc1").await.unwrap();
    let entries: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_event_log_entry")
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, 1785585600.0); // 2026-08-01T12:00:00Z
    let severity = entries[0]
        .labels
        .iter()
        .find(|(k, _)| *k == "severity")
        .map(|(_, v)| v.as_str());
    assert_eq!(severity, Some("Critical"));
}

#[tokio::test]
async fn collects_event_log_entries_across_pages() {
    let bmc = Arc::new(Mock::default());
    common_expects(&bmc);
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "@odata.type": "#LogEntryCollection.LogEntryCollection",
            "Name": "Entries",
            "Members": [entry_json("2", "2026-08-01T12:00:00Z", "Fan redundancy lost", "Critical")],
            "Members@odata.nextLink": format!("{ENTRIES}?$skip=1"),
        }),
    ));
    bmc.expect(Expect::get(
        format!("{ENTRIES}?$skip=1"),
        json!({
            "@odata.id": format!("{ENTRIES}?$skip=1"),
            "@odata.type": "#LogEntryCollection.LogEntryCollection",
            "Name": "Entries",
            "Members": [entry_json("1", "2026-08-01T13:00:00Z", "Power supply restored", "OK")],
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_event_logs(bmc, &root, "bmc1").await.unwrap();
    let entries: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_event_log_entry")
        .collect();
    assert_eq!(entries.len(), 2);
    let ts: Vec<_> = entries.iter().map(|m| m.value).collect();
    assert_eq!(ts, vec![1785585600.0, 1785589200.0]); // 12:00Z, 13:00Z
    let severities: Vec<_> = entries
        .iter()
        .map(|m| {
            m.labels
                .iter()
                .find(|(k, _)| *k == "severity")
                .map(|(_, v)| v.as_str())
        })
        .collect();
    assert_eq!(severities, vec![Some("Critical"), Some("OK")]);
}
