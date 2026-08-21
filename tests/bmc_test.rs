use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::bmc::{BmcError, build_http_client, establish_session, make_bmc};
use redfish_exporter::config::{AuthMethod, BmcConfig, SecretString};
use serde_json::json;
use std::sync::Arc;
use url::Url;

type Mock = MockBmc<serde_json::Error>;

fn cfg() -> BmcConfig {
    BmcConfig {
        name: "bmc1".into(),
        host: Url::parse("https://10.0.0.1").unwrap(),
        username: "admin".into(),
        password: SecretString::new("pw".into()),
        auth: AuthMethod::Basic,
        insecure_skip_verify: true,
        ca_cert_file: None,
    }
}

#[test]
fn builds_reqwest_client_with_tls_options() {
    let c = build_http_client(&cfg(), std::time::Duration::from_secs(10)).unwrap();
    // 无法直接断言 reqwest 内部状态；此测试主要验证类型与构造路径可运行。
    let _ = c;
}

#[test]
fn make_bmc_wraps_name() {
    let client = build_http_client(&cfg(), std::time::Duration::from_secs(10)).unwrap();
    let handle = make_bmc(&cfg(), client);
    assert_eq!(handle.name, "bmc1");
}

#[test]
fn rejects_missing_ca_file() {
    let mut c = cfg();
    c.ca_cert_file = Some("C:\\nonexistent\\ca.pem".into());
    assert!(build_http_client(&c, std::time::Duration::from_secs(10)).is_err());
}

/// 防回归：create_session 失败（如浪潮响应缺 Name 导致反序列化失败）时，
/// establish_session 返回 Err 而非 panic。
#[tokio::test]
async fn try_establish_session_failure_returns_err_not_panic() {
    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get(
        ODataId::service_root(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "SessionService": { "@odata.id": "/redfish/v1/SessionService" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService",
        json!({
            "@odata.id": "/redfish/v1/SessionService",
            "Id": "SessionService", "Name": "Session Service",
            "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService/Sessions",
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions",
            "@odata.type": "#SessionCollection.SessionCollection",
            "Name": "Sessions",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
    // 响应缺少 Name（真实设备浪潮缺陷）→ 反序列化失败 → create_session 返回 Err
    bmc.expect(Expect::create_session(
        "/redfish/v1/SessionService/Sessions",
        json!({ "UserName": "admin", "Password": "secret" }),
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions/1",
            "Id": "1",
            "UserName": "admin",
            "SessionType": "Redfish",
        }),
        "session-token-123",
        "/redfish/v1/SessionService/Sessions/1",
    ));

    let err = match establish_session(&bmc, "admin", "secret").await {
        Err(e) => e,
        Ok(_) => unreachable!("create_session 预期失败"),
    };
    assert!(matches!(err, BmcError::Session(_)));
}

/// establish_session 成功时返回 token 与可删除的会话句柄。
#[tokio::test]
async fn establish_session_returns_token_and_session() {
    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get(
        ODataId::service_root(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "SessionService": { "@odata.id": "/redfish/v1/SessionService" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService",
        json!({
            "@odata.id": "/redfish/v1/SessionService",
            "Id": "SessionService", "Name": "Session Service",
            "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService/Sessions",
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions",
            "@odata.type": "#SessionCollection.SessionCollection",
            "Name": "Sessions",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
    bmc.expect(Expect::create_session(
        "/redfish/v1/SessionService/Sessions",
        json!({ "UserName": "admin", "Password": "secret" }),
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions/1",
            "Id": "1", "Name": "User Session",
            "UserName": "admin",
            "SessionType": "Redfish",
        }),
        "session-token-123",
        "/redfish/v1/SessionService/Sessions/1",
    ));

    let est = establish_session(&bmc, "admin", "secret").await.unwrap();
    assert_eq!(est.token, "session-token-123");
    assert!(est.session.is_some());
}

#[test]
fn redirect_policy_blocks_https_downgrade() {
    use redfish_exporter::bmc::decide_redirect;
    let https = url::Url::parse("https://bmc.example/redfish/v1").unwrap();
    let http = url::Url::parse("http://bmc.example/redfish/v1/Systems").unwrap();
    let https2 = url::Url::parse("https://bmc.example/redfish/v1/Systems").unwrap();
    assert!(
        !decide_redirect(std::slice::from_ref(&https), &http),
        "https->http must be blocked"
    );
    assert!(decide_redirect(&[https], &https2), "https->https allowed");
    assert!(
        decide_redirect(std::slice::from_ref(&http), &http),
        "http->http allowed"
    );
    assert!(
        decide_redirect(&[], &http),
        "no previous hop is not a downgrade"
    );
    let chain: Vec<url::Url> = (0..10)
        .map(|i| url::Url::parse(&format!("https://bmc.example/r{i}")).unwrap())
        .collect();
    assert!(!decide_redirect(&chain, &http), "10-hop limit enforced");
}

/// 会话句柄的 delete() 对会话 URI 发起 DELETE 请求。
#[tokio::test]
async fn session_delete_issues_delete_request() {
    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get(
        ODataId::service_root(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "SessionService": { "@odata.id": "/redfish/v1/SessionService" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService",
        json!({
            "@odata.id": "/redfish/v1/SessionService",
            "Id": "SessionService", "Name": "Session Service",
            "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService/Sessions",
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions",
            "@odata.type": "#SessionCollection.SessionCollection",
            "Name": "Sessions",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
    bmc.expect(Expect::create_session(
        "/redfish/v1/SessionService/Sessions",
        json!({ "UserName": "admin", "Password": "secret" }),
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions/1",
            "Id": "1", "Name": "User Session",
            "UserName": "admin",
            "SessionType": "Redfish",
        }),
        "session-token-123",
        "/redfish/v1/SessionService/Sessions/1",
    ));
    bmc.expect(Expect::delete("/redfish/v1/SessionService/Sessions/1"));

    let est = establish_session(&bmc, "admin", "secret").await.unwrap();
    let res = est.session.unwrap().delete().await.unwrap();
    assert!(matches!(res, nv_redfish::core::ModificationResponse::Empty));
}
