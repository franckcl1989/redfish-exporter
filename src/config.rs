use serde::Deserialize;
use std::{
    fmt, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use url::Url;
use zeroize::Zeroize;

#[derive(Clone)]
pub struct SecretString(String);
impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    #[default]
    Basic,
    Session,
}

#[derive(Debug)]
pub struct BmcConfig {
    pub name: String,
    pub host: Url,
    pub username: String,
    pub password: SecretString,
    pub auth: AuthMethod,
    pub insecure_skip_verify: bool,
    pub ca_cert_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct WebConfig {
    pub auth_token: Option<SecretString>,
    pub tls_cert_file: Option<PathBuf>,
    pub tls_key_file: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub scrape_interval: Duration,
    pub slow_interval: Option<Duration>,
    pub scrape_timeout: Duration,
    pub request_timeout: Duration,
    pub bmcs: Vec<BmcConfig>,
    pub web: WebConfig,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Io(#[from] io::Error),
    #[error("failed to parse config: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Deserialize)]
struct RawBmcConfig {
    name: String,
    host: String,
    username: String,
    password: String,
    #[serde(default)]
    auth: AuthMethod,
    #[serde(default)]
    insecure_skip_verify: bool,
    #[serde(default)]
    ca_cert_file: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
struct RawWebConfig {
    auth_token: Option<String>,
    auth_token_file: Option<PathBuf>,
    tls_cert_file: Option<PathBuf>,
    tls_key_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default = "default_listen_addr")]
    listen_addr: String,
    #[serde(
        default = "default_interval",
        deserialize_with = "deserialize_duration"
    )]
    scrape_interval: Duration,
    #[serde(
        default = "default_none_duration",
        deserialize_with = "deserialize_optional_duration"
    )]
    slow_interval: Option<Duration>,
    #[serde(
        default = "default_scrape_timeout",
        deserialize_with = "deserialize_duration"
    )]
    scrape_timeout: Duration,
    #[serde(
        default = "default_request_timeout",
        deserialize_with = "deserialize_duration"
    )]
    request_timeout: Duration,
    #[serde(default)]
    bmcs: Vec<RawBmcConfig>,
    #[serde(default)]
    web: RawWebConfig,
}

fn default_listen_addr() -> String {
    "127.0.0.1:9417".into()
}
fn default_interval() -> Duration {
    Duration::from_secs(30)
}
fn default_none_duration() -> Option<Duration> {
    None
}
fn default_scrape_timeout() -> Duration {
    Duration::from_secs(15)
}
fn default_request_timeout() -> Duration {
    Duration::from_secs(10)
}

fn deserialize_duration<'de, D>(de: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(de)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

fn deserialize_optional_duration<'de, D>(de: D) -> Result<Option<Duration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(de)?;
    opt.map(|s| humantime::parse_duration(&s).map_err(serde::de::Error::custom))
        .transpose()
}

/// 生成 BMC 凭据对应的环境变量名后缀：name 大写、非字母数字替换为 `_`。
/// 完整变量名：`REDFISH_EXPORTER_PASSWORD_<后缀>`。
pub fn env_var_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    load_config_with_env(path, |key| std::env::var(key).ok())
}

