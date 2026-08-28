use serde::Deserialize;
use std::{
    fmt, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

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

#[derive(Debug, Clone)]
pub struct StabilityConfig {
    pub cooldown_failures: u32,
    pub cooldown_base: Duration,
    pub cooldown_max: Duration,
}

/// High-cardinality collectors are disabled by default. Operators may opt in,
/// but the limits remain bounded so a malformed or very large BMC inventory
/// cannot create an unbounded Prometheus snapshot.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CollectorsConfig {
    pub event_logs: bool,
    pub event_log_limit: usize,
    pub bios_attributes: bool,
    pub bios_attribute_limit: usize,
}

pub const MAX_EVENT_LOG_LIMIT: usize = 5_000;
pub const MAX_BIOS_ATTRIBUTE_LIMIT: usize = 10_000;

impl Default for CollectorsConfig {
    fn default() -> Self {
        Self {
            event_logs: false,
            event_log_limit: 500,
            bios_attributes: false,
            bios_attribute_limit: 10_000,
        }
    }
}

impl CollectorsConfig {
    pub fn all_enabled() -> Self {
        Self {
            event_logs: true,
            bios_attributes: true,
            ..Self::default()
        }
    }
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
    pub stability: StabilityConfig,
    pub collectors: CollectorsConfig,
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
#[serde(deny_unknown_fields)]
struct RawBmcConfig {
    name: String,
    host: String,
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    auth: AuthMethod,
    #[serde(default)]
    insecure_skip_verify: bool,
    #[serde(default)]
    ca_cert_file: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawWebConfig {
    auth_token: Option<String>,
    auth_token_file: Option<PathBuf>,
    tls_cert_file: Option<PathBuf>,
    tls_key_file: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStabilityConfig {
    #[serde(default = "default_cooldown_failures")]
    cooldown_failures: u32,
    #[serde(
        default = "default_cooldown_base",
        deserialize_with = "deserialize_duration"
    )]
    cooldown_base: Duration,
    #[serde(
        default = "default_cooldown_max",
        deserialize_with = "deserialize_duration"
    )]
    cooldown_max: Duration,
}

// 不能 derive Default：serde 的 #[serde(default)] 在整节缺失时调用 Default::default()，
// derive 版会得到全零值，导致缺失 stability 节时校验失败。手动实现返回真实默认值。
impl Default for RawStabilityConfig {
    fn default() -> Self {
        Self {
            cooldown_failures: default_cooldown_failures(),
            cooldown_base: default_cooldown_base(),
            cooldown_max: default_cooldown_max(),
        }
    }
}

