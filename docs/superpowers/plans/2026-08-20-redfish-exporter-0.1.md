# redfish-exporter 0.1.0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 基于 nv-redfish 0.15.1 实现生产就绪的多 BMC Prometheus exporter：周期采集标准 Redfish 指标并缓存，axum 暴露 `/metrics` 与 `/healthz`，纯静态单二进制分发。

**Architecture:** 单 crate。后台 scraper 周期任务每轮有界并发采集所有 BMC（每个 BMC 独立失败隔离），采集结果组装为 prometheus `Registry` 快照原子替换；axum server 从快照编码响应。collector 按资源分模块，全部走 nv-redfish 高层 wrapper + `raw()` 字段访问。

**Tech Stack:** Rust 2024 (toolchain 1.90.0)、tokio、axum 0.8、clap 4、serde + serde_yaml_ng、prometheus 0.14、tracing、thiserror、anyhow、humantime；`nv-redfish = "0.15"`（crates.io，features 见 Task 0）；测试用 `nv-redfish-bmc-mock = "0.15"`。

**Spec:** `docs/superpowers/specs/2026-08-20-redfish-exporter-0.1-design.md`

## Global Constraints

- 100% safe code：`src/lib.rs` 与 `src/main.rs` 顶部 `#![forbid(unsafe_code)]`；CI 用 `cargo geiger` 检查零 unsafe（允许 geiger 自身警告）
- Rust 2024 edition；`rust-toolchain.toml` 固定 `channel = "1.90.0"`（与 nv-redfish MSRV 对齐）
- 提交 `Cargo.lock`（二进制项目）
- 单一 crate，不拆 workspace
- 无 nightly；无 `RUSTC_BOOTSTRAP`
- Secret（BMC 密码）不进入 `Debug`、日志、错误上下文；类型层面实现 redaction
- 错误全部走 `Result`；`expect` 仅限不可变 invariant 且写明理由；外部输入不 unwrap
- 指标 label 仅 `bmc` + 有限硬件 id；禁止 request-id / 任意文本进 label
- 无 unbounded queue；每轮采集任务数上界 = BMC 数（`JoinSet`）
- 锁内不做网络 I/O；快照用 `RwLock<Option<Arc<Registry>>>`，无全局 `Arc<Mutex<AppState>>`
- 时长测量用 `Instant`（monotonic）
- 依赖最小化：仅本计划列出的依赖；新增依赖必须说明理由
- 每个 Task 以 `cargo fmt --check && cargo clippy -D warnings && cargo test` 通过收尾

## 依赖基线（Task 0 锁定，后续 Task 不得改动版本）

```toml
[dependencies]
nv-redfish = { version = "0.15", default-features = false, features = [
    "bmc-http", "chassis", "computer-systems", "managers", "assembly",
    "sensors", "thermal", "power", "power-supplies", "controls", "environment-metrics",
    "processors", "memory", "storages", "ethernet-interfaces",
    "network-adapters", "ports", "pcie-devices", "update-service", "session-service",
] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "time", "sync"] }
axum = "0.8"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_yaml_ng = "0.10"
prometheus = "0.14"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2"
anyhow = "1"
humantime = "2"
url = "2"

[dev-dependencies]
nv-redfish-bmc-mock = "0.15"
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time", "sync"] }
```

## nv-redfish API 速查（v0.15.1，来自调研；Task 0 需构建后核对生成代码）

- `ServiceRoot::new(bmc: Arc<B>) -> Result<Self, Error<B>>`；入口方法均 `async fn x(&self) -> Result<Option<XCollection<B>>, Error<B>>`（`chassis()`、`systems()`、`managers()`、`update_service()`、`session_service()`）
- 集合：`members() -> Result<Vec<Item<B>>, Error<B>>`；Item：`id()`、`name()`（`Resource` trait）、`raw() -> Arc<XSchema>`（生成类型字段全 pub）
- 传感器：`Chassis::sensor_links() -> Result<Option<Vec<SensorLink<B>>>, Error<B>>`；`SensorLink::fetch() -> Result<Arc<SchemaSensor>, Error<B>>`；Sensor 生成字段（构建后核对）：`reading: Option<Option<f64>>`、`reading_type`、`reading_units`、`thresholds`（`upper_critical`/`lower_critical`/`upper_caution`/`lower_caution`，各含 `reading: Option<Option<f64>>`）
- 枚举生成 Rust enum（`Health`、`State`、`LinkStatus`、`PowerState`、`ReadingType`）；数值 `Edm.Decimal → f64`
- `HttpBmc::new(client, Url, BmcCredentials, CacheSettings)`；`BmcCredentials::username_password(u, Option<String>)` / `token(String)`；`bmc.set_credentials(...)`
- reqwest 客户端：`nv_redfish::bmc_http::reqwest::Client::with_params(ClientParams)`；`ClientParams` 含 `timeout`、`connect_timeout`、`accept_invalid_certs`、`use_rust_tls`（默认 true）
- Session：`root.session_service()` → `sessions()` → `create_session(&SessionCreate::builder(u, p).build())` → `session.auth_token() -> Option<&str>` → `bmc.set_credentials(BmcCredentials::token(t))`
- bmc-mock：`Bmc::default()` + `bmc.expect(Expect::get(uri, json!({...})))`，FIFO 严格匹配；`Expect::expand(uri, json)`；uri 可用 `ODataId::service_root()`；JSON 需含 `ODATA_ID` 字段宏
- 错误：`nv_redfish::Error<B>` 含 `Bmc(B::Error)`、`...NotAvailable` 变体
- `http-extras` 不使用（YAGNI，无并发限制需求）

---

### Task 0: 项目脚手架

**Files:**
- Create: `Cargo.toml`、`rust-toolchain.toml`、`.gitignore`、`src/main.rs`、`src/lib.rs`、`.github/workflows/ci.yml`（占位，Task 13 完善）
- Modify: 无

**Interfaces:**
- Consumes: 无
- Produces: crate 根 `redfish_exporter`（lib + bin）；`src/lib.rs` 声明 `#![forbid(unsafe_code)]`

- [ ] **Step 1: 创建 Cargo.toml**

```toml
[package]
name = "redfish-exporter"
version = "0.1.0"
edition = "2024"
rust-version = "1.90"
license = "Apache-2.0"
description = "Prometheus exporter for Redfish BMCs, built on nv-redfish"

[dependencies]
# 按上方"依赖基线"原样填入
```

同时创建 `rust-toolchain.toml`：`channel = "1.90.0"`、`profile = "minimal"`、`components = ["clippy", "rustfmt"]`；`.gitignore` 含 `/target`。

- [ ] **Step 2: 创建 src/main.rs 与 src/lib.rs**

`src/lib.rs`：
```rust
#![forbid(unsafe_code)]

pub mod bmc;
pub mod collector;
pub mod config;
pub mod http;
pub mod metrics;
pub mod registry;
pub mod scraper;
```
（模块文件 Task 1+ 创建，此步 lib.rs 保留全部声明即可——先建空模块文件 `src/{bmc,collector,config,http,metrics,registry,scraper}.rs`，collector 为目录 `src/collector/mod.rs`。每个模块文件先只放一行空实现或 doc comment，保证编译。）

`src/main.rs`：
```rust
#![forbid(unsafe_code)]

fn main() {
    eprintln!("redfish-exporter: not yet implemented");
}
```

- [ ] **Step 3: 首次构建并核对生成代码**

Run: `cargo check`
Expected: 编译通过（nv-redfish 首编可能 3-10 分钟）

随后核对 nv-redfish 生成类型源码（路径 `~/.cargo/registry/src/*/nv-redfish-0.15.1/src/compiled_schema.rs`），用 Grep 确认以下字段/类型存在并记录精确名称（后续 Task 4-9 以此为准，如与本计划标注不符以生成代码为准并在 commit message 注明）：
- `Sensor` 结构：`reading`、`reading_type`、`reading_units`、`thresholds` 及 `Thresholds`/`Threshold` 的字段名
- `ChassisSchema` 的 `power_subsystem`、`power`、`thermal`、`controls`、`environment_metrics` 导航属性名
- `ProcessorMetrics`：`utilization_percent`、`temperature_celsius`、`power_watts`
- `MemoryMetrics`：`bandwidth_percent`；`Memory` 的 `capacity_mi_b` 或同类容量字段
- `DriveMetrics`：`utilization_percent`、`capacity_bytes`；`Drive` 的 `predicted_media_life_left_percent`、`failure_predicted`（若不存在记录替代字段）
- `EthernetInterface`：`link_status`、`speed_mbps`（枚举 `LinkStatus` 的 variant 名）
- `PCIeDevice` 的链路速率字段（`link_speed_gt_per_sec` 或同类）
- `SoftwareInventory`：`version`、`status`
- `Status`（`health`、`state`）在 Chassis/System/Manager/Drive/Processor/Memory 生成类型 `base` 链中的层级（`raw().base.status` vs `raw().base.base.status`）
将核对结果记录为 commit 前验证项。

- [ ] **Step 4: 运行质量门禁**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Expected: 全部通过（此时仅有空模块）

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore src/ .github/
git commit -m "chore: scaffold redfish-exporter 0.1.0 project"
```

---

### Task 1: 配置模块

**Files:**
- Create: `src/config.rs`、`tests/config_test.rs`
- Modify: `src/lib.rs`（已有声明，无需改）

**Interfaces:**
- Consumes: 无
- Produces:
```rust
pub struct Config { pub listen_addr: SocketAddr, pub scrape_interval: Duration,
    pub scrape_timeout: Duration, pub request_timeout: Duration, pub bmcs: Vec<BmcConfig> }
pub struct BmcConfig { pub name: String, pub host: Url, pub username: String,
    pub password: SecretString, pub auth: AuthMethod, pub insecure_skip_verify: bool,
    pub ca_cert_file: Option<PathBuf> }
