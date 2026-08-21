use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::logs::collect_event_logs;
use serde_json::json;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

#[tokio::test]
async fn collects_event_log_entries() {
    let bmc = Arc::new(Mock::default());
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
            "Entries": { "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Managers/BMC/LogServices/Sel/Entries",
        json!({
            "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries",
            "@odata.type": "#LogEntryCollection.LogEntryCollection",
            "Name": "Entries",
            "Members": [{
                "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries/1",
                "Id": "1", "Name": "Entry 1", "EntryType": "Event",
                "Created": "2026-08-01T12:00:00Z",
                "Message": "Fan redundancy lost",
                "Severity": "Critical",
            }],
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/Managers/BMC/LogServices/Sel/Entries/1",
        json!({
            "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries/1",
            "Id": "1", "Name": "Entry 1", "EntryType": "Event",
            "Created": "2026-08-01T12:00:00Z",
            "Message": "Fan redundancy lost",
            "Severity": "Critical",
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_event_logs(bmc, &root, "bmc1").await.unwrap();
    let entry: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_event_log_entry")
        .collect();
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].value, 1785585600.0); // 2026-08-01T12:00:00Z
}
