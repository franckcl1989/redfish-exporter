use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::pagination::fetch_all_pages;
use serde_json::json;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

const ENTRIES: &str = "/redfish/v1/Managers/1/LogServices/SEL/Entries";

#[tokio::test]
async fn walks_members_next_link_pages() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "Members": [{"Id": "2"}, {"Id": "1"}],
            "Members@odata.nextLink": format!("{ENTRIES}?$skip=2&$top=2"),
        }),
    ));
    bmc.expect(Expect::get(
        format!("{ENTRIES}?$skip=2&$top=2"),
        json!({
            "@odata.id": format!("{ENTRIES}?$skip=2&$top=2"),
            "Members": [{"Id": "0"}],
        }),
    ));
    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0]["Id"], "2");
    assert_eq!(pages[1]["Id"], "1");
    assert_eq!(pages[2]["Id"], "0");
}

#[tokio::test]
async fn walks_standard_next_link_pages() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "Members": [{"Id": "2"}],
            "@odata.nextLink": format!("{ENTRIES}?$skip=1"),
        }),
    ));
    bmc.expect(Expect::get(
        format!("{ENTRIES}?$skip=1"),
        json!({
            "@odata.id": format!("{ENTRIES}?$skip=1"),
            "Members": [{"Id": "1"}],
        }),
    ));
    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["Id"], "2");
    assert_eq!(pages[1]["Id"], "1");
}

#[tokio::test]
async fn stops_without_next_link() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "Members": [{"Id": "1"}],
        }),
    ));
    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["Id"], "1");
}

#[tokio::test]
async fn rejects_cross_origin_next_link() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(ENTRIES, json!({
        "@odata.id": ENTRIES,
        "Members": [{"Id": "2"}, {"Id": "1"}],
        "Members@odata.nextLink": "https://evil.example/redfish/v1/Managers/1/LogServices/SEL/Entries?$skip=2",
    })));
    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["Id"], "2");
    assert_eq!(pages[1]["Id"], "1");
}