pub enum AuthMethod { Basic, Session }
pub struct SecretString(String);
impl SecretString { pub fn new(s: String) -> Self; pub fn expose(&self) -> &str; }
impl fmt::Debug for SecretString { /* 输出 "[REDACTED]" */ }
pub fn load_config(path: &Path) -> Result<Config, ConfigError>
pub enum ConfigError { Io(#[source] io::Error), Yaml(#[source] serde_yaml_ng::Error),
    Invalid(String) }
```
YAML 结构（`BmcConfig` 字段与 YAML 键一致；`scrape_interval` 等用 `#[serde(deserialize_with = "deserialize_duration")]` 支持 `"30s"`/`"15s"`/`"10s"`（humantime 解析））：
```yaml
listen_addr: "0.0.0.0:9417"
scrape_interval: "30s"
scrape_timeout: "15s"
request_timeout: "10s"
bmcs:
  - name: bmc1
    host: https://10.0.0.1
    username: admin
    password: "pw"
    auth: basic        # basic | session
    insecure_skip_verify: false
    ca_cert_file: null
```

- [ ] **Step 1: 写失败测试** `tests/config_test.rs`

```rust
use redfish_exporter::config::{load_config, AuthMethod, ConfigError};
use std::io::Write;

fn write_tmp(content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("redfish-exporter-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config.yaml");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    p
}

#[test]
fn parses_valid_config() {
    let p = write_tmp(r#"
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
"#);
    let cfg = load_config(&p).unwrap();
    assert_eq!(cfg.bmcs.len(), 1);
    assert_eq!(cfg.bmcs[0].host.as_str(), "https://10.0.0.1");
    assert_eq!(cfg.bmcs[0].auth, AuthMethod::Session);
    assert_eq!(cfg.bmcs[0].password.expose(), "secret");
    assert_eq!(cfg.scrape_interval, std::time::Duration::from_secs(30));
}

#[test]
fn rejects_duplicate_bmc_names() {
    let p = write_tmp(r#"
listen_addr: "0.0.0.0:9417"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
  - { name: a, host: https://h2, username: u, password: "p" }
"#);
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn rejects_non_http_host() {
    let p = write_tmp(r#"
listen_addr: "0.0.0.0:9417"
bmcs:
  - { name: a, host: ftp://h1, username: u, password: "p" }
"#);
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn rejects_missing_file() {
    assert!(matches!(load_config(std::path::Path::new("C:\\nonexistent\\x.yaml")),
        Err(ConfigError::Io(_))));
}

#[test]
fn secret_is_redacted_in_debug() {
    let s = redfish_exporter::config::SecretString::new("hunter2".into());
    assert!(!format!("{s:?}").contains("hunter2"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test config_test`
Expected: 编译失败（`redfish_exporter::config` 不存在）

- [ ] **Step 3: 实现 config.rs**

```rust
use std::{fmt, io, net::SocketAddr, path::{Path, PathBuf}, time::Duration};
use anyhow::Context;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

#[derive(Debug)]
pub struct SecretString(String);
impl SecretString {
    pub fn new(s: String) -> Self { Self(s) }
    pub fn expose(&self) -> &str { &self.0 }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("[REDACTED]") }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod { Basic, Session }

#[derive(Debug)]
pub struct BmcConfig {
    pub name: String, pub host: Url, pub username: String, pub password: SecretString,
    pub auth: AuthMethod, pub insecure_skip_verify: bool, pub ca_cert_file: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub scrape_interval: Duration,
    pub scrape_timeout: Duration,
    pub request_timeout: Duration,
    pub bmcs: Vec<BmcConfig>,
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
    name: String, host: String, username: String, password: String,
    #[serde(default)] auth: AuthMethod,
    #[serde(default)] insecure_skip_verify: bool,
    #[serde(default)] ca_cert_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default = "default_listen_addr")] listen_addr: String,
    #[serde(default = "default_interval", deserialize_with = "deserialize_duration")]
    scrape_interval: Duration,
    #[serde(default = "default_scrape_timeout", deserialize_with = "deserialize_duration")]
    scrape_timeout: Duration,
    #[serde(default = "default_request_timeout", deserialize_with = "deserialize_duration")]
    request_timeout: Duration,
    #[serde(default)] bmcs: Vec<RawBmcConfig>,
}

fn default_listen_addr() -> String { "0.0.0.0:9417".into() }
fn default_interval() -> Duration { Duration::from_secs(30) }
fn default_scrape_timeout() -> Duration { Duration::from_secs(15) }
fn default_request_timeout() -> Duration { Duration::from_secs(10) }

fn deserialize_duration<'de, D>(de: D) -> Result<Duration, D::Error>
where D: serde::Deserializer<'de> {
    let s = String::deserialize(de)?;
    humantime::parse_duration(&s).map_err(serde::de::Error::custom)
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let raw: RawConfig = {
        let text = std::fs::read_to_string(path)?;
        serde_yaml_ng::from_str(&text)?
    };
    let mut names = std::collections::HashSet::new();
    let mut bmcs = Vec::with_capacity(raw.bmcs.len());
    for b in raw.bmcs {
        if b.name.trim().is_empty() { return Err(ConfigError::Invalid("bmc name must not be empty".into())); }
        if !names.insert(b.name.clone()) {
            return Err(ConfigError::Invalid(format!("duplicate bmc name '{}'", b.name)));
        }
        let host = Url::parse(&b.host).map_err(|e| ConfigError::Invalid(format!("host '{}': {e}", b.host)))?;
        if !matches!(host.scheme(), "http" | "https") {
            return Err(ConfigError::Invalid(format!("host '{}': scheme must be http or https", b.host)));
        }
        if b.password.is_empty() { return Err(ConfigError::Invalid(format!("bmc '{}': password must not be empty", b.name))); }
        bmcs.push(BmcConfig {
            name: b.name, host, username: b.username,
            password: SecretString::new(b.password), auth: b.auth,
            insecure_skip_verify: b.insecure_skip_verify, ca_cert_file: b.ca_cert_file,
        });
    }
    if bmcs.is_empty() { return Err(ConfigError::Invalid("at least one bmc is required".into())); }
    let listen_addr = raw.listen_addr.parse::<SocketAddr>()
        .map_err(|e| ConfigError::Invalid(format!("listen_addr '{}': {e}", raw.listen_addr)))?;
    if raw.scrape_interval.is_zero() { return Err(ConfigError::Invalid("scrape_interval must be > 0".into())); }
    if raw.scrape_timeout.is_zero() { return Err(ConfigError::Invalid("scrape_timeout must be > 0".into())); }
    if raw.request_timeout.is_zero() { return Err(ConfigError::Invalid("request_timeout must be > 0".into())); }
    Ok(Config { listen_addr, scrape_interval: raw.scrape_interval,
        scrape_timeout: raw.scrape_timeout, request_timeout: raw.request_timeout, bmcs })
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test config_test`
Expected: 5 个测试全部 PASS（注意 secret 测试用 `format!("{s:?}")` 触发自定义 Debug）

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/config.rs tests/config_test.rs && git commit -m "feat(config): typed config loading and validation"`

---

### Task 2: BMC 客户端

**Files:**
- Create: `src/bmc.rs`
- Modify: 无

**Interfaces:**
- Consumes: `config::BmcConfig`、`config::AuthMethod`
- Produces:
```rust
pub struct BmcHandle { pub name: String, pub bmc: Arc<HttpBmc<ReqwestClient>> }
pub fn build_http_client(cfg: &BmcConfig) -> Result<ReqwestClient, BmcError>
pub async fn establish_session(bmc: &Arc<HttpBmc<ReqwestClient>>) -> Result<(), BmcError>
pub enum BmcError { Transport(#[source] anyhow::Error), Session(String) }
```
说明：`HttpBmc` 类型路径为 `nv_redfish::bmc_http::HttpBmc`，reqwest Client 为 `nv_redfish::bmc_http::reqwest::Client`（bmc-http feature 下 re-export）。Basic 认证直接用 `BmcCredentials::username_password` 构造；Session 认证先用 basic 构造 → `establish_session` 换 token。

- [ ] **Step 1: 写失败测试（单元：client 参数构建）**

```rust
// tests/bmc_test.rs
use redfish_exporter::bmc::build_http_client;
use redfish_exporter::config::{AuthMethod, BmcConfig, SecretString};
use url::Url;

fn cfg() -> BmcConfig {
    BmcConfig { name: "bmc1".into(), host: Url::parse("https://10.0.0.1").unwrap(),
        username: "admin".into(), password: SecretString::new("pw".into()),
        auth: AuthMethod::Basic, insecure_skip_verify: true, ca_cert_file: None }
}

#[test]
fn builds_reqwest_client_with_tls_options() {
    let c = build_http_client(&cfg()).unwrap();
    // 无法直接断言 reqwest 内部状态；此测试主要验证类型与构造路径可运行。
    let _ = c;
}

#[test]
fn rejects_missing_ca_file() {
    let mut c = cfg();
    c.ca_cert_file = Some("C:\\nonexistent\\ca.pem".into());
    assert!(build_http_client(&c).is_err());
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test bmc_test`
Expected: 编译失败（模块不存在）

- [ ] **Step 3: 实现 bmc.rs**

```rust
use std::sync::Arc;
use anyhow::Context;
use thiserror::Error;
use url::Url;

use crate::config::{AuthMethod, BmcConfig};

pub type ReqwestClient = nv_redfish::bmc_http::reqwest::Client;
pub type HttpBmc<C> = nv_redfish::bmc_http::HttpBmc<C>;

#[derive(Debug, Error)]
pub enum BmcError {
    #[error("failed to build http client for bmc: {0}")]
    Transport(#[from] anyhow::Error),
    #[error("session establishment failed: {0}")]
    Session(String),
}

pub struct BmcHandle {
    pub name: String,
    pub bmc: Arc<HttpBmc<ReqwestClient>>,
}

pub fn build_http_client(cfg: &BmcConfig) -> Result<ReqwestClient, BmcError> {
    let mut params = nv_redfish::bmc_http::reqwest::ClientParams::default();
    params.accept_invalid_certs = cfg.insecure_skip_verify;
    if let Some(ca) = &cfg.ca_cert_file {
        let pem = std::fs::read(ca)
            .with_context(|| format!("read ca_cert_file '{}'", ca.display()))?;
        let cert = reqwest_tls_root(&pem).with_context(|| format!("parse ca_cert_file '{}'", ca.display()))?;
        params.root_certificates = Some(vec![cert]);
    }
    ReqwestClient::with_params(params).map_err(Into::into)
}

// reqwest crate 类型，经 nv-redfish re-export 路径引用：
fn reqwest_tls_root(pem: &[u8]) -> anyhow::Result<nv_redfish::bmc_http::reqwest::Certificate> {
    use nv_redfish::bmc_http::reqwest::Certificate;
    Certificate::from_pem(pem).map_err(Into::into)
}

pub fn make_bmc(cfg: &BmcConfig, client: ReqwestClient) -> BmcHandle {
    let credentials = nv_redfish::bmc_http::BmcCredentials::username_password(
        cfg.username.clone(), Some(cfg.password.expose().to_string()));
    let bmc = HttpBmc::new(client, cfg.host.clone(), credentials,
        nv_redfish::bmc_http::CacheSettings::default());
    BmcHandle { name: cfg.name.clone(), bmc: Arc::new(bmc) }
}

pub async fn establish_session(bmc: &Arc<HttpBmc<ReqwestClient>>) -> Result<(), BmcError> {
    let root = nv_redfish::ServiceRoot::new(Arc::clone(bmc)).await
        .map_err(|e| BmcError::Session(format!("service root: {e}")))?;
    let Some(session_service) = root.session_service().await
        .map_err(|e| BmcError::Session(format!("session service: {e}")))?
    else { return Err(BmcError::Session("session service not available".into())); };
    let Some(sessions) = session_service.sessions().await
        .map_err(|e| BmcError::Session(format!("sessions: {e}")))?
    else { return Err(BmcError::Session("sessions collection not available".into())); };
    let (u, p) = current_credentials(bmc).ok_or_else(|| BmcError::Session("no credentials to create session".into()))?;
    let create = nv_redfish::schema::session::SessionCreate::builder(u, p).build();
    let session = sessions.create_session(&create).await
        .map_err(|e| BmcError::Session(format!("create session: {e}")))?;
    let Some(token) = session.auth_token() else {
        return Err(BmcError::Session("session response without auth token".into()));
    };
    bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(token.to_string()));
    Ok(())
}

fn current_credentials(bmc: &HttpBmc<ReqwestClient>) -> Option<(String, String)> {
    // HttpBmc 不暴露当前凭据读取；此辅助仅保留接口。Session 建立仅用于 auth=session 的 BMC：
    // 调用方在建立前以 basic 构造。直接返回 None 会导致无法使用——因此实现改为：
    // establish_session 的凭据来自调用方传入参数（见 Step 3b 说明）。
    None
}
```
**实现修正（必须在编码时落实）**：`HttpBmc` 不提供读取当前凭据的接口，`establish_session` 改为接收用户名/密码参数：

```rust
pub async fn establish_session(bmc: &Arc<HttpBmc<ReqwestClient>>,
    username: &str, password: &str) -> Result<(), BmcError>
```
（内部 `SessionCreate::builder(username, password)`；其余同上述逻辑。`current_credentials` 辅助删除。）

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test bmc_test`
Expected: 2 个测试 PASS（`rejects_missing_ca_file` 需要 `root_certificates` 字段存在于 `ClientParams`；若 0.15.1 无此字段，改为 `use_rust_tls=false` 报错路径或删除该测试并在 commit message 注明——以构建为准）

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/bmc.rs tests/bmc_test.rs && git commit -m "feat(bmc): BMC client construction with TLS options"`

---

### Task 3: 指标模型

**Files:**
- Create: `src/metrics.rs`、`tests/metrics_test.rs`
- Modify: 无

**Interfaces:**
- Consumes: 无
- Produces:
```rust
pub struct Metric { pub name: &'static str, pub help: &'static str,
    pub labels: Vec<(&'static str, String)>, pub value: f64 }
impl Metric { pub fn gauge(name: &'static str, help: &'static str) -> MetricBuilder }
pub struct MetricBuilder { name: &'static str, help: &'static str, labels: Vec<(&'static str, String)> }
impl MetricBuilder { pub fn label(mut self, k: &'static str, v: String) -> Self;
    pub fn build(self, value: f64) -> Metric }
pub fn register_into(metrics: &[Metric], registry: &prometheus::Registry) -> Result<(), MetricsError>
pub fn encode(registry: &prometheus::Registry) -> String   // TextEncoder 输出
pub enum MetricsError { Prometheus(#[from] prometheus::Error) }
// 转换辅助：
pub fn unbox_reading(v: Option<Option<f64>>) -> Option<f64>   // 双重 Option → Option
pub fn health_state_labels(health: Option<&str>, state: Option<&str>) -> (String, String)
// 常量（指标名/help 集中定义，collector 引用）：
pub const UP: (&str, &str) = ("redfish_up", "Whether the last scrape of this BMC succeeded");
pub const SCRAPE_DURATION: (&str, &str) = ("redfish_scrape_duration_seconds", "Duration of the last scrape of this BMC");
pub const SCRAPE_ERROR: (&str, &str) = ("redfish_scrape_error", "Set to 1 when the last scrape of a resource failed");
pub const HEALTH_STATUS: (&str, &str) = ("redfish_health_status", "Health and state of a resource");
pub const INFO: (&str, &str) = ("redfish_info", "Static key-value information about a BMC");
pub const SENSOR_READING: (&str, &str) = ("redfish_sensor_reading", "Sensor reading");
pub const THRESHOLD_PREFIX: &str = "redfish_sensor_threshold_";
pub const POWER_CONSUMPTION: (&str, &str) = ("redfish_power_consumption_watts", "Total chassis power consumption in watts");
pub const POWER_INPUT: (&str, &str) = ("redfish_power_input_watts", "Chassis power input in watts");
pub const POWER_STATE: (&str, &str) = ("redfish_power_state", "Power state of a system, 1 = On");
pub const PROCESSOR_UTILIZATION: (&str, &str) = ("redfish_processor_utilization_percent", "Processor utilization percentage");
pub const PROCESSOR_TEMPERATURE: (&str, &str) = ("redfish_processor_temperature_celsius", "Processor temperature in Celsius");
pub const PROCESSOR_POWER: (&str, &str) = ("redfish_processor_power_watts", "Processor power in watts");
pub const MEMORY_CAPACITY: (&str, &str) = ("redfish_memory_capacity_bytes", "Memory capacity in bytes");
pub const MEMORY_BANDWIDTH: (&str, &str) = ("redfish_memory_bandwidth_percent", "Memory bandwidth utilization percentage");
pub const VOLUME_CAPACITY: (&str, &str) = ("redfish_volume_capacity_bytes", "Volume capacity in bytes");
pub const DRIVE_CAPACITY: (&str, &str) = ("redfish_drive_capacity_bytes", "Drive capacity in bytes");
pub const DRIVE_UTILIZATION: (&str, &str) = ("redfish_drive_utilization_percent", "Drive utilization percentage");
pub const LINK_STATUS: (&str, &str) = ("redfish_ethernet_interface_link_status", "Ethernet link status, 1 = up");
pub const LINK_SPEED: (&str, &str) = ("redfish_ethernet_interface_speed_mbps", "Ethernet link speed in Mbps");
```

- [ ] **Step 1: 写失败测试** `tests/metrics_test.rs`

```rust
use redfish_exporter::metrics::{Metric, register_into, encode, unbox_reading, SENSOR_READING};

#[test]
fn builds_gauge_metric() {
    let m = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into())
        .label("name", "Ambient".into())
        .build(25.5);
    assert_eq!(m.name, "redfish_sensor_reading");
    assert_eq!(m.value, 25.5);
    assert_eq!(m.labels.len(), 2);
}

#[test]
fn unboxes_double_option() {
    assert_eq!(unbox_reading(Some(Some(12.0))), Some(12.0));
    assert_eq!(unbox_reading(Some(None)), None);
    assert_eq!(unbox_reading(None), None);
}

#[test]
fn register_and_encode_roundtrip() {
    let reg = prometheus::Registry::new();
    let m = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into()).label("name", "A".into()).build(1.0);
    register_into(&[m], &reg).unwrap();
    let out = encode(&reg);
    assert!(out.contains("redfish_sensor_reading"));
    assert!(out.contains("bmc=\"bmc1\""));
    assert!(out.contains("1"));
}

#[test]
fn duplicate_labels_are_merged() {
    // 同 name+label 集合并追加：register_into 对重复 name 使用 Counter/GaugeVec 合并
    let reg = prometheus::Registry::new();
    let m1 = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into()).label("name", "A".into()).build(1.0);
    let m2 = Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
        .label("bmc", "bmc1".into()).label("name", "A".into()).build(2.0);
    register_into(&[m1, m2], &reg).unwrap();
    let out = encode(&reg);
    assert_eq!(out.matches("redfish_sensor_reading{").count(), 1);
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test metrics_test`
Expected: 编译失败（模块不存在）

- [ ] **Step 3: 实现 metrics.rs**

```rust
#![allow(dead_code)] // 常量由后续 Task 消费；Task 9 完成后删除此行
use thiserror::Error;

pub const UP: (&str, &str) = ("redfish_up", "Whether the last scrape of this BMC succeeded");
pub const SCRAPE_DURATION: (&str, &str) = ("redfish_scrape_duration_seconds", "Duration of the last scrape of this BMC");
pub const SCRAPE_ERROR: (&str, &str) = ("redfish_scrape_error", "Set to 1 when the last scrape of a resource failed");
pub const HEALTH_STATUS: (&str, &str) = ("redfish_health_status", "Health and state of a resource");
pub const INFO: (&str, &str) = ("redfish_info", "Static key-value information about a BMC");
pub const SENSOR_READING: (&str, &str) = ("redfish_sensor_reading", "Sensor reading");
pub const THRESHOLD_PREFIX: &str = "redfish_sensor_threshold_";
pub const POWER_CONSUMPTION: (&str, &str) = ("redfish_power_consumption_watts", "Total chassis power consumption in watts");
pub const POWER_INPUT: (&str, &str) = ("redfish_power_input_watts", "Chassis power input in watts");
pub const POWER_STATE: (&str, &str) = ("redfish_power_state", "Power state of a system, 1 = On");
pub const PROCESSOR_UTILIZATION: (&str, &str) = ("redfish_processor_utilization_percent", "Processor utilization percentage");
pub const PROCESSOR_TEMPERATURE: (&str, &str) = ("redfish_processor_temperature_celsius", "Processor temperature in Celsius");
pub const PROCESSOR_POWER: (&str, &str) = ("redfish_processor_power_watts", "Processor power in watts");
pub const MEMORY_CAPACITY: (&str, &str) = ("redfish_memory_capacity_bytes", "Memory capacity in bytes");
pub const MEMORY_BANDWIDTH: (&str, &str) = ("redfish_memory_bandwidth_percent", "Memory bandwidth utilization percentage");
pub const VOLUME_CAPACITY: (&str, &str) = ("redfish_volume_capacity_bytes", "Volume capacity in bytes");
pub const DRIVE_CAPACITY: (&str, &str) = ("redfish_drive_capacity_bytes", "Drive capacity in bytes");
pub const DRIVE_UTILIZATION: (&str, &str) = ("redfish_drive_utilization_percent", "Drive utilization percentage");
pub const LINK_STATUS: (&str, &str) = ("redfish_ethernet_interface_link_status", "Ethernet link status, 1 = up");
pub const LINK_SPEED: (&str, &str) = ("redfish_ethernet_interface_speed_mbps", "Ethernet link speed in Mbps");

pub struct Metric {
    pub name: &'static str,
    pub help: &'static str,
    pub labels: Vec<(&'static str, String)>,
    pub value: f64,
}

pub struct MetricBuilder {
    name: &'static str, help: &'static str, labels: Vec<(&'static str, String)>,
}

impl Metric {
    pub fn gauge(name: &'static str, help: &'static str) -> MetricBuilder {
        MetricBuilder { name, help, labels: Vec::new() }
    }
}

impl MetricBuilder {
    pub fn label(mut self, key: &'static str, value: String) -> Self {
        self.labels.push((key, value)); self
    }
    pub fn build(self, value: f64) -> Metric {
        Metric { name: self.name, help: self.help, labels: self.labels, value }
    }
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("prometheus error: {0}")]
    Prometheus(#[from] prometheus::Error),
}

/// 把所有指标注册进 registry。同名+同 label 集的指标合并（GaugeVec）。
pub fn register_into(metrics: &[Metric], registry: &prometheus::Registry) -> Result<(), MetricsError> {
    use prometheus::{GaugeVec, Opts};
    use std::collections::HashMap;
    let mut by_name: HashMap<(&str, &str), (Vec<(&'static str, String)>, Opts)> = HashMap::new();
    for m in metrics {
        let (labels, opts) = by_name.entry((m.name, m.help)).or_insert_with(|| {
            (m.labels.clone(), Opts::new(m.name, m.help))
        });
        let mut label_names = labels.iter().map(|(k, _)| *k).collect::<Vec<_>>();
        label_names.sort();
        label_names.dedup();
        let gv = GaugeVec::new(opts.clone(), &label_names).unwrap();
        // 为每个指标创建标签值对
    }
    // 实现说明：prometheus crate 的 GaugeVec 需要预先声明 label 名集合。
    // 简化且可靠的做法——跳过 Vec 缓存，直接遍历：
    let mut vecs: HashMap<(&'static str, &'static str), prometheus::GaugeVec> = HashMap::new();
    for m in metrics {
        let mut names: Vec<&'static str> = m.labels.iter().map(|(k, _)| *k).collect();
        names.sort(); names.dedup();
        let gv = vecs.entry((m.name, m.help)).or_insert_with(|| {
            prometheus::GaugeVec::new(prometheus::Opts::new(m.name, m.help), &names).unwrap()
        });
        let mut label_values: Vec<String> = names.iter()
            .map(|n| m.labels.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default())
            .collect();
        let _ = &mut label_values;
        gv.with_label_values(&label_values).set(m.value);
    }
    for (_, gv) in vecs {
        registry.register(Box::new(gv)).map_err(|e| match e {
            prometheus::Error::AlreadyReg => prometheus::Error::AlreadyReg, // 已注册视为合并（重复 collect）
            other => other,
        })?;
    }
    Ok(())
}

pub fn encode(registry: &prometheus::Registry) -> String {
    use prometheus::TextEncoder;
    let mut buf = String::new();
    TextEncoder::new().encode_utf8(&registry.gather(), &mut buf)
        .expect("TextEncoder::encode_utf8 writes to String and cannot fail");
    buf
}

pub fn unbox_reading(v: Option<Option<f64>>) -> Option<f64> {
    v.flatten()
}

pub fn health_state_labels(health: Option<&str>, state: Option<&str>) -> (String, String) {
    (health.unwrap_or("unknown").to_string(), state.unwrap_or("unknown").to_string())
}
```

**实现注意事项（必须落实）**：
- `register_into` 的简化实现有缺陷（`HashMap` 遍历时 `prometheus::Error` 处理）。正确的最终实现：先收集所有 (name, help) 的 label 名集（排序去重），为每个 name 建一个 `GaugeVec`，再逐指标 `set`；`registry.register` 冲突（同名指标重复注册）时跳过（`AlreadyReg`）。以 `cargo test --test metrics_test` 与 `duplicate_labels_are_merged` 通过为准重构该函数，保持签名不变。
- 标签值必须与 `GaugeVec` 声明的 label 名序列对齐（排序后统一顺序），这是 `duplicate_labels_are_merged` 断言的基础。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test metrics_test`
Expected: 4 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/metrics.rs tests/metrics_test.rs && git commit -m "feat(metrics): metric model, registration and encoding"`

---

### Task 4: Collector 框架 + 传感器

**Files:**
- Create: `src/collector/mod.rs`、`src/collector/sensors.rs`、`tests/collector_sensors_test.rs`
- Modify: 无

**Interfaces:**
- Consumes: `metrics::{Metric, register_into, unbox_reading, ...}`；`nv_redfish`（ServiceRoot、Error、schema 类型）
- Produces（`collector/mod.rs`）：
```rust
pub mod sensors;
pub struct ScrapeReport { pub metrics: Vec<Metric>, pub failed_resources: Vec<String> }
pub async fn collect_all<B: Bmc>(bmc: &B, bmc_name: &str) -> Result<ScrapeReport, nv_redfish::Error<B>>
```
说明：`Bmc` trait 来自 `nv_redfish::bmc_http::Bmc`（core re-export；`nv_redfish::bmc_http::Bmc` 在 bmc-http feature 下可用）。`collect_all` 内部：`ServiceRoot::new(bmc)` → 依次调各 collector 函数（chassis/sensors/power/processors/memory/storage/network/systems），每个返回 `Result<Vec<Metric>, String>`（Err 记录 resource 名进 `failed_resources`，不中断其他）；最后附加 `redfish_up`（有任一失败记 0）+ `redfish_scrape_duration_seconds`。Task 4 先实现 `collect_all` 骨架（只挂 sensors），后续 Task 逐个挂接。

`collector/sensors.rs`：
```rust
pub async fn collect_chassis_sensors<B: Bmc>(bmc: &B, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
```
该函数遍历 `root.chassis()` → `members()` → 每 chassis：`sensor_links()` → `link.fetch()` → 读 `reading`/`reading_type`/`reading_units`/`thresholds`，产出：
- `redfish_sensor_reading{bmc, chassis, name, units, sensor_type, health}`（value = reading；无 reading 跳过；非数值读数跳过并记录）
- 阈值：`redfish_sensor_threshold_upper_critical{同 label}`、`_upper_warning`、`_lower_warning`、`_lower_critical`（仅存在的阈值；字段名以 Task 0 核对结果为准：`upper_critical`/`upper_caution`/`lower_caution`/`lower_critical`，其中 `caution` 即 warning）

- [ ] **Step 1: 写失败测试** `tests/collector_sensors_test.rs`

使用 bmc-mock。JSON 骨架基于 DMTF Sensor CSDL（字段名以 Task 0 核对为准）：

```rust
use nv_redfish::bmc_http::Bmc;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use nv_redfish::ServiceRoot;
use redfish_exporter::collector::sensors::collect_chassis_sensors;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn collects_sensor_readings_and_thresholds() {
    let bmc = MockBmc::default();
    let root_id = nv_redfish::ODataId::service_root();
    bmc.expect(Expect::get(root_id.clone(), json!({
        "Id": "Root", "Name": "Root",
        "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
    })));
    bmc.expect(Expect::expand("/redfish/v1/Chassis", json!({
        "Members": [
            { "@odata.id": "/redfish/v1/Chassis/1" },
        ],
        "Members@odata.count": 1,
    })));
    bmc.expect(Expect::get("/redfish/v1/Chassis/1", json!({
        "Id": "1", "Name": "Chassis 1",
        "Sensors": { "@odata.id": "/redfish/v1/Chassis/1/Sensors" },
    })));
    bmc.expect(Expect::get("/redfish/v1/Chassis/1/Sensors", json!({
        "Members": [
            { "@odata.id": "/redfish/v1/Chassis/1/Sensors/Ambient" },
        ],
    })));
    bmc.expect(Expect::get("/redfish/v1/Chassis/1/Sensors/Ambient", json!({
        "Id": "Ambient", "Name": "Ambient Temp",
        "Reading": 25.5,
        "ReadingType": "Temperature",
        "ReadingUnits": "Cel",
        "Status": { "Health": "OK", "State": "Enabled" },
        "Thresholds": {
            "UpperCritical": { "Reading": 60.0 },
            "UpperCaution": { "Reading": 50.0 },
        },
    })));

    let root = ServiceRoot::new(Arc::new(bmc)).await.unwrap();
    let metrics = collect_chassis_sensors(&root.bmc_ref(), &root, "bmc1").await.unwrap();
    let readings: Vec<_> = metrics.iter().filter(|m| m.name == "redfish_sensor_reading").collect();
    assert_eq!(readings.len(), 1);
    let r = &readings[0];
    assert_eq!(r.value, 25.5);
    let labels: std::collections::HashMap<_, _> = r.labels.iter().map(|(k, v)| (k, v.as_str())).collect();
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("name"), Some(&"Ambient"));
    assert_eq!(labels.get("units"), Some(&"Cel"));
    let thresholds: Vec<_> = metrics.iter()
        .filter(|m| m.name.starts_with("redfish_sensor_threshold_")).collect();
    assert_eq!(thresholds.len(), 2);
    assert!(metrics.iter().any(|m| m.name == "redfish_sensor_threshold_upper_critical"
        && m.value == 60.0));
    assert!(metrics.iter().any(|m| m.name == "redfish_sensor_threshold_upper_warning"
        && m.value == 50.0));
}

#[tokio::test]
async fn missing_sensor_collection_is_ok() {
    let bmc = MockBmc::default();
    let root_id = nv_redfish::ODataId::service_root();
    bmc.expect(Expect::get(root_id.clone(), json!({
        "Id": "Root", "Name": "Root",
        "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
    })));
    bmc.expect(Expect::expand("/redfish/v1/Chassis", json!({
        "Members": [{ "@odata.id": "/redfish/v1/Chassis/1" }],
    })));
    bmc.expect(Expect::get("/redfish/v1/Chassis/1", json!({
        "Id": "1", "Name": "Chassis 1",
    })));
    let root = ServiceRoot::new(Arc::new(bmc)).await.unwrap();
    let metrics = collect_chassis_sensors(&root.bmc_ref(), &root, "bmc1").await.unwrap();
    assert!(metrics.is_empty());
}
```
注意：`root.bmc_ref()` 不存在于 ServiceRoot——签名改为 `collect_chassis_sensors<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>, bmc_name: &str)`，测试传 `Arc::new(bmc)`。同理 `collect_all<B: Bmc>(bmc: Arc<B>, bmc_name: &str)`（内部自行 `ServiceRoot::new(Arc::clone(&bmc))`）。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test collector_sensors_test`
Expected: 编译失败（collector 模块不存在）

- [ ] **Step 3: 实现 collector/mod.rs 与 collector/sensors.rs**

`src/collector/mod.rs`（骨架，后续 Task 挂接）：
```rust
pub mod sensors;

use nv_redfish::bmc_http::Bmc;
use crate::metrics::{Metric, UP, SCRAPE_DURATION};
use std::sync::Arc;

pub struct ScrapeReport { pub metrics: Vec<Metric>, pub failed_resources: Vec<String> }

pub async fn collect_all<B: Bmc>(bmc: Arc<B>, bmc_name: &str) -> Result<ScrapeReport, nv_redfish::Error<B>> {
    let started = std::time::Instant::now();
    let root = nv_redfish::ServiceRoot::new(Arc::clone(&bmc)).await?;
    let mut metrics = Vec::new();
    let mut failed_resources = Vec::new();

    match sensors::collect_chassis_sensors(Arc::clone(&bmc), &root, bmc_name).await {
        Ok(m) => metrics.extend(m),
        Err(resource) => failed_resources.push(resource),
    }
    // 后续 Task 在此挂接其余 collector

    let up = if failed_resources.is_empty() { 1.0 } else { 0.0 };
    metrics.push(Metric::gauge(UP.0, UP.1).label("bmc", bmc_name.to_string()).build(up));
    metrics.push(Metric::gauge(SCRAPE_DURATION.0, SCRAPE_DURATION.1)
        .label("bmc", bmc_name.to_string())
        .build(started.elapsed().as_secs_f64()));
    Ok(ScrapeReport { metrics, failed_resources })
}
```

`src/collector/sensors.rs`：
```rust
use nv_redfish::bmc_http::Bmc;
use crate::metrics::{Metric, SENSOR_READING, THRESHOLD_PREFIX};
use std::sync::Arc;

pub async fn collect_chassis_sensors<B: Bmc>(bmc: Arc<B>, root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(chassis_collection) = root.chassis().await.map_err(|e| format!("chassis: {e}"))?
    else { return Ok(out) };
    let chassis_members = chassis_collection.members().await.map_err(|e| format!("chassis members: {e}"))?;
    for chassis in chassis_members {
        let chassis_id = chassis.id().to_string();
        let Some(links) = chassis.sensor_links().await.map_err(|e| format!("sensor links: {e}"))? else { continue };
        for link in links {
            let sensor = link.fetch().await.map_err(|e| format!("sensor fetch: {e}"))?;
            let name = sensor_base_name(&sensor);
            let Some(reading) = crate::metrics::unbox_reading(sensor.reading) else { continue };
            let units = sensor.reading_units.clone().flatten().unwrap_or_default();
            let sensor_type = sensor.reading_type.as_ref()
                .and_then(|t| t.as_ref()).map(|t| format!("{t:?}")).unwrap_or_default();
            let (health, state) = crate::metrics::health_state_labels(
                health_of(&sensor), state_of(&sensor));
            out.push(Metric::gauge(SENSOR_READING.0, SENSOR_READING.1)
                .label("bmc", bmc_name.to_string())
                .label("chassis", chassis_id.clone())
                .label("name", name)
                .label("units", units)
                .label("sensor_type", sensor_type)
                .label("health", health)
                .label("state", state)
                .build(reading));
            // 阈值：字段名以 Task 0 核对为准（upper_critical / upper_caution / lower_caution / lower_critical）
            push_threshold(&mut out, bmc_name, &chassis_id, &sensor, "upper_critical");
            push_threshold(&mut out, bmc_name, &chassis_id, &sensor, "upper_warning");
            push_threshold(&mut out, bmc_name, &chassis_id, &sensor, "lower_warning");
            push_threshold(&mut out, bmc_name, &chassis_id, &sensor, "lower_critical");
        }
    }
    Ok(out)
}
```
辅助函数 `sensor_base_name`、`health_of`、`state_of`、`push_threshold` 的实现依赖生成类型结构（`base.base.status` 链、`Thresholds` 字段名），在 Task 0 核对结果基础上实现；`push_threshold` 签名：
```rust
fn push_threshold(out: &mut Vec<Metric>, bmc_name: &str, chassis_id: &str,
    sensor: &SensorSchema, kind: &str) -> (/* 无返回值，内部 match kind 读对应阈值字段 */)
```
阈值 label 与 reading 完全一致（bmc/chassis/name/units/sensor_type/health/state），value 为阈值 `reading`。kind `upper_warning` 映射字段 `upper_caution`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test collector_sensors_test`
Expected: 2 个测试 PASS。若 JSON 字段与生成类型不符（如 Status 在 `base` 内、Thresholds 字段名不同），以 Task 0 核对结果修正 JSON 或代码。

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/collector/ tests/collector_sensors_test.rs && git commit -m "feat(collector): chassis sensor collection with thresholds"`

---

### Task 5: 电源/热/环境采集

**Files:**
- Create: `src/collector/power.rs`、`tests/collector_power_test.rs`
- Modify: `src/collector/mod.rs`（挂接 `power::collect_power_metrics`）

**Interfaces:**
- Consumes: `collector/mod.rs` 的 `ScrapeReport` 模式；Task 0 核对的 `ChassisSchema` 字段
- Produces:
```rust
pub async fn collect_power_metrics<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
```
采集内容：
1. **Legacy `Thermal`**（`chassis.thermal()` → `raw()`）：`Temperatures[]` 数组每项 → `redfish_sensor_reading{name=<MemberName>, units="Cel", sensor_type="Temperature", health, state}`（`reading_celsius`；阈值数组 `UpperThresholdCritical`/`UpperThresholdFatal` 等，取 critical 与 warning 字段，以 Task 0 核对为准）
2. **Legacy `Power`**（`chassis.power()` → `raw()`）：`PowerControl[]` → `redfish_power_consumption_watts{bmc, chassis}`（`power_consumed_watts`）；`PowerSupplies[]` → 每项 `power_output_watts` → `redfish_sensor_reading{sensor_type="Power", name=<MemberName>, units="W"}` + 健康
3. **`PowerSupplies`（新模型）**：`chassis.power_supplies()` → `members()` → `metrics()` → `metrics_sensor_links()` → fetch → 统一进 `redfish_sensor_reading`（输出/输入电压电流功耗温度风扇），`name` = `<PSU 名> <sensor 名>`，units 从 sensor 读取
4. **`EnvironmentMetrics`**：`chassis.environment_metrics()` → `sensor_links()` → 同样进 `redfish_sensor_reading`
5. **`Controls`**：`chassis.controls()` → `members()` → `raw()` 的读数（`set_point`/`reading` 回读，字段以核对为准，无则跳过）→ `redfish_sensor_reading{sensor_type="Control", units=...}`

测试（bmc-mock，legacy thermal + power 各一例）：
- `collects_legacy_thermal_readings`：Chassis JSON 带 `"Thermal": {"@odata.id": "/redfish/v1/Chassis/1/Thermal"}` → GET 返回 `{"Temperatures": [{"MemberId": "1", "Name": "CPU1", "ReadingCelsius": 42.0, "Status": {...}}]}` → 断言 `redfish_sensor_reading{name="CPU1", units="Cel", sensor_type="Temperature"}` value 42.0
- `collects_legacy_power_consumption`：Chassis JSON 带 `"Power": {...}` → GET 返回 `{"PowerControl": [{"Name": "Total", "PowerConsumedWatts": 320.0}]}` → 断言 `redfish_power_consumption_watts` 320.0
- `no_thermal_power_links_is_ok`：Chassis 无 Thermal/Power 链接 → 空结果

**Interfaces 补充**：`collect_power_metrics` 只在根下有 chassis 时工作；`mod.rs` 挂接位置在 sensors 之后：
```rust
match power::collect_power_metrics(Arc::clone(&bmc), &root, bmc_name).await {
    Ok(m) => metrics.extend(m),
    Err(resource) => failed_resources.push(resource),
}
```

- [ ] **Step 1: 写失败测试** `tests/collector_power_test.rs`（按上方接口写 3 个测试；JSON 骨架如上，字段名以 Task 0 核对为准）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test collector_power_test`
Expected: 编译失败（`collector::power` 不存在）

- [ ] **Step 3: 实现 collector/power.rs + mod.rs 挂接**

按上方接口实现。注意点：
- `power()`/`thermal()` 在 0.15.1 返回 `Option<Power<B>>`/`Option<Thermal<B>>`（legacy wrapper），`raw()` 返回 `Arc<PowerSchema>`/`Arc<ThermalSchema>`；字段数组直接 pub 访问
- 阈值字段（`UpperThresholdCritical` 等）类型为 `Option<Option<Threshold>>`，值为 `f64` 或 enum（`threshold_activation` 忽略，只取 `reading`）
- 所有 `redfish_sensor_reading` 保持与 Task 4 相同的 label 集（bmc/chassis/name/units/sensor_type/health/state）
- `chassis` label 一律用 `chassis.id().to_string()`

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test collector_power_test`
Expected: 3 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/collector/power.rs src/collector/mod.rs tests/collector_power_test.rs && git commit -m "feat(collector): thermal, power and environment metric collection"`

---

### Task 6: 处理器与内存采集

**Files:**
- Create: `src/collector/processors.rs`、`src/collector/memory.rs`、`tests/collector_proc_mem_test.rs`
- Modify: `src/collector/mod.rs`（挂接两个 collector）

**Interfaces:**
- Produces:
```rust
pub async fn collect_processors<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
pub async fn collect_memory<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
```
- `collect_processors`：`root.systems()` → `members()` → `system.processors()` → `members()` → `processor.metrics()`（`Option<ProcessorMetrics<B>>`）→ `raw()` 读 `utilization_percent` / `temperature_celsius` / `power_watts` → 三个指标（label：`bmc`、`system`、`id`）；同时每处理器产出 `redfish_health_status{resource_type="processor", id}`（Task 9 的通用 health 也做处理器——本 Task 由 processors collector 附带 health/state/info：`manufacturer`、`model` → `redfish_info{key, value}`）
- `collect_memory`：`system.memory_modules()` → `members()` → `memory.metrics()` → `raw()` 读 `bandwidth_percent`（`redfish_memory_bandwidth_percent`）；容量字段（`capacity_mi_b` × 1048576 或 `capacity_bytes`，以核对为准）→ `redfish_memory_capacity_bytes`；health/info 同处理器模式
- 无 metrics 时跳过数值指标，只产 health/info

测试（bmc-mock）：
- `collects_processor_metrics`：root → Systems 集合 → System 带 `"Processors": {...}` → Processors 集合 → Processor 带 `"Metrics": {"@odata.id": ...}` → ProcessorMetrics JSON `{"UtilizationPercent": 12.5, "TemperatureCelsius": 55.0, "PowerWatts": 60.0}` → 断言三个指标值与 label
- `collects_memory_metrics`：Memory 集合 → Memory 带 `"Metrics"` 链接 → `{"BandwidthPercent": 30.0}`；容量字段在 Memory JSON 本体（`CapacityMiB`）→ 断言 `redfish_memory_capacity_bytes` = MiB × 1048576
- `missing_metrics_links_produce_info_only`：无 Metrics 链接 → 无数值指标但有 health/info

- [ ] **Step 1: 写失败测试** `tests/collector_proc_mem_test.rs`（3 个测试，JSON 骨架如上，字段以 Task 0 核对为准）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test collector_proc_mem_test`
Expected: 编译失败

- [ ] **Step 3: 实现两个 collector + mod.rs 挂接**

按接口实现。`redfish_health_status` 的 labels：`bmc`、`resource_type`（`"processor"`/`"memory"`）、`id`（`memory.id()`/`processor.id()` 或成员名）、`health`、`state`，value=1。`redfish_info`：labels `bmc`、`key`、`value`，value=1。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test collector_proc_mem_test`
Expected: 3 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/collector/ tests/collector_proc_mem_test.rs && git commit -m "feat(collector): processor and memory metric collection"`

---

### Task 7: 存储采集

**Files:**
- Create: `src/collector/storage.rs`、`tests/collector_storage_test.rs`
- Modify: `src/collector/mod.rs`（挂接）

**Interfaces:**
- Produces:
```rust
pub async fn collect_storage<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
```
遍历：`root.systems()` → `members()` → `system.storage_controllers()`（`Result<Vec<Storage<B>>>`）→ 每 Storage：
- `storage.raw()` 读 `storage_controllers[]`（数组，`StorageController`）→ `redfish_health_status{resource_type="storage_controller", id}` + info（`manufacturer`/`model`/`firmware_version`，存在则加）
- `storage.drives()` → `Option<Vec<Drive<B>>>` → 每 Drive：
  - `raw()` 读容量字段（`capacity_bytes` 或 `capacity_mi_b`，以核对为准）→ `redfish_drive_capacity_bytes{bmc, system, storage, id}`
  - `raw()` 读 `predicted_media_life_left_percent`（存在则 `redfish_drive_utilization_percent`？不——该字段独立：`redfish_drive_life_left_percent`？**简化决策**：`predicted_media_life_left_percent` 产出为 `redfish_drive_utilization_percent`（磨损语义，文档注明）与 `failure_predicted` → `redfish_drive_predictive_failure{bmc, id}` 1/0。以 Task 0 核对的实际字段名为准，字段不存在则跳过该指标）
  - `drive.metrics()` → `raw()` 读 `utilization_percent`（DriveMetrics 存在时）→ `redfish_drive_utilization_percent`
  - health/info
- Volume：`storage.raw()` 的 `volumes` 导航（存在时）→ fetch → `capacity_bytes`/`allocated_bytes` → `redfish_volume_capacity_bytes{bmc, system, storage, id}`；**0.1.0 简化**：Volume 仅在 storage raw 含 `Volumes` 导航时遍历，字段以核对为准

测试（bmc-mock）：
- `collects_drive_metrics`：System → Storage 集合（`storage_controllers()` 路径：System JSON `"Storage": {"@odata.id": "/redfish/v1/Systems/1/Storage"}` → GET 返回 Storage 数组 JSON `{"Members": [...]}`，Storage JSON 含 `"Drives": [...]` 导航数组 → 逐 drive GET：`{"CapacityBytes": 1024, "PredictedMediaLifeLeftPercent": 80, "FailurePredicted": false, "Status": {...}}` + drive `"Metrics"` GET `{"UtilizationPercent": 10.0}`）→ 断言 `redfish_drive_capacity_bytes`、`redfish_drive_utilization_percent`（80.0，来自 life left——若字段语义不符以实际字段调整）、`redfish_drive_predictive_failure` = 0
- `no_storage_is_ok`：System 无 Storage → 空
（注意：`storage_controllers()` 的实现细节——它返回 `Vec<Storage<B>>`，遍历成员；以 v0.15.1 实际行为为准调整 JSON 序列。）

- [ ] **Step 1: 写失败测试** `tests/collector_storage_test.rs`

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test collector_storage_test`
Expected: 编译失败

- [ ] **Step 3: 实现 collector/storage.rs + mod.rs 挂接**

按接口实现。Storage 的 `drives()` 与 `storage_controllers()` 返回类型以 Task 0 核对为准；`Drive` 的 `metrics()` 返回 `Result<Option<Arc<DriveMetrics>>>`（`Arc` 而非 wrapper——直接访问字段）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test collector_storage_test`
Expected: 2 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/collector/storage.rs src/collector/mod.rs tests/collector_storage_test.rs && git commit -m "feat(collector): storage, drive and volume metric collection"`

---

### Task 8: 网络与 PCIe 采集

**Files:**
- Create: `src/collector/network.rs`、`tests/collector_network_test.rs`
- Modify: `src/collector/mod.rs`（挂接）

**Interfaces:**
- Produces:
```rust
pub async fn collect_network<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
```
遍历：
- Ethernet：`system.ethernet_interfaces()` → `members()` → `link_status()`（`Option<LinkStatus>`，枚举：`Up`/`Down` 等，以核对为准）→ `redfish_ethernet_interface_link_status{bmc, system, id}` 1/0；`raw()` 的 `speed_mbps` → `redfish_ethernet_interface_speed_mbps`；health/info（mac 地址进 info）
- NetworkAdapter：`chassis.network_adapters()` → `members()` → health/info（manufacturer/model/firmware）
- Port：`adapter.ports()` → `members()` → health（`resource_type="port"`）
- PCIeDevice：`chassis.pcie_devices()` → `members()` → `status()`（`ResourceProvidesStatus`）→ health；`raw()` 链路速率字段（核对）→ `redfish_pcie_device_link_speed_gt_per_sec`？**简化决策**：0.1.0 只做 health + info，链路速率字段若存在也产出（命名 `redfish_pcie_device_link_speed_gt_per_sec`）

测试（bmc-mock）：
- `collects_ethernet_link_status`：System → EthernetInterfaces 集合 → 接口 JSON `{"LinkStatus": "Up", "SpeedMbps": 1000, "Status": {...}, "MACAddress": "00:11:22:33:44:55"}` → 断言 link_status=1、speed=1000、info 含 mac
- `collects_pcie_health`：Chassis → PCIeDevices 集合 → `{"Status": {"Health": "OK", "State": "Enabled"}}` → 断言 `redfish_health_status{resource_type="pcie_device", health="OK"}` = 1

- [ ] **Step 1: 写失败测试** `tests/collector_network_test.rs`

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test collector_network_test`
Expected: 编译失败

- [ ] **Step 3: 实现 collector/network.rs + mod.rs 挂接**

按接口实现。`LinkStatus` 枚举 variant 以核对为准（如 `Up`/`Down`/`NotPresent`/`Unknown`）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test collector_network_test`
Expected: 2 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/collector/network.rs src/collector/mod.rs tests/collector_network_test.rs && git commit -m "feat(collector): network and PCIe metric collection"`

---

### Task 9: 系统/机箱/管理器/固件 + 通用健康

**Files:**
- Create: `src/collector/systems.rs`、`tests/collector_systems_test.rs`
- Modify: `src/collector/mod.rs`（挂接全部）

**Interfaces:**
- Produces:
```rust
pub async fn collect_systems<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
pub async fn collect_chassis_health<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
pub async fn collect_managers<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
pub async fn collect_assembly<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
pub async fn collect_firmware<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>,
    bmc_name: &str) -> Result<Vec<Metric>, String>
```
内容：
- `collect_systems`：`root.systems()` → `members()` → `power_state()`（`Option<PowerState>`，`On`→1）→ `redfish_power_state{bmc, system}`；health（`resource_type="system"`）；info：manufacturer/model/serial_number/sku
- `collect_chassis_health`：chassis 健康（`resource_type="chassis"`）+ info（manufacturer/model/serial/part_number）+ `redfish_power_consumption_watts` 已在 Task 5 处理，此处不重复
- `collect_managers`：`root.managers()` → `members()` → health（`resource_type="manager"`）+ info（manufacturer/model/firmware_version、`manager.raw()` 的 `firmware_version`/`model`，核对字段名）
- `collect_assembly`：`chassis.assembly()` → `assemblies()` → 每项 health + info（`producer`/`model`/`part_number`/`serial_number`，`resource_type="assembly"`）
- `collect_firmware`：`root.update_service()` → `firmware_inventories()`/`software_inventories()` → 每项：`redfish_info{key="firmware_version", value=version}` + health（`resource_type="software_inventory"`）

health/state 读取辅助（Task 0 核对后实现于 `collector/mod.rs`）：
```rust
pub(crate) fn status_of<T>(raw: &T) -> Option<&nv_redfish::schema::resource::Status> // 依 base 链
pub(crate) fn push_health(out: &mut Vec<Metric>, bmc: &str, resource_type: &str,
    id: &str, raw_status: Option<(&str, &str)>)
pub(crate) fn push_info(out: &mut Vec<Metric>, bmc: &str, key: &str, value: &str)
```
（`status_of` 需按类型实现或按核对结果用具体路径访问；若各类型 base 链一致则用通用 helper，否则逐类型写小函数。）

测试（bmc-mock）：
- `collects_system_power_and_health`：System JSON `{"PowerState": "On", "Manufacturer": "Dell", "Model": "R750", "SerialNumber": "SN1", "Status": {...}}` → 断言 `redfish_power_state`=1、health_status、info 条目
- `collects_firmware_inventory`：UpdateService → FirmwareInventory 集合 → `{"Version": "1.2.3", "Status": {...}}` → 断言 info 与 health
- `missing_services_are_ok`：根无 UpdateService/Managers 链接 → 空

- [ ] **Step 1: 写失败测试** `tests/collector_systems_test.rs`

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test collector_systems_test`
Expected: 编译失败

- [ ] **Step 3: 实现 systems.rs + 通用辅助 + mod.rs 挂接**

按接口实现。`update_service()` 在根 JSON 无 `"UpdateService"` 键时返回 `Ok(None)`（已验证 v0.15.1 行为：`Option` 导航）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test collector_systems_test`
Expected: 3 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
（删除 `metrics.rs` 中 `#![allow(dead_code)]`，此时所有常量已消费）
Commit: `git add src/collector/ src/metrics.rs tests/collector_systems_test.rs && git commit -m "feat(collector): system, chassis, manager, assembly and firmware collection"`

---

### Task 10: Scraper 与快照

**Files:**
- Create: `src/scraper.rs`、`src/registry.rs`、`tests/scraper_test.rs`
- Modify: 无

**Interfaces:**
- Consumes: `config::Config`、`bmc::{BmcHandle, establish_session, build_http_client, make_bmc}`、`collector::collect_all`
- Produces:
```rust
// registry.rs
pub struct Snapshot { inner: RwLock<Option<Arc<prometheus::Registry>>> }
impl Snapshot {
    pub fn new() -> Self
    pub fn update(&self, registry: Arc<prometheus::Registry>)  // 原子替换
    pub fn registry(&self) -> Option<Arc<prometheus::Registry>>  // 读快照
}
pub async fn build_registry(report: &ScrapeReport) -> Result<Arc<prometheus::Registry>, MetricsError>
// scraper.rs
pub struct Scraper { bmcs: Vec<BmcHandle>, interval: Duration, timeout: Duration,
    snapshot: Arc<Snapshot> }
impl Scraper {
    pub fn new(cfg: &Config, snapshot: Arc<Snapshot>) -> Result<Self, anyhow::Error>
    pub fn run(self) -> tokio::task::JoinHandle<()>   // 后台循环，返回句柄
}
```
`run` 逻辑：`interval` 循环（`tokio::time::interval`）；每轮：
1. `tokio::time::timeout(timeout, async { JoinSet 并发 collect_all })`——超时则本轮作废（记录日志，快照不更新）
2. 每个 BMC 的结果 → `build_registry` → `snapshot.update`（任一 BMC 失败仍有其余快照；BMC 整体失败 → 构造只含 `redfish_up=0` + `redfish_scrape_error{resource="bmc"}` 的 registry 更新）
3. `auth: session` 的 BMC：`establish_session` 在 `Scraper::new` 中先执行一次（阻塞等待失败 → `anyhow!` 返回，main 层决定退出）；401 重登逻辑：Task 12 集成测试补（0.1.0 先做初次建立 + 失败记 redfish_up=0）

shutdown：`run` 每次循环开始前检查 `tokio::sync::watch` 的停止信号（`main` 持有 sender）；收到后完成当前轮（受 timeout 限制）并返回。**实现**：`run(self, mut stop: watch::Receiver<bool>)`，`select!` 于 `stop.changed()` 与 `interval.tick()`。

- [ ] **Step 1: 写失败测试** `tests/scraper_test.rs`

单元测试（不依赖真实 HTTP）：
```rust
use redfish_exporter::registry::{Snapshot, build_registry};
use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::metrics::Metric;

#[test]
fn snapshot_atomic_update_and_read() {
    let snap = Snapshot::new();
    assert!(snap.registry().is_none());
    let reg = build_registry(&ScrapeReport { metrics: vec![
        Metric::gauge("redfish_up", "up").label("bmc", "b".into()).build(1.0),
    ], failed_resources: vec![] }).unwrap();
    snap.update(reg.clone());
    assert!(Arc::ptr_eq(&snap.registry().unwrap(), &reg));
}

#[test]
fn build_registry_includes_error_metrics() {
    let reg = build_registry(&ScrapeReport { metrics: vec![], failed_resources: vec!["sensors".into()] }).unwrap();
    let out = redfish_exporter::metrics::encode(&reg);
    assert!(out.contains("redfish_up") && out.contains("0"));
    assert!(out.contains("redfish_scrape_error") && out.contains("sensors"));
}
```
（`ScrapeReport` 构造需 `failed_resources`；`build_registry` 在 report 基础上追加 `redfish_scrape_error{bmc, resource}=1` 并保证 `redfish_up` 已存在。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test scraper_test`
Expected: 编译失败

- [ ] **Step 3: 实现 registry.rs 与 scraper.rs**

按接口实现。`build_registry`：`prometheus::Registry::new()` + `register_into`；对每个 failed resource 追加 `Metric::gauge(SCRAPE_ERROR.0, SCRAPE_ERROR.1).label("bmc", name).label("resource", res).build(1.0)`。`Scraper::new` 中：逐 BMC `build_http_client` + `make_bmc`；`auth == AuthMethod::Session` 时 `establish_session`（返回 Err → `anyhow::bail!` 带 BMC 名）。`run` 用 `watch` 停止信号（`interval.tick()` 与 `stop.changed()` 之间 `select!`；停止时若当前轮未开始则直接返回）。每轮结束后记录 tracing：`info!(bmc_count, duration_ms, failed)`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test scraper_test`
Expected: 2 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/scraper.rs src/registry.rs tests/scraper_test.rs && git commit -m "feat(scraper): periodic scraping with snapshot registry"`

---

### Task 11: HTTP 服务与 main

**Files:**
- Create: `src/http.rs`、`tests/http_test.rs`
- Modify: `src/main.rs`（完整实现）

**Interfaces:**
- Consumes: `registry::Snapshot`、`config::Config`
- Produces:
```rust
pub fn router(snapshot: Arc<Snapshot>) -> axum::Router
pub async fn serve(cfg: &Config, snapshot: Arc<Snapshot>) -> anyhow::Result<()>
```
路由：`GET /metrics` → 200 + `encode(registry)`（无快照 → 空 body + `X-Redfish-Exporter: no-data-yet` 头，仍 200）；`GET /healthz` → `200 "ok"`。`serve` 绑定 `cfg.listen_addr` 并 `axum::serve(...).with_graceful_shutdown(stop_signal)`（`tokio::signal::ctrl_c()` 与 SIGTERM 双路）。

`main.rs` 装配：
1. clap 解析：`-c/--config`（默认 `config.yaml`）、`-p/--port`（覆盖 listen_addr 端口，可选）、`--log-level`（默认 `info`，经 `EnvFilter`）
2. tracing_subscriber 初始化（fmt + EnvFilter，`RUST_LOG` 环境变量优先）
3. `load_config` → `Config`（错误 → `anyhow!` 退出码 1）
4. `Snapshot::new()`；`Scraper::new(&cfg, snapshot)`（session 建立失败 → 退出码 1）
5. `http::serve` 与 `scraper.run(stop_rx)` 并行（`tokio::join!`）；`watch` channel：`ctrl_c`/SIGTERM → send(true) → scraper 停止 → serve graceful 关闭 → 正常退出（码 0）
6. 端口覆盖：`-p` 存在时用 `SocketAddr::new(listen_addr.ip(), port)`

- [ ] **Step 1: 写失败测试** `tests/http_test.rs`

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use redfish_exporter::http::router;
use redfish_exporter::registry::Snapshot;
use redfish_exporter::collector::ScrapeReport;
use redfish_exporter::metrics::Metric;
use std::sync::Arc;

#[tokio::test]
async fn metrics_endpoint_returns_cached_snapshot() {
    let snap = Arc::new(Snapshot::new());
    let reg = redfish_exporter::registry::build_registry(&ScrapeReport {
        metrics: vec![Metric::gauge("redfish_up", "up").label("bmc", "b1".into()).build(1.0)],
        failed_resources: vec![] }).unwrap();
    snap.update(reg);
    let resp = router(snap).oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.contains("redfish_up"));
}

#[tokio::test]
async fn healthz_returns_ok() {
    let snap = Arc::new(Snapshot::new());
    let resp = router(snap).oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
```
（需 dev-dependencies：`http-body-util`、`tower`、`axum` 的 `http1` 支持——`router().oneshot` 需要 `tower::ServiceExt`；若 axum 0.8 不直接暴露，改用 `axum::serve` 配 `tokio::net::TcpListener` 于随机端口测试。以实际 API 为准，测试目的不变。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test http_test`
Expected: 编译失败

- [ ] **Step 3: 实现 http.rs 与 main.rs**

按接口实现。`serve` 的 graceful shutdown 信号：`axum::serve(listener, router).with_graceful_shutdown(async move { let _ = stop_rx.changed().await; })`——stop 由 main 的 watch sender 控制。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test http_test`
Expected: 2 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add src/http.rs src/main.rs tests/http_test.rs && git commit -m "feat(http): metrics and healthz endpoints with graceful shutdown"`

---

### Task 12: 集成测试与行为验证

**Files:**
- Create: `tests/integration_test.rs`
- Modify: 无

**Interfaces:**
- Consumes: 全部既有接口

- [ ] **Step 1: 写集成测试** `tests/integration_test.rs`

1. `full_scrape_cycle_with_mock_bmc`：用一个 `MockBmc`（含 service root + chassis + 2 sensors + system + processor 的最小 JSON 集），经 `collect_all` 跑一轮 → 断言 output registry 编码后包含 `redfish_sensor_reading`、`redfish_processor_utilization_percent`、`redfish_up{...} 1`、`redfish_health_status`、`redfish_info`
2. `bmc_failure_isolated`：两个 MockBmc，第二个预期序列在 GET service root 即返回 UnexpectedGet 错误（构造预期为 `Expect::get(root_id, json)` 后不再匹配 → `bmc.expect` 队列耗尽即报错——用 `Bmc::debug_expect()` 辅助调试）；`collect_all` 对失败的返回 `Err`；对成功的返回正常指标。断言：失败方 `Err`、成功方正常（模拟 scraper 中单 BMC 失败不影响其他——scraper 层隔离通过 `JoinSet` 逐任务 Result 实现，测试验证 `collect_all` 自身错误传播）
3. `session_auth_flow`：MockBmc 走 `establish_session` 路径：预期序列 = service root GET（含 `"SessionService"` 链接）→ session service GET → Sessions GET → create_session POST → 断言 `bmc` 内凭据已换为 token（mock 不支持读取凭据时，退化为验证 `establish_session` 不报错 + `SessionCreate` JSON 匹配 mock 预期）

JSON 规模控制在每个资源 1 条。字段名一律以 Task 0 核对结果为准。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test integration_test`
Expected: 编译失败（无此文件则直接进入 Step 3 后验证）

- [ ] **Step 3: 按接口实现测试体**

无生产代码改动（若发现 bug，按 bug 修复并更新对应模块测试后重新门禁）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test integration_test`
Expected: 3 个测试 PASS

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add tests/integration_test.rs && git commit -m "test: end-to-end scrape, isolation and session flow"`

---

### Task 13: 文档与部署样例

**Files:**
- Create: `README.md`、`docs/metrics.md`、`docs/design.md`、`config.example.yaml`、`deploy/prometheus/redfish-alerts.yml`、`deploy/grafana/redfish-dashboard.json`、`deploy/kubernetes/deployment.yaml`、`deploy/kubernetes/service-monitor.yaml`
- Modify: 无

- [ ] **Step 1: 编写 README.md**

内容：项目简介（nv-redfish 基础、多 BMC、周期采集+缓存架构一句话）、快速开始（`cargo run -- -c config.yaml`）、Docker 运行示例、配置说明表（对照 config.example.yaml 每键含义）、指标表概览（链接 docs/metrics.md）、认证说明（basic/session）、告警规则引用、安全说明（secret 脱敏、TLS、非 root）、开发（测试/门禁命令）、License（Apache-2.0）。

- [ ] **Step 2: 编写 docs/metrics.md**

A 类完整指标表：指标名、类型（Gauge）、labels、help、来源资源；B 类记录：log-services/event-service/task-service/accounts/session-service/bios/boot-options/secure-boot/host-interfaces/manager-network-protocol + 全部 OEM——每项一行说明"为何不实现（0.1.0）"与后续方向。

- [ ] **Step 3: 编写 docs/design.md**

架构图（ASCII）、组件职责、数据流、错误模型（BMC 级/资源级隔离）、生命周期与 shutdown、认证流程、配置校验规则、指标命名约定（单位进名或 label 的规则）。

- [ ] **Step 4: 编写 config.example.yaml 与 deploy/ 样例**

config.example.yaml：全字段 + 注释（含 insecure_skip_verify 警示）。deploy/prometheus/redfish-alerts.yml：redfish_up==0、redfish_sensor_reading 超阈值（示例）、redfish_health_status!=OK。deploy/grafana/redfish-dashboard.json：单面板温度/功耗/利用率基础 dashboard（Gauge 面板即可）。deploy/kubernetes/：Deployment（非 root、readOnlyRootFilesystem、resource 限额、探针 /healthz）+ ServiceMonitor（matchLabels 对齐）。

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add README.md docs/ config.example.yaml deploy/ && git commit -m "docs: README, metrics reference, design doc and deploy samples"`

---

### Task 14: CI 与静态单二进制发布

**Files:**
- Create: `.github/workflows/ci.yml`、`.github/workflows/release.yml`、`Dockerfile`、`.dockerignore`、`docker/entrypoint`（如需要，非 root 无需 entrypoint）
- Modify: 无

**约束：** 产物为纯静态单二进制（Linux），无 OpenSSL 依赖（rustls 已默认）。

- [ ] **Step 1: 编写 .github/workflows/ci.yml**

```yaml
name: CI
on: [push, pull_request]
jobs:
  lint-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { toolchain: 1.90.0, components: clippy, rustfmt }
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test --all-targets
      - run: cargo doc --no-deps
      - uses: cargo-bins/cargo-binstall@main
      - run: cargo binstall -y cargo-geiger
      - run: cargo geiger --all-targets  # 断言 unsafe 数量 0；允许 exit code 处理（geiger 对无 unsafe 输出 "0"）
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install cargo-audit --locked
      - run: cargo audit
      - uses: EmbarkStudios/cargo-deny-action@v2   # license + advisory + sources 默认配置
```
（`cargo geiger` 对 `#![forbid(unsafe_code)]` 的 crate 输出 0 unsafe；若 geiger 对依赖树报告非零（来自依赖），则 ci 中增加显式 `cargo geiger` 输出检查步骤并只对第一方断言——实施时以 geiger 实际输出为准调整，目标：第一方零 unsafe 且记录依赖中 unsafe 清单到 docs/design.md 安全章节。）

- [ ] **Step 2: 编写 Dockerfile（多阶段，静态 musl）**

```dockerfile
# syntax=docker/dockerfile:1
FROM rust:1.90.0-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/redfish-exporter /redfish-exporter
EXPOSE 9417
ENTRYPOINT ["/redfish-exporter"]
```
补充：`.dockerignore`（target、.git、docs、tests、deploy）。镜像默认非 root（distroless nonroot，UID 65532）。

- [ ] **Step 3: 编写 .github/workflows/release.yml**

`on: push: tags: ['v*']`：构建 musl 静态二进制（`cross` 或上述 alpine builder 步骤）+ `sha256sum` + GitHub Release assets（二进制 tarball + SHA256SUMS）。Docker 镜像构建推送可选（示例：ghcr.io 推送，`permissions: packages: write`），0.1.0 至少产出二进制附件。

- [ ] **Step 4: 本地验证静态链接**

在 Linux 环境（WSL 或 Docker alpine 容器）执行：
```
file target/x86_64-unknown-linux-musl/release/redfish-exporter   # 期望 "statically linked"
ldd target/x86_64-unknown-linux-musl/release/redfish-exporter    # 期望 "not a dynamic executable"
```
Windows 本地可只验证 `cargo build --release` 通过。记录验证结果到 docs/design.md（发布章节）。

- [ ] **Step 5: 质量门禁 + Commit**

Run: `cargo fmt --check && cargo clippy -D warnings && cargo test`
Commit: `git add .github/ Dockerfile .dockerignore && git commit -m "ci: lint, audit, static musl build and release pipeline"`

---

### Task 15: 收尾验收

**Files:**
- Modify: 无（仅验证与修补）

- [ ] **Step 1: 全量门禁**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && cargo doc --no-deps`
Expected: 全绿

- [ ] **Step 2: 规范自查**

对照《Rust 工程开发规范》与本计划 Global Constraints 逐条核对代码：
- `#![forbid(unsafe_code)]` 生效（grep 确认无 unsafe 关键字）
- 无 unwrap/expect 于外部输入路径（grep `unwrap()` / `expect(` 逐一复核；`expect` 仅限不变量且带说明）
- 无 `sleep` 用于测试同步（grep `sleep`）
- 密码不经 Debug/日志（grep `password` 于 tracing/Debug 路径）
- 指标 label 集合有限（对照 docs/metrics.md 检查 collector）
- 错误均带上下文（`{e}` 或 `with_context`）

- [ ] **Step 3: 冒烟验证**

`cargo run --release -- --help` 输出正确；`cargo run --release -- -c tests/fixtures/minimal-config.yaml`（创建最小配置指向不可达 IP，端口随机）→ 进程启动、/healthz 200、/metrics 200（含 redfish_up=0 或 no-data 头）、Ctrl-C 优雅退出码 0。

- [ ] **Step 4: 最终提交**

若修补了问题：`git add -A && git commit -m "chore: final polish for 0.1.0"`；否则无提交。

- [ ] **Step 5: 验收清单**

对照 Spec §13 逐项打勾：clippy/test/fmt 全绿；零 unsafe；集成测试覆盖主要资源与错误路径；静态链接验证记录；Docker 非 root；文档齐全；无 panic 路径。
