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
async fn resolves_path_relative_next_link() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "Members": [{"Id": "2"}],
            "@odata.nextLink": "Entries?$skip=1",
        }),
    ));
    let relative = "/redfish/v1/Managers/1/LogServices/SEL/Entries?$skip=1";
    bmc.expect(Expect::get(
        relative,
        json!({"@odata.id": relative, "Members": [{"Id": "1"}]}),
    ));

    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 2);
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
    let result = fetch_all_pages(&bmc, &url).await;
    assert!(matches!(
        result,
        Err(redfish_exporter::pagination::PaginationError::InvalidNextLink(_))
    ));
}

#[tokio::test]
async fn stops_on_repeated_next_link_loop() {
    // 浪潮等 BMC 固件缺陷：nextLink 恒指同一 URL（$skip 被忽略）。
    // 期望：抓两页后检测到循环停止，不无限抓取。
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    let looped = format!("{ENTRIES}?$skip=100&$top=100");
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "Members": [{"Id": "1"}],
            "Members@odata.nextLink": looped,
        }),
    ));
    bmc.expect(Expect::get(
        &looped,
        json!({
            "@odata.id": looped,
            "Members": [{"Id": "2"}],
            "Members@odata.nextLink": looped,
        }),
    ));
    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["Id"], "1");
    assert_eq!(pages[1]["Id"], "2");
}

#[tokio::test]
async fn errors_when_page_limit_exceeded() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    for i in 0..3 {
        let id = if i == 0 {
            ENTRIES.to_string()
        } else {
            format!("{ENTRIES}?$skip={i}")
        };
        bmc.expect(Expect::get(
            &id,
            json!({
                "@odata.id": id,
                "Members": [{"Id": i}],
                "Members@odata.nextLink": format!("{ENTRIES}?$skip={}", i + 1),
            }),
        ));
    }
    let res =
        redfish_exporter::pagination::fetch_all_pages_with_limits(&bmc, &url, 2, 1024 * 1024, 100)
            .await;
    assert!(matches!(
        res,
        Err(redfish_exporter::pagination::PaginationError::TooManyPages(
            2
        ))
    ));
}

#[tokio::test]
async fn errors_when_page_exceeds_size_limit() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    let big = "x".repeat(4096);
    bmc.expect(Expect::get(
        ENTRIES,
        json!({ "@odata.id": ENTRIES, "Members": [{"Id": "0", "Message": big}] }),
    ));
    let res =
        redfish_exporter::pagination::fetch_all_pages_with_limits(&bmc, &url, 1000, 512, 1000)
            .await;
    assert!(matches!(
        res,
        Err(redfish_exporter::pagination::PaginationError::PageTooLarge)
    ));
}

#[tokio::test]
async fn errors_when_total_members_exceeded() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(
        ENTRIES,
        json!({ "@odata.id": ENTRIES, "Members": [{"Id": "1"}, {"Id": "2"}, {"Id": "3"}] }),
    ));
    let res =
        redfish_exporter::pagination::fetch_all_pages_with_limits(&bmc, &url, 1000, 1024 * 1024, 2)
            .await;
    assert!(matches!(
        res,
        Err(redfish_exporter::pagination::PaginationError::TooManyMembers(2))
    ));
}

#[tokio::test]
async fn capped_fetch_truncates_without_requesting_another_page() {
    let bmc = Arc::new(Mock::default());
    let url: ODataId = ENTRIES.to_string().into();
    bmc.expect(Expect::get(
        ENTRIES,
        json!({
            "@odata.id": ENTRIES,
            "Members": [{"Id": "3"}, {"Id": "2"}, {"Id": "1"}],
            "Members@odata.nextLink": format!("{ENTRIES}?$skip=3")
        }),
    ));

    let pages = redfish_exporter::pagination::fetch_pages_up_to(&bmc, &url, 2)
        .await
        .unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["Id"], "3");
    assert_eq!(pages[1]["Id"], "2");
}