pub fn load_config_with_env(
    path: &Path,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Result<Config, ConfigError> {
    let raw: RawConfig = {
        let text = std::fs::read_to_string(path)?;
        serde_yaml_ng::from_str(&text)?
    };
    let mut names = std::collections::HashSet::new();
    let mut env_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut bmcs = Vec::with_capacity(raw.bmcs.len());
    for b in raw.bmcs {
        if b.name.trim().is_empty() {
            return Err(ConfigError::Invalid("bmc name must not be empty".into()));
        }
        if b.name.chars().any(|c| c.is_control()) {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': name must not contain control characters",
                b.name
            )));
        }
        if !names.insert(b.name.clone()) {
            return Err(ConfigError::Invalid(format!(
                "duplicate bmc name '{}'",
                b.name
            )));
        }
        let host = Url::parse(&b.host)
            .map_err(|e| ConfigError::Invalid(format!("host '{}': {e}", b.host)))?;
        if !matches!(host.scheme(), "http" | "https") {
            return Err(ConfigError::Invalid(format!(
                "host '{}': scheme must be http or https",
                b.host
            )));
        }
        if !host.username().is_empty() || host.password().is_some() {
            return Err(ConfigError::Invalid(format!(
                "host '{}': credentials in URL are not allowed; use the username/password fields",
                b.host
            )));
        }
        if b.password.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': password must not be empty",
                b.name
            )));
        }
        let env_name = env_var_name(&b.name);
        if let Some(prev) = env_names.insert(env_name.clone(), b.name.clone())
            && prev != b.name
        {
            return Err(ConfigError::Invalid(format!(
                "bmc names '{prev}' and '{}' map to the same env var REDFISH_EXPORTER_PASSWORD_{env_name}",
                b.name
            )));
        }
        let mut password = b.password;
        if let Some(env_pw) = env_lookup(&format!("REDFISH_EXPORTER_PASSWORD_{env_name}")) {
            if env_pw.is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "env var REDFISH_EXPORTER_PASSWORD_{env_name} is set but empty"
                )));
            }
            password.zeroize();
            password = env_pw;
        }
        bmcs.push(BmcConfig {
            name: b.name,
            host,
            username: b.username,
            password: SecretString::new(password),
            auth: b.auth,
            insecure_skip_verify: b.insecure_skip_verify,
            ca_cert_file: b.ca_cert_file,
        });
    }
    if bmcs.is_empty() {
        return Err(ConfigError::Invalid("at least one bmc is required".into()));
    }
    let listen_addr = raw
        .listen_addr
        .parse::<SocketAddr>()
        .map_err(|e| ConfigError::Invalid(format!("listen_addr '{}': {e}", raw.listen_addr)))?;
    if raw.scrape_interval.is_zero() {
        return Err(ConfigError::Invalid("scrape_interval must be > 0".into()));
    }
    if let Some(si) = raw.slow_interval
        && si.is_zero()
    {
        return Err(ConfigError::Invalid("slow_interval must be > 0".into()));
    }
    if raw.scrape_timeout.is_zero() {
        return Err(ConfigError::Invalid("scrape_timeout must be > 0".into()));
    }
    if raw.request_timeout.is_zero() {
        return Err(ConfigError::Invalid("request_timeout must be > 0".into()));
    }
    let web = build_web_config(&raw.web)?;
    Ok(Config {
        listen_addr,
        scrape_interval: raw.scrape_interval,
        slow_interval: raw.slow_interval,
        scrape_timeout: raw.scrape_timeout,
        request_timeout: raw.request_timeout,
        bmcs,
        web,
    })
}

/// 校验并构建 web 配置节：token 长度下限、token 与 token_file 互斥、TLS 证书与密钥成对。
fn build_web_config(raw: &RawWebConfig) -> Result<WebConfig, ConfigError> {
    let auth_token = match (&raw.auth_token, &raw.auth_token_file) {
        (Some(_), Some(_)) => {
            return Err(ConfigError::Invalid(
                "web: auth_token and auth_token_file are mutually exclusive".into(),
            ));
        }
        (Some(t), None) => {
            if t.len() < 16 {
                return Err(ConfigError::Invalid(
                    "web.auth_token must be at least 16 characters".into(),
                ));
            }
            Some(SecretString::new(t.clone()))
        }
        (None, Some(path)) => {
            let content = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
            let token = content.trim().to_string();
            if token.len() < 16 {
                return Err(ConfigError::Invalid(format!(
                    "web.auth_token_file '{}': token must be at least 16 characters",
                    path.display()
                )));
            }
            Some(SecretString::new(token))
        }
        (None, None) => None,
    };
    let tls = match (&raw.tls_cert_file, &raw.tls_key_file) {
        (None, None) => (None, None),
        (Some(_), None) | (None, Some(_)) => {
            return Err(ConfigError::Invalid(
                "web: tls_cert_file and tls_key_file must be set together".into(),
            ));
        }
        (Some(c), Some(k)) => (Some(c.clone()), Some(k.clone())),
    };
    Ok(WebConfig {
        auth_token,
        tls_cert_file: tls.0,
        tls_key_file: tls.1,
    })
}
