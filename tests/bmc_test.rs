use redfish_exporter::bmc::{build_http_client, make_bmc};
use redfish_exporter::config::{AuthMethod, BmcConfig, SecretString};
use url::Url;

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
    let c = build_http_client(&cfg()).unwrap();
    // 无法直接断言 reqwest 内部状态；此测试主要验证类型与构造路径可运行。
    let _ = c;
}

#[test]
fn make_bmc_wraps_name() {
    let client = build_http_client(&cfg()).unwrap();
    let handle = make_bmc(&cfg(), client);
    assert_eq!(handle.name, "bmc1");
}
