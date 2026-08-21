# 安全加固专项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 0.1.0 安全专项——分层可配置安全模型（默认安全基线 + 可选 Bearer 认证 + 可选自身 TLS + 出站/供应链加固）。

**Architecture:** 全部为增量层改动：配置校验与 `web` 节（config.rs）、常量时间 Bearer 认证（新增 src/auth.rs + axum middleware）、axum-server 承载 rustls TLS 与 header read timeout（http.rs serve 改造）、reqwest 重定向降级阻断（bmc.rs）、分页防御上限（pagination.rs 错误类型化）、供应链白名单（.cargo/audit.toml + deny.toml）、文档与验收。

**Tech Stack:** Rust 1.90 / edition 2024、axum 0.8、axum-server 0.7（tls-rustls）、reqwest 0.12（rustls-tls）、zeroize 1、rcgen 0.13（dev）、nv-redfish 0.15.1。

**Spec:** `docs/superpowers/specs/2026-08-21-security-hardening-design.md`（计划依 spec 论证，执行者须先读 spec）

## Global Constraints

- rust-version 1.90、edition 2024；`#![forbid(unsafe_code)]` 在 src/（lib.rs、main.rs）必须保持；测试中仅 Task 2 的 zeroize 验证允许带注释的 unsafe 块（产品代码零 unsafe）
- 零 OpenSSL/native-tls：新依赖仅 `zeroize = "1"`、`axum-server = { version = "0.7", default-features = false, features = ["tls-rustls"] }`（运行时）、`rcgen = "0.13"`（dev）
- 每任务结束必须 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets` 全绿后才提交（PowerShell 中 clippy 写法：`cargo clippy --all-targets -- -D warnings`，注意 `--`）
- 代码注释用中文（仓库惯例）；提交信息用英文、沿用仓库风格（`feat:`/`fix:`/`docs:`/`chore:`）
- 安全语义冻结（spec §9.3）：`web` 节配置语义、认证/TLS 行为、SecretString/zeroize、host userinfo 拒绝、重定向降级阻断、分页上限、name 校验——只增不改
- 精确值（spec 逐字）：token ≥16 字符；header read timeout 10s；分页单页上限 64MiB、页数 1000、累计成员 200_000；默认监听 `127.0.0.1:9417`；env 变量 `REDFISH_EXPORTER_PASSWORD_<NAME>`（name 大写、非 `[A-Z0-9]` 替换为 `_`）；重定向上限 10 跳

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `src/config.rs` | 改 | host userinfo 拒绝、name 控制字符校验、默认绑定收紧、`WebConfig`/`web` 节、env 覆盖（`load_config_with_env`）、文件权限检查（unix）、SecretString zeroize |
| `src/auth.rs` | 新建 | 常量时间比较 + Bearer 头解析（纯函数，可单测） |
| `src/http.rs` | 改 | AppState 增加 auth_token、认证 middleware、serve 改造（axum-server + TLS + header timeout + `serve_on` 测试接缝） |
| `src/main.rs` | 改 | 配置文件权限 warn（unix） |
| `src/bmc.rs` | 改 | 重定向降级阻断策略 |
| `src/pagination.rs` | 改 | `PaginationError` 错误类型 + 三个防御上限 + `fetch_all_pages_with_limits` |
| `src/collector/logs.rs` | 改 | 分页错误可见化（warn + 跳过该日志服务） |
| `src/lib.rs` | 改 | 注册 `auth` 模块 |
| `Cargo.toml` | 改 | zeroize、axum-server（tls-rustls）、rcgen（dev） |
| `.cargo/audit.toml` | 新建 | RUSTSEC-2026-0194/0195 白名单 + 理由 |
| `deny.toml` | 新建 | advisories/bans/licenses/sources 四类策略 |
| `.github/workflows/ci.yml` | 改 | audit job 注释指向 audit.toml 理由 |
| `tests/config_test.rs` | 改 | Task 1/2/3/4/8 测试 |
| `tests/http_test.rs` | 改 | Task 4 认证测试 + Task 5 TLS/超时测试 |
| `tests/bmc_test.rs` | 改 | Task 6 重定向策略测试 |
| `tests/pagination_test.rs` | 改 | Task 7 上限测试 |
| `docs/security.md` | 新建 | 威胁模型 + 基线矩阵 + 运维说明 |
| `README.md` / `config.example.yaml` / `docs/design.md` / `deploy/kubernetes/deployment.yaml` | 改 | Task 10 文档同步 |

---

### Task 1: 配置校验加固（host userinfo 拒绝 / name 控制字符 / 默认绑定 127.0.0.1）

**Files:**
- Modify: `src/config.rs:108-110`（default_listen_addr）、`:148-171`（bmc 循环校验）
- Test: `tests/config_test.rs`（追加）

**Interfaces:**
- Produces: `load_config` 对含 userinfo 的 host、含控制字符的 name 返回 `Err(ConfigError::Invalid)`；默认 `listen_addr` = `127.0.0.1:9417`

- [ ] **Step 1: 写失败测试**（追加到 `tests/config_test.rs` 末尾）

```rust
#[test]
fn rejects_host_with_userinfo() {
    let p = write_tmp(
        "userinfo",
        r#"
bmcs:
  - { name: a, host: https://user:secret@h1, username: u, password: "p" }
"#,
    );
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn rejects_control_chars_in_bmc_name() {
    let p = write_tmp(
        "ctrl_name",
        "bmcs:\n  - { name: \"a\\nb\", host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn default_listen_addr_is_localhost() {
    let p = write_tmp(
        "default_bind",
        "bmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    let cfg = load_config(&p).unwrap();
    assert_eq!(cfg.listen_addr, "127.0.0.1:9417".parse().unwrap());
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test config_test`
Expected: 新增 3 个测试 FAIL（userinfo 被接受、控制字符被接受、默认绑定仍是 `0.0.0.0`）

- [ ] **Step 3: 实现**

`src/config.rs` 三处改动：

`default_listen_addr`：

```rust
fn default_listen_addr() -> String {
    "127.0.0.1:9417".into()
}
```

bmc 循环中 name 空校验之后加控制字符校验（`char::is_control` 覆盖 C0/DEL/C1）：

```rust
        if b.name.chars().any(|c| c.is_control()) {
            return Err(ConfigError::Invalid(format!(
                "bmc '{}': name must not contain control characters",
                b.name
            )));
        }
```

scheme 校验之后加 userinfo 拒绝（url crate：`username()`/`password()` 即 userinfo 部分，与配置文件 `username` 字段无关）：

```rust
        if !host.username().is_empty() || host.password().is_some() {
            return Err(ConfigError::Invalid(format!(
                "host '{}': credentials in URL are not allowed; use the username/password fields",
                b.host
            )));
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test config_test`（现有 `parses_valid_config` 显式写 `0.0.0.0:9417` 不受默认值影响，仍应通过）
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```powershell
git add src/config.rs tests/config_test.rs
git commit -m "fix: reject URL credentials and control chars in config, default bind to 127.0.0.1"
```

---

### Task 2: SecretString zeroize（Drop 清零）

**Files:**
- Modify: `Cargo.toml:32`（dependencies 追加）
- Modify: `src/config.rs:11-25`（SecretString）
- Test: `tests/config_test.rs`（追加）

**Interfaces:**
- Produces: `SecretString` 在 `Drop` 时对内部 `String` 缓冲区 `zeroize()`

- [ ] **Step 1: 添加依赖并写失败测试**

`Cargo.toml` dependencies 段追加：

```toml
zeroize = "1"
```

追加测试（验证手法与 zeroize crate 自身测试一致：捕获堆指针，drop 后读取原位置；仅此测试允许 unsafe，附注释说明）：

```rust
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
    assert!(
        bytes.iter().all(|&b| b == 0),
        "secret not zeroized: {:?}",
        &bytes[..bytes.len().min(32)]
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test config_test secret_zeroizes_on_drop`
Expected: FAIL（内存仍是密码明文）

- [ ] **Step 3: 实现**

`src/config.rs`：

```rust
use zeroize::Zeroize;
```

```rust
impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test config_test`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```powershell
git add Cargo.toml Cargo.lock src/config.rs tests/config_test.rs
git commit -m "feat: zeroize SecretString password buffer on drop"
```

---

### Task 3: 环境变量凭据覆盖（load_config_with_env + 冲突/空值校验）

**Files:**
- Modify: `src/config.rs:141`（load_config 拆分为可注入 env lookup 的版本）、bmc 循环内密码解析
- Test: `tests/config_test.rs`（追加）

**Interfaces:**
- Produces:
  - `pub fn env_var_name(name: &str) -> String`（name 大写、非 `[A-Z0-9]` → `_`）
  - `pub fn load_config(path: &Path) -> Result<Config, ConfigError>`（委托 `load_config_with_env(path, |k| std::env::var(k).ok())`）
  - `pub fn load_config_with_env(path: &Path, env_lookup: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError>`

- [ ] **Step 1: 写失败测试**

```rust
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
    assert!(matches!(load_config_with_env(&p, |_| None), Err(ConfigError::Invalid(_))));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test config_test env_var`
Expected: 4 个测试 FAIL（`load_config_with_env` 不存在）

- [ ] **Step 3: 实现**

`src/config.rs`：在 `load_config` 前加：

```rust
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
```

`load_config` 改写为委托（原函数体移入 `load_config_with_env`，签名改为泛型）：

```rust
pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    load_config_with_env(path, |key| std::env::var(key).ok())
}

pub fn load_config_with_env(
    path: &Path,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Result<Config, ConfigError> {
```

函数内，`let mut names = ...` 之后加 env 名映射表，bmc 循环内 password 非空校验之后、`bmcs.push` 之前加（Task 1 已引入 `zeroize` import，此处复用）：

```rust
    let mut env_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
```

循环内：

```rust
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
```

并把 `bmcs.push` 中 `password: SecretString::new(b.password)` 改为 `password: SecretString::new(password)`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test config_test`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```powershell
git add src/config.rs tests/config_test.rs
git commit -m "feat: environment variable password override with collision detection"
```

---

### Task 4: Bearer Token 认证（web 配置节 + auth 模块 + middleware）

**Files:**
- Create: `src/auth.rs`
- Modify: `src/lib.rs`（注册模块）、`src/config.rs`（WebConfig + 校验）、`src/http.rs`（AppState/路由/middleware）、`src/main.rs:50`（不变——serve 内部取 token）
- Test: `tests/config_test.rs`（web 节校验）、`tests/http_test.rs`（认证行为 + 现有调用点适配）

**Interfaces:**
- Consumes: Task 1/3 的 config 结构
- Produces:
  - `pub struct WebConfig { pub auth_token: Option<SecretString>, pub tls_cert_file: Option<PathBuf>, pub tls_key_file: Option<PathBuf> }`（`Config.web` 字段，`#[derive(Debug, Clone, Default)]`）
  - `crate::auth::constant_time_eq(a: &[u8], b: &[u8]) -> bool`、`crate::auth::bearer_authorized(headers: &axum::http::HeaderMap, expected: &str) -> bool`
  - `http::router(snapshot, config, config_path, auth_token: Option<SecretString>) -> Router`（签名变化，Task 5 依赖此签名）

- [ ] **Step 1: 写失败测试（config 校验）**

`tests/config_test.rs` 追加：

```rust
#[test]
fn web_token_too_short_is_rejected() {
    let p = write_tmp(
        "web_token_short",
        "web:\n  auth_token: short\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn web_token_and_file_are_mutually_exclusive() {
    let p = write_tmp(
        "web_token_both",
        "web:\n  auth_token: 0123456789abcdef\n  auth_token_file: /tmp/tok\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
}

#[test]
fn web_tls_cert_requires_key() {
    let p = write_tmp(
        "web_tls_half",
        "web:\n  tls_cert_file: /tmp/cert.pem\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config(&p), Err(ConfigError::Invalid(_))));
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
    let cfg = load_config(&p).unwrap();
    assert_eq!(cfg.web.auth_token.as_ref().unwrap().expose(), "0123456789abcdef");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test config_test web_`
Expected: FAIL（web 节被忽略/不存在）

- [ ] **Step 3: 实现 config 部分**

`src/config.rs`：

```rust
#[derive(Debug, Clone, Default)]
pub struct WebConfig {
    pub auth_token: Option<SecretString>,
    pub tls_cert_file: Option<PathBuf>,
    pub tls_key_file: Option<PathBuf>,
}
```

`Config` 增加字段 `pub web: WebConfig`；`Ok(Config { ... })` 增加 `web`。

RawConfig 增加 `#[serde(default)] web: RawWebConfig`，并新增：

```rust
#[derive(Deserialize, Default)]
struct RawWebConfig {
    auth_token: Option<String>,
    auth_token_file: Option<PathBuf>,
    tls_cert_file: Option<PathBuf>,
    tls_key_file: Option<PathBuf>,
}
```

`load_config_with_env` 末尾（Ok 构造前）加：

```rust
    let web = build_web_config(&raw.web)?;
```

并新增函数：

```rust
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
```

- [ ] **Step 4: 运行 config 测试确认通过**

Run: `cargo test --test config_test`
Expected: 新测试 PASS（http_test 等此刻编译失败属预期，下一子步骤修复）

- [ ] **Step 5: 写失败测试（认证行为）**

`tests/http_test.rs`：先改两个辅助函数与两处 `router(...)` 调用：

```rust
fn test_router(snapshot: Arc<Snapshot>, cfg: Config) -> Router {
    router(
        snapshot,
        Arc::new(RwLock::new(cfg)),
        PathBuf::from("config.yaml"),
        None,
    )
}

fn test_router_with_token(snapshot: Arc<Snapshot>, cfg: Config, token: &str) -> Router {
    router(
        snapshot,
        Arc::new(RwLock::new(cfg)),
        PathBuf::from("config.yaml"),
        Some(SecretString::new(token.into())),
    )
}
```

`test_config` 中 `Config {` 字面量补 `web: WebConfig::default(),`；顶部 use 增加 `WebConfig`。两处 `let app = router(snap, cfg, path.clone());` 改为 `router(snap, cfg, path.clone(), None)`。

追加测试：

```rust
const TEST_TOKEN: &str = "0123456789abcdef";

#[tokio::test]
async fn auth_missing_token_returns_401_with_challenge() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(snap, test_config(&[("bmc1", "https://10.0.0.1")]), TEST_TOKEN);
    let resp = app
        .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers()["www-authenticate"], "Bearer");
}

#[tokio::test]
async fn auth_wrong_token_returns_401() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(snap, test_config(&[("bmc1", "https://10.0.0.1")]), TEST_TOKEN);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("authorization", "Bearer fedcba9876543210")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_correct_token_grants_all_get_endpoints() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router_with_token(snap, test_config(&[("bmc1", "https://10.0.0.1")]), TEST_TOKEN);
    for path in ["/metrics", "/healthz", "/info", "/discover"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "endpoint {path}");
    }
}

#[tokio::test]
async fn auth_correct_token_grants_reload() {
    let dir = temp_config_dir();
    let path = dir.join("config.yaml");
    std::fs::write(&path, CONFIG_A).unwrap();
    let cfg = Arc::new(RwLock::new(load_config(&path).unwrap()));
    let snap = Arc::new(Snapshot::new());
    let app = router(snap, cfg, path.clone(), Some(SecretString::new(TEST_TOKEN.into())));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reload")
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    cleanup(&dir);
}

#[tokio::test]
async fn auth_disabled_keeps_open_access() {
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let resp = app
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
```

- [ ] **Step 6: 运行测试确认失败**

Run: `cargo test --test http_test auth_`
Expected: FAIL（router 尚不接受 token 参数 / 无 middleware）

- [ ] **Step 7: 实现 auth 模块与 middleware**

新建 `src/auth.rs`：

```rust
//! Bearer 认证辅助：常量时间 token 比较与 Authorization 头解析（纯函数，便于单测）。

use axum::http::HeaderMap;

/// 常量时间比较：XOR 折叠遍历到两输入最大长度，长度差异折叠进累加器，
/// 不因长度或内容差异提前退出（防时序侧信道）。空输入相等仅当两者皆空。
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut acc = 0u8;
    for i in 0..a.len().max(b.len()) {
        acc |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    acc == 0
}

/// 校验请求是否携带合法的 `Authorization: Bearer <expected>`（前缀区分大小写，按 RFC 6750）。
pub fn bearer_authorized(headers: &HeaderMap, expected: &str) -> bool {
    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}
```

`src/lib.rs` 增加 `pub mod auth;`。

`src/http.rs`：AppState 增加字段，router 签名与 middleware：

```rust
use crate::auth::bearer_authorized;
use crate::config::SecretString;
```

```rust
struct AppState {
    snapshot: Arc<Snapshot>,
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    auth_token: Option<SecretString>,
}

pub fn router(
    snapshot: Arc<Snapshot>,
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    auth_token: Option<SecretString>,
) -> Router {
    let state = AppState {
        snapshot,
        config,
        config_path,
        auth_token,
    };
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler))
        .route("/info", get(info_handler))
        .route("/discover", get(discover_handler))
        .route("/reload", post(reload_handler))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

/// Bearer 认证中间件：配置了 token 时全部端点（含 /healthz、/reload）统一要求认证。
/// 401 + `WWW-Authenticate: Bearer`；未配置 token 时直通（默认安全基线外的显式选择）。
async fn auth_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(token) = &state.auth_token
        && !bearer_authorized(req.headers(), token.expose())
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
            "unauthorized",
        )
            .into_response();
    }
    next.run(req).await
}
```

`src/http.rs` 的 `serve()` 内，把 `let listen_addr = config.read().await.listen_addr;` 改为一次读出监听地址与 token：

```rust
    let (listen_addr, auth_token) = {
        let cfg = config.read().await;
        (cfg.listen_addr, cfg.web.auth_token.clone())
    };
```

并改 router 调用：`router(snapshot, config, config_path, auth_token)`。注意：token 变更需重启生效（middleware 持有启动时的值，与 spec §4.3 reload 语义一致，文档在 Task 10 写明）。

- [ ] **Step 8: 运行全部测试确认通过**

Run: `cargo test --test config_test`; `cargo test --test http_test`
Expected: 全部 PASS

- [ ] **Step 9: 提交**

```powershell
git add src/config.rs src/auth.rs src/lib.rs src/http.rs tests/config_test.rs tests/http_test.rs
git commit -m "feat: optional bearer token auth for all endpoints (constant-time compare)"
```

---

### Task 5: 自身 TLS + header read timeout（axum-server 承载，serve 增加测试接缝）

**Files:**
- Modify: `Cargo.toml:17`（dependencies 追加 axum-server；dev-dependencies 追加 rcgen）、`src/http.rs`（serve 改造）
- Test: `tests/http_test.rs`（追加）

**Interfaces:**
- Consumes: Task 4 的 `router(..., auth_token)` 签名与 `WebConfig`
- Produces:
  - `pub const DEFAULT_HEADER_READ_TIMEOUT: Duration`（10s）
  - `pub async fn load_tls(web: &WebConfig) -> anyhow::Result<Option<axum_server::tls_rustls::RustlsConfig>>`（无效 PEM/文件缺失 → Err，fail-fast）
  - `pub async fn serve_on(listener: std::net::TcpListener, router: Router, tls: Option<RustlsConfig>, header_read_timeout: Duration, stop: watch::Receiver<bool>) -> anyhow::Result<()>`
  - `serve()` 组装上述（main.rs 调用不变）

- [ ] **Step 1: 添加依赖**

```toml
axum-server = { version = "0.7", default-features = false, features = ["tls-rustls"] }
```

dev-dependencies：

```toml
rcgen = "0.13"
```

Run: `cargo build`（触发依赖解析；如 axum-server 0.7 与 axum 0.8 不兼容，改为能解析的最新 0.x 并在下一步验证零 OpenSSL）
Expected: 编译通过

- [ ] **Step 2: 验证零 OpenSSL**

Run: `cargo tree | Select-String -Pattern "openssl|native-tls"`（PowerShell）
Expected: 无输出（rustls 栈保持）

- [ ] **Step 3: 写失败测试（load_tls 无效 PEM + 端到端自签 TLS + header 超时）**

`tests/http_test.rs` 追加（use 增加：`use redfish_exporter::config::WebConfig;`、`use std::io::Read;`）：

```rust
#[tokio::test]
async fn load_tls_rejects_invalid_pem() {
    let dir = temp_config_dir();
    std::fs::write(dir.join("cert.pem"), "not a pem").unwrap();
    std::fs::write(dir.join("key.pem"), "also not a pem").unwrap();
    let web = WebConfig {
        auth_token: None,
        tls_cert_file: Some(dir.join("cert.pem")),
        tls_key_file: Some(dir.join("key.pem")),
    };
    assert!(redfish_exporter::http::load_tls(&web).await.is_err());
    cleanup(&dir);
}

#[tokio::test]
async fn tls_end_to_end_with_self_signed_cert() {
    let dir = temp_config_dir();
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    let web = WebConfig {
        auth_token: None,
        tls_cert_file: Some(cert_path),
        tls_key_file: Some(key_path),
    };
    let tls = redfish_exporter::http::load_tls(&web).await.unwrap().expect("tls loaded");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move {
        redfish_exporter::http::serve_on(listener, app, Some(tls), Duration::from_secs(10), rx)
            .await
    });
    // 等待监听就绪（TCP 连接探测）
    let mut connected = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(addr).is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(connected, "server did not start");
    // 信任证书的客户端成功
    let trusting = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(cert.cert.pem().as_bytes()).unwrap())
        .build()
        .unwrap();
    let resp = trusting
        .get(format!("https://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ok");
    // 不信任证书的客户端失败（TLS 实际生效）
    let plain = reqwest::Client::new();
    assert!(plain.get(format!("https://{addr}/healthz")).send().await.is_err());
    let _ = tx.send(true);
    handle.await.unwrap().unwrap();
    cleanup(&dir);
}

#[tokio::test]
async fn header_read_timeout_closes_idle_connection() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let snap = Arc::new(Snapshot::new());
    let app = test_router(snap, test_config(&[("bmc1", "https://10.0.0.1")]));
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(async move {
        redfish_exporter::http::serve_on(listener, app, None, Duration::from_millis(300), rx)
            .await
    });
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0u8; 16];
    let started = std::time::Instant::now();
    let n = stream.read(&mut buf);
    let elapsed = started.elapsed();
    assert!(
        n.map(|n| n == 0).unwrap_or(true),
        "expected connection closed by server (EOF)"
    );
    assert!(elapsed < Duration::from_secs(5), "timeout took {elapsed:?}");
    let _ = tx.send(true);
    handle.await.unwrap().unwrap();
}
```

- [ ] **Step 4: 运行测试确认失败**

Run: `cargo test --test http_test load_tls` / `tls_end_to_end` / `header_read_timeout`
Expected: FAIL（`load_tls`/`serve_on` 不存在）

- [ ] **Step 5: 实现 serve 改造**

`src/http.rs`（保留现有 handler 与 middleware；顶部 use 增加 `std::net::TcpListener as StdTcpListener`、`std::time::Duration`、`axum_server::tls_rustls::RustlsConfig`）：

```rust
/// 默认 HTTP/1.1 header 读取超时（慢连接防护，spec §3.1）。
pub const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// 读取 web 节的 TLS 配置；未配置返回 Ok(None)（纯 HTTP）。失败即启动失败（fail-fast）。
pub async fn load_tls(web: &crate::config::WebConfig) -> anyhow::Result<Option<RustlsConfig>> {
    match (&web.tls_cert_file, &web.tls_key_file) {
        (Some(cert), Some(key)) => {
            let cfg = RustlsConfig::from_pem_file(cert, key)
                .await
                .with_context(|| {
                    format!(
                        "failed to load TLS cert '{}' / key '{}'",
                        cert.display(),
                        key.display()
                    )
                })?;
            Ok(Some(cfg))
        }
        (None, None) => Ok(None),
        _ => unreachable!("cert/key pairing validated in load_config"),
    }
}

/// 在已绑定的 listener 上服务（测试接缝：测试自建 listener 以获取端口 0 的实际地址）。
/// stop 触发后优雅停机（axum-server Handle）。
pub async fn serve_on(
    listener: StdTcpListener,
    router: Router,
    tls: Option<RustlsConfig>,
    header_read_timeout: Duration,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut server = match tls {
        Some(cfg) => axum_server::Server::from_tcp_rustls(listener, cfg)
            .context("failed to start TLS server")?,
        None => axum_server::Server::from_tcp(listener).context("failed to start server")?,
    };
    server
        .http_builder()
        .header_read_timeout(Some(header_read_timeout));
    let handle = server.handle();
    tokio::spawn(async move {
        let _ = stop.changed().await;
        handle.graceful_shutdown(None);
    });
    server.serve(router).await.context("http server error")
}
```

`serve()` 改写（main.rs 调用签名不变）：

```rust
pub async fn serve(
    config: Arc<RwLock<Config>>,
    snapshot: Arc<Snapshot>,
    config_path: PathBuf,
    stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let (listen_addr, web) = {
        let cfg = config.read().await;
        (cfg.listen_addr, cfg.web.clone())
    };
    let tls = load_tls(&web).await?;
    let listener = StdTcpListener::bind(listen_addr)
        .with_context(|| format!("failed to bind listen address {listen_addr}"))?;
    info!(addr = %listen_addr, tls = tls.is_some(), "http server listening");
    let auth_token = web.auth_token.clone();
    serve_on(
        listener,
        router(snapshot, config, config_path, auth_token),
        tls,
        DEFAULT_HEADER_READ_TIMEOUT,
        stop,
    )
    .await
}
```

注意（执行者必须核对 rustdoc，axum-server 0.7 API）：`Server::from_tcp(listener: StdTcpListener)`、`Server::from_tcp_rustls(listener, RustlsConfig)`、`http_builder()` 返回 `Http1Builder`、`header_read_timeout(Option<Duration>)`、`handle()` / `Handle::graceful_shutdown(Option<Duration>)`、`serve(Router)`。若方法名/签名不同（如 `serve` 需要 `into_make_service()`），以解析版本的 rustdoc 为准调整，行为语义不变。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test --test http_test`
Expected: 全部 PASS（含既有 8 个测试）

- [ ] **Step 7: 提交**

```powershell
git add Cargo.toml Cargo.lock src/http.rs tests/http_test.rs
git commit -m "feat: optional rustls TLS for the exporter HTTP server with header read timeout"
```

---

### Task 6: 出站重定向降级阻断（https→http）

**Files:**
- Modify: `src/bmc.rs:50-53`（build_http_client）
- Test: `tests/bmc_test.rs`（追加）

**Interfaces:**
- Produces: `pub fn decide_redirect(previous: &[url::Url], next: &url::Url) -> bool`（纯函数，测试接缝，与 scraper.rs 的 slow_due 同法）、`pub fn no_downgrade_redirect() -> reqwest::redirect::Policy`

- [ ] **Step 1: 写失败测试**（`tests/bmc_test.rs` 追加，执行者先读该文件顶部了解现有 import 与 `cfg()` 辅助）

```rust
#[test]
fn redirect_policy_blocks_https_downgrade() {
    use redfish_exporter::bmc::decide_redirect;
    let https = url::Url::parse("https://bmc.example/redfish/v1").unwrap();
    let http = url::Url::parse("http://bmc.example/redfish/v1/Systems").unwrap();
    let https2 = url::Url::parse("https://bmc.example/redfish/v1/Systems").unwrap();
    assert!(!decide_redirect(&[https.clone()], &http), "https->http must be blocked");
    assert!(decide_redirect(&[https], &https2), "https->https allowed");
    assert!(decide_redirect(&[http.clone()], &http), "http->http allowed");
    assert!(decide_redirect(&[], &http), "no previous hop is not a downgrade");
    let chain: Vec<url::Url> = (0..10)
        .map(|i| url::Url::parse(&format!("https://bmc.example/r{i}")).unwrap())
        .collect();
    assert!(!decide_redirect(&chain, &http), "10-hop limit enforced");
}
```

（若 `tests/bmc_test.rs` 未 import `url`，加 `use url::Url;` 或全路径引用）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test bmc_test redirect_policy`
Expected: FAIL（函数不存在）

- [ ] **Step 3: 实现**

`src/bmc.rs`（use 区已有 `std::time::Duration`，追加 `reqwest::redirect::Policy`）：

```rust
/// 判定一次重定向是否放行：拒绝 https→http 降级；最多 10 跳（与 reqwest 默认一致）。
/// 独立为纯函数便于单元测试（与 scraper 的 slow_due 同法）。
pub fn decide_redirect(previous: &[url::Url], next: &url::Url) -> bool {
    if previous.len() >= 10 {
        return false;
    }
    match previous.last() {
        Some(prev) if prev.scheme() == "https" && next.scheme() == "http" => false,
        _ => true,
    }
}

/// 出站重定向策略：阻止 TLS 降级（https→http），其余按 10 跳上限跟随。
/// 注意：reqwest 自定义 policy 不自动限制跳数（文档明确），故跳数上限在 decide_redirect 内实现。
pub fn no_downgrade_redirect() -> Policy {
    Policy::custom(|attempt| {
        if decide_redirect(attempt.previous(), attempt.url()) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}
```

`build_http_client` 的 builder 链（`.user_agent(...)` 之后）加：

```rust
        .redirect(no_downgrade_redirect())
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test bmc_test`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```powershell
git add src/bmc.rs tests/bmc_test.rs
git commit -m "feat: block https->http redirect downgrade on outbound BMC requests"
```

---

### Task 7: 分页防御上限（错误类型化 + 64MiB/1000 页/20 万成员）

**Files:**
- Modify: `src/pagination.rs`（整体改造）、`src/collector/logs.rs:41-43`
- Test: `tests/pagination_test.rs`（追加）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub enum PaginationError<B: Bmc> { Bmc(#[from] B::Error), PageTooLarge, TooManyPages(usize), TooManyMembers(usize) }`（`#[derive(Debug, thiserror::Error)]`）
  - `pub const MAX_PAGES: usize = 1000; pub const MAX_PAGE_BYTES: usize = 64 * 1024 * 1024; pub const MAX_TOTAL_MEMBERS: usize = 200_000;`
  - `pub async fn fetch_all_pages<B: Bmc>(bmc, url) -> Result<Vec<Value>, PaginationError<B>>`（返回类型变化）
  - `pub async fn fetch_all_pages_with_limits<B: Bmc>(bmc, url, max_pages, max_page_bytes, max_total_members) -> Result<Vec<Value>, PaginationError<B>>`（测试接缝）

- [ ] **Step 1: 写失败测试**（`tests/pagination_test.rs` 追加）

```rust
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
        Err(redfish_exporter::pagination::PaginationError::TooManyPages(2))
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
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test pagination_test`
Expected: 3 个新测试 FAIL（函数/类型不存在）

- [ ] **Step 3: 实现 pagination.rs**

完整替换 `fetch_all_pages` 及常量（`Page` 增加 `Serialize` derive；保留 `resolve_next_link` 与防循环逻辑）：

```rust
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Page { /* 不变 */ }

/// 分页页数上限：防设备返回环状 nextLink（visited 去重为主防线，此为第二防线）。
pub const MAX_PAGES: usize = 1000;
/// 单页响应体上限（64MiB）：防异常/恶意 BMC 超大响应耗尽内存。
/// 说明：nv-redfish 类型化 fetch 不提供流式读取（Bmc trait 无 raw get），
/// 上限以反序列化后序列化字节数近似校验（与真实响应同量级），超限即报错丢弃，
/// 内存峰值受限于单页；流式上限记入 backlog（security.md 如实文档化）。
pub const MAX_PAGE_BYTES: usize = 64 * 1024 * 1024;
/// 累计成员上限：防无 nextLink 变化的设备无限积累（真机最大 2064 条，留 100 倍余量）。
pub const MAX_TOTAL_MEMBERS: usize = 200_000;

/// 分页失败：BMC 请求错误 / 单页超限 / 页数超限 / 成员累计超限。
#[derive(Debug, thiserror::Error)]
pub enum PaginationError<B: Bmc> {
    #[error("bmc request failed: {0}")]
    Bmc(#[from] B::Error),
    #[error("pagination page exceeds size limit")]
    PageTooLarge,
    #[error("pagination exceeded {0} pages")]
    TooManyPages(usize),
    #[error("pagination exceeded {0} total members")]
    TooManyMembers(usize),
}

pub async fn fetch_all_pages<B: Bmc>(
    bmc: &Arc<B>,
    url: &ODataId,
) -> Result<Vec<Value>, PaginationError<B>> {
    fetch_all_pages_with_limits(bmc, url, MAX_PAGES, MAX_PAGE_BYTES, MAX_TOTAL_MEMBERS).await
}

/// 带显式上限的分页抓取（测试接缝）：上限语义同 fetch_all_pages。
pub async fn fetch_all_pages_with_limits<B: Bmc>(
    bmc: &Arc<B>,
    url: &ODataId,
    max_pages: usize,
    max_page_bytes: usize,
    max_total_members: usize,
) -> Result<Vec<Value>, PaginationError<B>> {
    let mut out = Vec::new();
    let mut next = url.clone();
    let mut visited = std::collections::HashSet::new();
    visited.insert(next.to_string());
    for _ in 0..max_pages {
        let page = bmc.get::<Page>(&next).await?;
        let size = serde_json::to_vec(&*page).map(|v| v.len()).unwrap_or(usize::MAX);
        if size > max_page_bytes {
            tracing::warn!(url = %next, size, "pagination page exceeds size limit");
            return Err(PaginationError::PageTooLarge);
        }
        out.extend(page.members.iter().cloned());
        if out.len() > max_total_members {
            tracing::warn!(
                url = %next,
                members = out.len(),
                "pagination member accumulation limit reached"
            );
            return Err(PaginationError::TooManyMembers(max_total_members));
        }
        let Some(link) = &page.next_link else {
            return Ok(out);
        };
        let Some(resolved) = resolve_next_link(&next, link) else {
            return Ok(out);
        };
        if !visited.insert(resolved.to_string()) {
            tracing::warn!(url = %resolved, "pagination loop detected, stopping");
            return Ok(out);
        }
        next = resolved;
    }
    tracing::warn!(url = %next, "pagination page limit reached");
    Err(PaginationError::TooManyPages(max_pages))
}
```

`src/collector/logs.rs:41-43` 改为错误可见化（子资源隔离惯例：warn + 跳过该日志服务，不拖垮其他服务）：

```rust
            let entries = match fetch_all_pages(&bmc, entries_ref.id()).await {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::warn!(
                        bmc = %bmc_name,
                        manager = %manager_id,
                        service = %service_id,
                        error = %e,
                        "event log pagination failed, skipping log service"
                    );
                    continue;
                }
            };
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test pagination_test`; `cargo test --test collector_logs_test`
Expected: 全部 PASS（既有测试调用 `fetch_all_pages(...).unwrap()`，新错误类型实现 Debug，不受影响）

- [ ] **Step 5: 提交**

```powershell
git add src/pagination.rs src/collector/logs.rs tests/pagination_test.rs
git commit -m "feat: pagination defense caps (page size, page count, member accumulation)"
```

---

### Task 8: 配置文件权限警告（Unix）

**Files:**
- Modify: `src/config.rs`（追加）、`src/main.rs:37-41`（加载后 warn）
- Test: `tests/config_test.rs`（追加，`#[cfg(unix)]`）

**Interfaces:**
- Produces: `#[cfg(unix)] pub fn config_file_is_wide_open(path: &Path) -> bool`（mode & 0o077 != 0）

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(unix)]
#[test]
fn detects_wide_open_config_permissions() {
    use redfish_exporter::config::config_file_is_wide_open;
    use std::os::unix::fs::PermissionsExt;
    let p = write_tmp("perm_wide", "bmcs: []\n");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(config_file_is_wide_open(&p));
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!config_file_is_wide_open(&p));
}
```

- [ ] **Step 2: 运行测试确认失败（Windows 上跳过，Linux CI 验证）**

Run: `cargo test --test config_test detects_wide_open`
Expected: Windows 上 SKIP（`#[cfg(unix)]`）；Linux CI 上 FAIL（函数不存在）

- [ ] **Step 3: 实现**

`src/config.rs` 末尾追加：

```rust
/// Unix：配置文件是否对 group/other 可读（mode & 0o077 != 0）。Windows 无此函数。
#[cfg(unix)]
pub fn config_file_is_wide_open(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path)
        .map(|m| m.mode() & 0o077 != 0)
        .unwrap_or(false)
}
```

`src/main.rs` 中 `load_config` 成功之后、`port` 覆盖之前加：

```rust
    #[cfg(unix)]
    if config::config_file_is_wide_open(&args.config) {
        tracing::warn!(
            path = %args.config.display(),
            "config file is readable by group/others; consider chmod 600"
        );
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test config_test`; `cargo build`
Expected: PASS / 编译通过

- [ ] **Step 5: 提交**

```powershell
git add src/config.rs src/main.rs tests/config_test.rs
git commit -m "feat: warn on group/world-readable config file (unix)"
```

---

### Task 9: 供应链门禁（audit.toml 白名单 + deny.toml 四类策略）

**Files:**
- Create: `.cargo/audit.toml`、`deny.toml`
- Modify: `.github/workflows/ci.yml:26-27`（注释）

**Interfaces:**
- Produces: `cargo audit` 忽略两项构建期漏洞；`cargo deny` 全绿

- [ ] **Step 1: 创建 `.cargo/audit.toml`**（cargo-audit 自动读取该路径）

```toml
# cargo-audit 忽略清单（0.1.0 安全专项 spec §5 / 验收报告 §5）
# RUSTSEC-2026-0194 / RUSTSEC-2026-0195：quick-xml 0.38.4 漏洞。
# quick-xml 仅经 nv-redfish-csdl-compiler 进入构建期依赖树，不进入运行时二进制
# （cargo tree --edges normal 无 quick-xml）。nv-redfish 0.15.1 锁定无法升级
# （0.41 为 major bump，升级破坏依赖链约束）；跟踪 nv-redfish 上游修复后移除本清单。
[advisories]
ignore = [
  "RUSTSEC-2026-0194",
  "RUSTSEC-2026-0195",
]
```

- [ ] **Step 2: 创建 `deny.toml`**

```toml
# cargo-deny 策略（0.1.0 安全专项 spec §5）：advisories/bans/licenses/sources 四类全开。
[advisories]
version = 2
yanked = "deny"

[bans]
multiple-versions = "deny"
wildcards = "deny"
# 依赖树若存在不可避免的同名多版本（执行 cargo deny check bans 观察输出），
# 以 [[bans.skip]] 逐项豁免并附理由注释，禁止 wildcard 豁免。

[licenses]
version = 2
allow = [
  "Apache-2.0",
  "MIT",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Zlib",
  "Unicode-3.0",
  "MPL-2.0",
  "OpenSSL",
]
# 说明：MPL-2.0（webpki-roots）与 OpenSSL（ring 许可证表达式组成部分）为 rustls 栈既有依赖。
# 若 cargo deny check licenses 报出清单外许可证，仅可添加 OSI/FSF 批准的许可证并附理由，禁止 "*"。

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 3: 本地验证并迭代**

Run: `cargo deny check`（未安装则 `cargo install cargo-deny --locked`，PowerShell 注意参数转义）
Expected: 全绿；若 bans 出现多版本冲突，按 Step 2 注释处理（逐项 skip + 理由）；licenses 同理。

- [ ] **Step 4: 本地验证 audit 白名单**

Run: `cargo audit`（未安装则 `cargo install cargo-audit --locked`）
Expected: 2 个 high 被忽略（输出中标注 ignored），无其他新告警

- [ ] **Step 5: ci.yml 注释**

`ci.yml` audit job 中 `run: cargo audit` 上方加注释行：

```yaml
      - name: Audit dependencies
        # 白名单见 .cargo/audit.toml（quick-xml 构建期依赖，跟踪 nv-redfish 上游）
        run: cargo audit
```

- [ ] **Step 6: 提交**

```powershell
git add .cargo/audit.toml deny.toml .github/workflows/ci.yml
git commit -m "chore: cargo audit allowlist with rationale and cargo-deny policy"
```

---

### Task 10: 文档交付（security.md / README / config.example / design.md / deploy）

**Files:**
- Create: `docs/security.md`
- Modify: `README.md`（§Quick start 端点表下注记、§Configuration 表、§Security 节）、`config.example.yaml`、`docs/design.md`（新增安全设计节）、`deploy/kubernetes/deployment.yaml:29`（注释）

- [ ] **Step 1: 创建 `docs/security.md`**（内容如下，须与实际实现一致）

```markdown
# 安全设计（0.1.0 安全专项）

日期：2026-08-21
依据：spec `docs/superpowers/specs/2026-08-21-security-hardening-design.md`

## 威胁模型

部署形态为混合环境（可信内网 / 半信任 / 高暴露并存），采用分层可配置安全模型：
默认安全基线（零成本）+ 可选 Bearer 认证 + 可选自身 TLS。

### 入站面（exporter HTTP）

| 攻击面 | 防线 |
|---|---|
| 默认全网卡监听 | 默认 `127.0.0.1:9417`；远程部署需显式 `0.0.0.0` 并配置认证/TLS |
| 端点无认证（数据/拓扑泄露） | `web.auth_token[_file]` 开启后全端点（含 /healthz、/reload）统一 401 + `WWW-Authenticate: Bearer`；常量时间比较防时序侧信道 |
| 明文传输 | `web.tls_cert_file`/`tls_key_file`（rustls，零 OpenSSL） |
| 慢连接占用 | HTTP/1.1 header read timeout 10s |
| 日志注入 | BMC name 拒绝控制字符（配置校验） |

### 出站面（到 BMC）

| 攻击面 | 防线 |
|---|---|
| URL 内嵌凭据 | host 含 userinfo 拒绝配置 |
| 密码残留内存 | SecretString Drop 时 zeroize；已知局限：nv-redfish 内部凭据副本不受控（如实说明） |
| 密码落盘 | 环境变量覆盖 `REDFISH_EXPORTER_PASSWORD_<NAME>`（name 大写、非字母数字替换 `_`；空值/映射冲突报错） |
| TLS 降级 | 重定向 https→http 阻断 + 10 跳上限 |
| 响应耗尽内存 | 分页单页 64MiB / 页数 1000 / 累计成员 200,000；已知局限：上限为反序列化后校验（nv 类型化 fetch 无流式读取），流式上限记 backlog |
| TLS 校验绕过 | per-BMC `insecure_skip_verify` opt-in，优先 `ca_cert_file` |
| 代理截获 | 环境变量透传（reqwest 默认）；BMC 网段应入 NO_PROXY 或依赖 TLS |

### 运行时 / 供应链

- `#![forbid(unsafe_code)]`；TLS 仅 rustls（零 OpenSSL/native-tls）；cargo deny 四类策略
- cargo audit：2 个 high（quick-xml，构建期依赖）白名单化，跟踪 nv-redfish 上游（`.cargo/audit.toml`）
- Unix 配置文件权限过宽启动 warn（建议 chmod 600）；Windows 无此检查
- 会话 token 驻留 nv 内部凭据；shutdown 删除服务器端会话（既有 Task 13）

## 基线矩阵

| 档位 | 入站 | 出站 | 适用 |
|---|---|---|---|
| 可信内网 | 默认 `127.0.0.1` 或显式 `0.0.0.0` 依赖网络边界 | TLS 校验默认开启 + `ca_cert_file` | 同网段 Prometheus 直连 |
| 半信任 | + `auth_token_file` | 同上 + NO_PROXY 覆盖 BMC 网段 | 跨网段/多租户 |
| 高暴露 | + `tls_cert_file`/`tls_key_file` | 严格校验（禁 `insecure_skip_verify`） | 公网可达 |

## 运维说明

- 认证开启时 Prometheus 用 `bearer_token_file`；k8s 探针需在 httpGet 中带 token。
- 认证/TLS 配置变更需重启生效（/reload 仅热替换配置对象，与 BMC 凭据变更同语义）。
- `auth_token`（≥16 字符）与 `auth_token_file` 二选一；推荐文件方式（防进程列表/env 泄露），文件末尾换行会被裁剪。
```

- [ ] **Step 2: 更新 `config.example.yaml`**

```yaml
# redfish-exporter configuration example
listen_addr: "0.0.0.0:9417"   # default is now 127.0.0.1:9417; remote Prometheus needs an explicit bind
scrape_interval: "30s"         # periodic scrape interval (default 30s)
# slow_interval: "300s"        # optional: slow group (storage/network/firmware/assembly/event log/BIOS) interval (default: every round)
scrape_timeout: "15s"          # per-round deadline (default 15s)
request_timeout: "10s"         # per-request timeout (default 10s)
web:                           # optional inbound hardening (see docs/security.md)
  # auth_token: "change-me-at-least-16-chars"   # or auth_token_file: /path/to/token (mutually exclusive)
  # auth_token_file: null
  # tls_cert_file: /path/to/server.pem           # both must be set together
  # tls_key_file: /path/to/server-key.pem
bmcs:
  - name: bmc1                 # unique, used as the `bmc` metric label
    host: https://10.0.0.1     # http:// or https://; credentials in the URL are rejected
    username: admin
    password: "change-me"      # can be overridden by env var REDFISH_EXPORTER_PASSWORD_BMC1
    auth: basic                # basic | session (session creates an X-Auth-Token)
    insecure_skip_verify: false  # WARNING: only for self-signed BMC certs, TLS verification disabled
    ca_cert_file: null         # optional PEM CA bundle for self-signed BMCs
```

- [ ] **Step 3: 更新 `README.md`**

- Quick start 段落之后、Endpoints 表之后加一句：`listen_addr` 默认值为 `127.0.0.1:9417`；远程抓取需显式绑定 `0.0.0.0`。
- Configuration 表 `listen_addr` 行默认值改为 `127.0.0.1:9417`，说明改为 `HTTP listen address; defaults to 127.0.0.1 for safe-by-default, set 0.0.0.0 explicitly for remote Prometheus`；新增 `web` 行：`null` / `Inbound hardening: auth_token (>=16 chars), auth_token_file (mutually exclusive), tls_cert_file + tls_key_file (must be set together)`。
- `bmcs[].password` 行说明追加：`can be overridden by env var REDFISH_EXPORTER_PASSWORD_<NAME> (name uppercased, non-alphanumerics replaced by _)`。
- Security 节替换为：

```markdown
## Security

- **Inbound hardening (optional, per docs/security.md)**: bearer-token auth on all endpoints (constant-time compare, `web.auth_token` / `web.auth_token_file`, >= 16 chars) and server-side TLS via rustls (`web.tls_cert_file` + `web.tls_key_file`). `listen_addr` defaults to `127.0.0.1:9417` — remote scraping requires an explicit `0.0.0.0` bind plus auth/TLS per the baseline matrix.
- **Secret redaction**: passwords are stored in a `SecretString` type whose `Debug` representation is `[REDACTED]` and whose buffer is zeroized on drop; they are never logged. Passwords can be injected via environment variables instead of the config file.
- **TLS**: rustls — no OpenSSL dependency. Custom CA bundles via `ca_cert_file`. https->http redirect downgrades are blocked; `insecure_skip_verify` is per-BMC opt-in for self-signed BMC certificates only — prefer `ca_cert_file`.
- **Supply chain**: `#![forbid(unsafe_code)]`, cargo-deny policy (`deny.toml`), cargo-audit allowlist with rationale (`.cargo/audit.toml`), non-root distroless container with read-only root filesystem.
```

- [ ] **Step 4: 更新 `docs/design.md`**

在「Config validation rules」清单追加两条（默认值条目同步改 `127.0.0.1:9417`）：

```
10. bmc `name` must not contain control characters; bmc `host` must not carry URL credentials (userinfo) — use `username`/`password` fields.
11. `web` section: `auth_token` (>=16 chars) XOR `auth_token_file` (content trimmed, >=16 chars); `tls_cert_file` and `tls_key_file` must be set together; passwords may be overridden by `REDFISH_EXPORTER_PASSWORD_<NAME>` (empty env value rejected, name collisions rejected).
```

「Authentication flow」节末尾追加「Inbound security」小节：

```
## Inbound security

- Optional bearer-token auth: when `web.auth_token`/`auth_token_file` is set, an axum middleware (`from_fn_with_state`) requires `Authorization: Bearer <token>` on every endpoint (incl. `/healthz` and `/reload`); comparison is constant-time (XOR fold, `src/auth.rs`); 401 carries `WWW-Authenticate: Bearer`. Token changes require a restart (consistent with the reload semantics).
- Optional server TLS: `serve()` loads a rustls config from `web.tls_cert_file`/`tls_key_file` (fail-fast on error) and serves via axum-server; a 10s HTTP/1.1 header read timeout applies in both modes. Outbound requests carry a custom redirect policy blocking https->http downgrades (10-hop limit).
- Pagination defense caps: 64MiB per page (post-deserialization check; nv typed fetch has no streaming API), 1000 pages, 200k accumulated members — violations fail the affected log service (resource-level isolation).
```

- [ ] **Step 5: 更新 `deploy/kubernetes/deployment.yaml`**

`listen_addr: "0.0.0.0:9417"` 行上方加注释：`# default is 127.0.0.1; 0.0.0.0 is required here so Prometheus can reach the pod — add web.auth_token/tls when the cluster is not a trusted network (docs/security.md)`。执行者先读该文件确认 YAML 缩进后修改。

- [ ] **Step 6: 提交**

```powershell
git add docs/security.md config.example.yaml README.md docs/design.md deploy/kubernetes/deployment.yaml
git commit -m "docs: security guide, baseline matrix, and config reference updates"
```

---

### Task 11: 全量验证与验收记录

**Files:**
- Create: `docs/audit/2026-08-21-security-hardening.md`
- 无代码改动

- [ ] **Step 1: 全量质量门禁**

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo doc --no-deps
```

Expected: 全绿。任何失败回到对应任务修复（修完重跑本命令）。

- [ ] **Step 2: 供应链门禁复查**

```powershell
cargo audit
cargo deny check
cargo tree | Select-String -Pattern "openssl|native-tls"
```

Expected: audit 仅 2 项白名单内 ignored；deny 全绿；tree 无 openssl/native-tls。

- [ ] **Step 3: 安全测试套件点名复查**

Run: `cargo test --test config_test --test http_test --test bmc_test --test pagination_test`
Expected: 全绿（后续四维回归基线即此四文件 + integration/scraper 全套）。

- [ ] **Step 4: 真机回归（有条件）**

按 `docs/audit/2026-08-21-acceptance.md` §1 配置（Dell 10.10.90.70 / 浪潮 10.10.90.80；注意新默认绑定需显式 `0.0.0.0` 或本机访问），跑 exporter 至少 2 轮：
- 验证快组 up=1、指标产出与验收报告一致（重定向策略与 TLS 改动未破坏采集）
- 开启 `web.auth_token` 冒烟：无 token 401、带 token 200
Expected: 通过则记录；真机不可达则如实记录「未执行 + 原因」，标记为投产前遗留跟踪（与验收报告惯例一致）。

- [ ] **Step 5: 撰写验收记录 `docs/audit/2026-08-21-security-hardening.md`**

```markdown
# 0.1.0 安全专项验收记录

日期：2026-08-21
依据：spec `docs/superpowers/specs/2026-08-21-security-hardening-design.md` §11 验收标准
范围：五维专项第 1 项（安全性）；成果构成后续四维回归基线

## 结论

**PASS / CONDITIONAL PASS / FAIL**（执行者按证据填写，附理由）

## 决策记录

D1 混合环境分级模型 / D2 文件+env 覆盖 / D3 Bearer+可选 TLS / D4 audit 白名单 /
D5 威胁模型+加固+测试+文档 / D6 默认 127.0.0.1 / D7 /reload 保持现状+文档 / D8 代理透传+文档

## 验收标准对照

| 标准（spec §11） | 结果 | 证据 |
|---|---|---|
| 1. 测试全绿 + clippy/fmt/deny/audit | | Step 1/2 命令输出摘录 |
| 2. 文档交付齐全一致 | | docs/security.md、README、config.example、design.md |
| 3. 真机回归 | | Step 4 结果（或未执行+原因） |
| 4. 语义冻结清单生效 | | spec §9.3 清单与实现对照 |

## 遗留跟踪

- quick-xml 构建期漏洞（白名单内，跟踪 nv-redfish 上游）
- 分页响应上限为反序列化后校验（nv 无流式 fetch），流式上限 backlog
- 认证/TLS 配置变更需重启生效（与 reload 语义一致，设计既定）
```

- [ ] **Step 6: 提交**

```powershell
git add docs/audit/2026-08-21-security-hardening.md
git commit -m "docs: 0.1.0 security hardening acceptance record"
```

---

## Self-Review 记录

- **Spec 覆盖**：§2 攻击面（Task 1/4/5/6/7/8 对应全部行）、§3 入站（Task 4/5）、§4 出站与凭据（Task 1/2/3/6/7/8）、§5 供应链（Task 9）、§6 测试（各任务 Step 1）、§7 文档（Task 10）、§9 回归门禁（Task 11 Step 3）、§10 风险（axum-server API 核对在 Task 5 注释；真机回归在 Task 11 Step 4）、§11 验收（Task 11）。
- **占位符**：无 TBD/TODO；所有步骤含实际代码或精确命令。
- **类型一致性**：`router(...)` 新签名在 Task 4 定义、Task 5 使用（一致）；`load_tls`/`serve_on`/`DEFAULT_HEADER_READ_TIMEOUT` 定义与测试一致；`fetch_all_pages_with_limits` 参数顺序与测试一致；`WebConfig` 字段名与 spec/config 示例一致。
