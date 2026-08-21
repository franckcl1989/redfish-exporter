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

    let err = establish_session(&bmc, "admin", "secret")
        .await
        .unwrap_err();
    assert!(matches!(err, BmcError::Session(_)));
}
