use redfish_exporter::config::{AuthMethod, ConfigError, load_config};
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
    let cfg = load_config(&p).unwrap();
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
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn rejects_non_http_host() {
    let p = write_tmp(
        "ftp",
        r#"
listen_addr: "0.0.0.0:9417"
bmcs:
  - { name: a, host: ftp://h1, username: u, password: "p" }
"#,
    );
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn rejects_missing_file() {
    assert!(matches!(
        load_config(std::path::Path::new("C:\\nonexistent\\x.yaml")),
        Err(ConfigError::Io(_))
    ));
}

#[test]
fn secret_is_redacted_in_debug() {
    let s = redfish_exporter::config::SecretString::new("hunter2".into());
    assert!(!format!("{s:?}").contains("hunter2"));
}
