use redfish_exporter::config::{AuthMethod, ConfigError, load_config_with_env};
use std::io::Write;

fn write_tmp(name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("redfish-exporter-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{name}.yaml"));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    p
}

#[test]
fn parses_valid_config() {
    let p = write_tmp(
        "valid",
        r#"
listen_addr: "0.0.0.0:9417"
scrape_interval: "30s"
scrape_timeout: "15s"
request_timeout: "10s"
bmcs:
  - name: bmc1
    host: https://10.0.0.1
    username: admin
    password: "secret"
    auth: session
"#,
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.bmcs.len(), 1);
    assert_eq!(cfg.bmcs[0].host.host_str(), Some("10.0.0.1"));
    assert_eq!(cfg.bmcs[0].host.path(), "/");
    assert_eq!(cfg.bmcs[0].auth, AuthMethod::Session);
    assert_eq!(cfg.bmcs[0].password.expose(), "secret");
    assert_eq!(cfg.scrape_interval, std::time::Duration::from_secs(30));
}

#[test]
fn rejects_duplicate_bmc_names() {
    let p = write_tmp(
        "dup",
        r#"
listen_addr: "0.0.0.0:9417"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
  - { name: a, host: https://h2, username: u, password: "p" }
"#,
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn rejects_non_http_host() {
    let p = write_tmp(
        "ftp",
        r#"
listen_addr: "0.0.0.0:9417"
bmcs:
  - { name: a, host: ftp://user:secret@h1, username: u, password: "p" }
"#,
    );
    let err = load_config_with_env(&p, |_| None).unwrap_err();
    assert!(
        !err.to_string().contains("secret"),
        "credentials leaked into error: {err}"
    );
    assert!(matches!(err, ConfigError::Invalid(_)));
}

#[test]
fn rejects_missing_file() {
    assert!(matches!(
        load_config_with_env(std::path::Path::new("C:\\nonexistent\\x.yaml"), |_| None),
        Err(ConfigError::Io(_))
    ));
}

#[test]
fn secret_is_redacted_in_debug() {
    let s = redfish_exporter::config::SecretString::new("hunter2".into());
    assert!(!format!("{s:?}").contains("hunter2"));
}

#[test]
fn secret_zeroizes_on_drop() {
    // 与 zeroize crate 自身测试同法：记录缓冲区指针，drop 后读取原内存位置。
    // 产品代码保持 #![forbid(unsafe_code)]，本测试是唯一的 unsafe 例外（验证类代码）。
    let s = redfish_exporter::config::SecretString::new("super-secret-password-123".repeat(4));
    let ptr = s.expose().as_ptr();
    let len = s.expose().len();
    let mut owned = std::mem::ManuallyDrop::new(s);
    unsafe { std::mem::ManuallyDrop::drop(&mut owned) };
    // 已释放内存由分配器保留（drop 与读取之间无新分配），内容可读
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    // Windows 堆释放时分配器（HeapFree）会向已释放块头部写入 16 字节空闲链表指针
    // （drop 胶水顺序：本类型的 Drop 清零先执行，String 字段随后释放时才写入；
    // glibc tcache 同理写入 8 字节），故只校验头部之后的字节——
    // 实现前 RED 证据中密码明文即位于 16 字节之后，此断言仍可捕获未清零。
    assert!(
        bytes[16..].iter().all(|&b| b == 0),
        "secret not zeroized: {:?}",
        &bytes[..bytes.len().min(32)]
    );
}

#[test]
fn parses_slow_interval() {
    let p = write_tmp(
        "slow",
        r#"
scrape_interval: "30s"
slow_interval: "300s"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
"#,
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.slow_interval, Some(std::time::Duration::from_secs(300)));
}

#[test]
fn slow_interval_defaults_to_none() {
    let p = write_tmp(
        "no_slow",
        r#"
scrape_interval: "30s"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
"#,
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.slow_interval, None);
}

#[test]
fn rejects_zero_slow_interval() {
    let p = write_tmp(
        "slow_zero",
        r#"
scrape_interval: "30s"
slow_interval: "0s"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
"#,
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn rejects_host_with_userinfo() {
    let p = write_tmp(
        "userinfo",
        r#"
bmcs:
  - { name: a, host: https://user:secret@h1, username: u, password: "p" }
"#,
    );
    let err = load_config_with_env(&p, |_| None).unwrap_err();
    // 防回归：错误消息（启动日志与 /reload 400 响应体）不得回显 URL 中的凭据
    assert!(
        !err.to_string().contains("secret"),
        "credentials leaked into error: {err}"
    );
    assert!(matches!(err, ConfigError::Invalid(_)));
}

#[test]
fn parse_error_does_not_leak_userinfo() {
    let p = write_tmp(
        "userinfo_parse",
        "bmcs:\n  - { name: a, host: \"https://user:secret@\", username: u, password: \"p\" }\n",
    );
    let err = load_config_with_env(&p, |_| None).unwrap_err();
    assert!(
        !err.to_string().contains("secret"),
        "credentials leaked into error: {err}"
    );
    assert!(matches!(err, ConfigError::Invalid(_)));
}

#[test]
fn rejects_control_chars_in_bmc_name() {
    let p = write_tmp(
        "ctrl_name",
        "bmcs:\n  - { name: \"a\\nb\", host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn default_listen_addr_is_localhost() {
    let p = write_tmp(
        "default_bind",
        "bmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.listen_addr, "127.0.0.1:9417".parse().unwrap());
}

#[test]
fn env_var_overrides_password() {
    let p = write_tmp(
        "env_override",
        "bmcs:\n  - { name: my-bmc, host: https://h1, username: u, password: \"filepw\" }\n",
    );
    let cfg = load_config_with_env(&p, |k| {
        (k == "REDFISH_EXPORTER_PASSWORD_MY_BMC").then(|| "envpw".to_string())
    })
    .unwrap();
    assert_eq!(cfg.bmcs[0].password.expose(), "envpw");
}

#[test]
fn env_var_absent_keeps_file_password() {
    let p = write_tmp(
        "env_absent",
        "bmcs:\n  - { name: my-bmc, host: https://h1, username: u, password: \"filepw\" }\n",
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.bmcs[0].password.expose(), "filepw");
}

#[test]
fn empty_env_var_is_rejected() {
    let p = write_tmp(
        "env_empty",
        "bmcs:\n  - { name: my-bmc, host: https://h1, username: u, password: \"filepw\" }\n",
    );
    let res = load_config_with_env(&p, |k| {
        (k == "REDFISH_EXPORTER_PASSWORD_MY_BMC").then(String::new)
    });
    assert!(matches!(res, Err(ConfigError::Invalid(_))));
}

#[test]
fn colliding_env_names_are_rejected() {
    let p = write_tmp(
        "env_collide",
        r#"
bmcs:
  - { name: a-b, host: https://h1, username: u, password: "p" }
  - { name: a_b, host: https://h2, username: u, password: "p" }
"#,
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn web_token_too_short_is_rejected() {
    let p = write_tmp(
        "web_token_short",
        "web:\n  auth_token: short\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn web_token_and_file_are_mutually_exclusive() {
    let p = write_tmp(
        "web_token_both",
        "web:\n  auth_token: 0123456789abcdef\n  auth_token_file: /tmp/tok\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn web_tls_cert_requires_key() {
    let p = write_tmp(
        "web_tls_half",
        "web:\n  tls_cert_file: /tmp/cert.pem\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Invalid(_))
    ));
}

#[cfg(unix)]
#[test]
fn detects_wide_open_config_permissions() {
    use redfish_exporter::config::config_file_is_wide_open;
    use std::os::unix::fs::PermissionsExt;
    let p = write_tmp("perm_wide", "bmcs: []\n");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(config_file_is_wide_open(&p));
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(
        config_file_is_wide_open(&p),
        "group-readable must count as wide open"
    );
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(
        config_file_is_wide_open(&p),
        "world-writable must count as wide open"
    );
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!config_file_is_wide_open(&p));
}

#[test]
fn web_token_file_content_is_loaded() {
    let dir = std::env::temp_dir().join(format!("redfish-exporter-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tok_path = dir.join("webtoken");
    std::fs::write(&tok_path, "0123456789abcdef\n").unwrap();
    let p = write_tmp(
        "web_token_file",
        &format!(
            "web:\n  auth_token_file: {}\nbmcs:\n  - {{ name: a, host: https://h1, username: u, password: \"p\" }}\n",
            tok_path.display()
        ),
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(
        cfg.web.auth_token.as_ref().unwrap().expose(),
        "0123456789abcdef"
    );
}

#[test]
fn web_token_file_missing_fails_config_load() {
    let dir = std::env::temp_dir().join(format!("redfish-exporter-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("missing-token-file");
    let p = write_tmp(
        "web_token_file_missing",
        &format!(
            "web:\n  auth_token_file: {}\nbmcs:\n  - {{ name: a, host: https://h1, username: u, password: \"p\" }}\n",
            missing.display()
        ),
    );
    assert!(matches!(
        load_config_with_env(&p, |_| None),
        Err(ConfigError::Io(_))
    ));
}