fn default_cooldown_failures() -> u32 {
    3
}
fn default_cooldown_base() -> Duration {
    Duration::from_secs(60)
}
fn default_cooldown_max() -> Duration {
    Duration::from_secs(300)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    stability: RawStabilityConfig,
    #[serde(default)]
    collectors: CollectorsConfig,
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

/// 从原始 host 字符串剥离 userinfo（`scheme://user:pass@host` → `scheme://host`），
/// 供错误消息使用：凭据绝不允许回显到启动日志或配置校验错误中。
/// 仅在 authority 段（首个 `/`、`?` 或 `#` 之前）内查找 `@`，路径/查询中的 `@` 不受影响。
fn redact_userinfo(raw: &str) -> String {
    let host_start = raw.find("://").map_or(0, |i| i + 3);
    let rest = &raw[host_start..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    match authority.rfind('@') {
        Some(at) => format!(
            "{}{}{}",
            &raw[..host_start],
            &authority[at + 1..],
            &rest[authority_end..]
        ),
        None => raw.to_string(),
    }
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    load_config_with_env(path, |key| std::env::var(key).ok())
}

pub fn load_config_with_env(
    path: &Path,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Result<Config, ConfigError> {
    let mut raw: RawConfig = {
        // The source buffer itself contains every YAML password/token. Zeroize it
        // after deserialization instead of leaving a dropped String allocation behind.
        let text = Zeroizing::new(std::fs::read_to_string(path)?);
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
        let host = Url::parse(&b.host).map_err(|e| {
            ConfigError::Invalid(format!("host '{}': {e}", redact_userinfo(&b.host)))
        })?;
        if !matches!(host.scheme(), "http" | "https") {
            return Err(ConfigError::Invalid(format!(
                "host '{}': scheme must be http or https",
                redact_userinfo(&b.host)
            )));
        }
        if host.host_str().is_none() {
            return Err(ConfigError::Invalid(format!(
                "host '{}': host name or IP address is required",
                redact_userinfo(&b.host)
            )));
        }
        if !host.username().is_empty() || host.password().is_some() {
            return Err(ConfigError::Invalid(format!(
                "host '{}': credentials in URL are not allowed; use the username/password fields",
                redact_userinfo(&b.host)
            )));
        }
        if host.query().is_some() || host.fragment().is_some() {
            return Err(ConfigError::Invalid(format!(
                "host '{}': query strings and fragments are not allowed in a BMC base URL",
                redact_userinfo(&b.host)
            )));
        }
        if host.path() != "/" {
            return Err(ConfigError::Invalid(format!(
                "host '{}': BMC base URL must not contain a path",
                redact_userinfo(&b.host)
            )));
        }
        if host.port() == Some(0) {
            return Err(ConfigError::Invalid(format!(
                "host '{}': port must be greater than zero",
                redact_userinfo(&b.host)
            )));
        }
        if b.username.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': username must not be empty",
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
        if password.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': password must be set in config or REDFISH_EXPORTER_PASSWORD_{env_name}",
                b.name
            )));
        }
        if host.scheme() == "http" && (b.insecure_skip_verify || b.ca_cert_file.is_some()) {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': TLS options require an https host",
                b.name
            )));
        }
        if b.insecure_skip_verify && b.ca_cert_file.is_some() {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': insecure_skip_verify and ca_cert_file are mutually exclusive",
                b.name
            )));
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
    let web = build_web_config(&mut raw.web)?;
    if raw.stability.cooldown_failures == 0 {
        return Err(ConfigError::Invalid(
            "stability.cooldown_failures must be > 0".into(),
        ));
    }
    if raw.stability.cooldown_base.is_zero() || raw.stability.cooldown_max.is_zero() {
        return Err(ConfigError::Invalid(
            "stability: cooldown_base and cooldown_max must be > 0".into(),
        ));
    }
    if raw.stability.cooldown_max < raw.stability.cooldown_base {
        return Err(ConfigError::Invalid(
            "stability: cooldown_max must be >= cooldown_base".into(),
        ));
    }
    if !(1..=MAX_EVENT_LOG_LIMIT).contains(&raw.collectors.event_log_limit) {
        return Err(ConfigError::Invalid(format!(
            "collectors.event_log_limit must be between 1 and {MAX_EVENT_LOG_LIMIT}"
        )));
    }
    if !(1..=MAX_BIOS_ATTRIBUTE_LIMIT).contains(&raw.collectors.bios_attribute_limit) {
        return Err(ConfigError::Invalid(format!(
            "collectors.bios_attribute_limit must be between 1 and {MAX_BIOS_ATTRIBUTE_LIMIT}"
        )));
    }
    Ok(Config {
        listen_addr,
        scrape_interval: raw.scrape_interval,
        slow_interval: raw.slow_interval,
        scrape_timeout: raw.scrape_timeout,
        request_timeout: raw.request_timeout,
        bmcs,
        web,
        stability: StabilityConfig {
            cooldown_failures: raw.stability.cooldown_failures,
            cooldown_base: raw.stability.cooldown_base,
            cooldown_max: raw.stability.cooldown_max,
        },
        collectors: raw.collectors,
    })
}

/// 校验并构建 web 配置节：token 长度下限、token 与 token_file 互斥、TLS 证书与密钥成对。
/// YAML token 与 token 文件内容等中间字符串用 Zeroizing 包裹，错误路径也不残留明文。
fn build_web_config(raw: &mut RawWebConfig) -> Result<WebConfig, ConfigError> {
    if raw.auth_token.is_some() && raw.auth_token_file.is_some() {
        if let Some(mut t) = raw.auth_token.take() {
            t.zeroize();
        }
        return Err(ConfigError::Invalid(
            "web: auth_token and auth_token_file are mutually exclusive".into(),
        ));
    }
    let auth_token = if let Some(t) = raw.auth_token.take() {
        let t = Zeroizing::new(t);
        validate_auth_token(&t, "web.auth_token")?;
        Some(SecretString::new((*t).clone()))
    } else if let Some(path) = raw.auth_token_file.take() {
        let content = Zeroizing::new(std::fs::read_to_string(&path).map_err(ConfigError::Io)?);
        let token = Zeroizing::new(content.trim().to_string());
        validate_auth_token(&token, &format!("web.auth_token_file '{}'", path.display()))?;
        Some(SecretString::new((*token).clone()))
    } else {
        None
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

fn validate_auth_token(token: &str, field: &str) -> Result<(), ConfigError> {
    if token.len() < 16 {
        return Err(ConfigError::Invalid(format!(
            "{field}: token must be at least 16 characters"
        )));
    }
    if !token.is_ascii()
        || token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(ConfigError::Invalid(format!(
            "{field}: token must contain only visible ASCII characters without whitespace"
        )));
    }
    Ok(())
}

/// Unix：配置文件是否对 group/other 可读（mode & 0o077 != 0）。Windows 无此函数。
#[cfg(unix)]
pub fn config_file_is_wide_open(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .map(|m| m.mode() & 0o077 != 0)
        .unwrap_or(false)
}
