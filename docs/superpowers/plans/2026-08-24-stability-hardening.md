# 稳定性专项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 0.1.0 稳定性专项——分层稳定性模型：运行时韧性（崩溃面收敛）+ 自适应调度（退避冷却 + 会话降级兜底）+ 验证（故障注入/soak/CI 冒烟）。

**Architecture:** 新增 `src/stability.rs` 状态机模块（纯函数 seam：`BmcState`/`RoundAction`/`RoundOutcome`/`next_backoff`/`round_action`/`on_round_result`）；scraper.rs 提取 `run_bmc_round` 测试接缝并接入 per-BMC 状态机（`scrape_once(&mut self)` + `states: HashMap`）；冷却轮直接发布 up=0+resource=cooldown 不发请求；SessionDegraded 用 basic 凭据兜底采集 + session 按退避重试；崩溃韧性统一 Mutex 中毒恢复；验证层含真实 401 故障注入、`#[ignore]` soak（`sysinfo` dev-dep）、CI Linux SIGTERM job。

**Tech Stack:** Rust 1.90 / edition 2024、tokio、axum（测试服务器）、nv-redfish-bmc-mock、sysinfo（dev）、既有 prometheus/tracing。

**Spec:** `docs/superpowers/specs/2026-08-24-stability-hardening-design.md`（计划依 spec 论证，执行者须先读 spec）

## Global Constraints

- rust-version 1.90、edition 2024；`#![forbid(unsafe_code)]` 在 src/ 必须保持
- 每任务结束必须 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets` 全绿后才提交
- 代码注释用中文（仓库惯例）；提交信息用英文、仓库风格（`feat:`/`fix:`/`docs:`/`chore:`）
- 零 OpenSSL/native-tls；本计划新依赖仅 `sysinfo`（dev-dependency）；运行时零新依赖
- 安全回归（第 1 项成果不可破坏）：`cargo test --test config_test --test http_test --test bmc_test --test pagination_test` 必须全绿；cargo audit（白名单内零新增）/ deny 全绿
- 稳定性语义冻结（spec §6.2）：`stability` 配置节语义、冷却状态机行为、SessionDegraded 标记、§4.3 发布语义表——只增不改
- 精确值（spec 逐字）：`cooldown_failures=3`、`cooldown_base=60s`、`cooldown_max=300s`；退避序列 60/120/240/300s（`min(max, base × 2^n)`，n 从 1 起）；soak 默认 `SOAK_SECS=14400`、验收实跑 ≥7200；冷却轮不发请求、不累计 `redfish_scrape_errors_total`

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `src/stability.rs` | 新建 | 状态机：`BmcState`/`RoundAction`/`RoundOutcome`/`next_backoff`/`round_action`/`on_round_result`（纯函数） |
| `src/scraper.rs` | 改 | `run_bmc_round` 提取（pub 接缝）、`scrape_once(&mut self)` 状态机接线、`publish_cooldown`、`with_degraded_mark`、Mutex 中毒恢复 |
| `src/registry.rs` | 改 | Mutex/RwLock 中毒恢复（6 处） |
| `src/http.rs` | 改 | `unreachable!` → Err |
| `src/main.rs` | 改 | 信号 `expect` → `error!`+`exit(1)` |
| `src/lib.rs` | 改 | 注册 `stability` 模块；`pub(crate) fn recover_lock` |
| `src/config.rs` | 改 | `StabilityConfig` + `stability` 节 + 校验 |
| `tests/stability_test.rs` | 新建 | 状态机纯函数单测 + 真实 401 故障注入 |
| `tests/scraper_test.rs` | 改 | `run_bmc_round` 相关 seam 测试、cooldown/degraded 发布 seam 测试 |
| `tests/common/mod.rs` | 新建 | MockBmc 期望辅助（自 integration_test.rs 抽取） |
| `tests/integration_test.rs` | 改 | 改用 common 模块（删除本地重复 helper） |
| `tests/soak_test.rs` | 新建 | `#[ignore]` soak（`SOAK_SECS` 参数化） |
| `.github/workflows/ci.yml` | 改 | Linux SIGTERM 冒烟 job |
| `docs/design.md` / `README.md` / `config.example.yaml` / `docs/audit/2026-08-24-stability.md` | 改/新建 | 文档交付 |

---

### Task 1: 崩溃韧性收敛（Mutex 中毒恢复 + unreachable/expect 消除）

**Files:**
- Modify: `src/lib.rs`（recover_lock + #[cfg(test)]）、`src/registry.rs:33-88`、`src/scraper.rs`（7 处 `.unwrap()`）、`src/http.rs:161-167`、`src/main.rs:97-107`、`src/metrics.rs:150-192`（注释）

**Interfaces:**
- Produces: `crate::recover_lock<T>(r: std::sync::LockResult<T>) -> T`（pub(crate)，恢复中毒锁 + `error!` 日志）
- 后续任务消费：Task 4 在 scraper 新增代码中沿用 `recover_lock`

- [ ] **Step 1: 写失败测试**

`src/lib.rs` 末尾追加 `#[cfg(test)] mod tests`（同文件单元测试可访问 pub(crate)，且能直接制造中毒）：

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::recover_lock;

    #[test]
    fn recover_lock_returns_guard_after_poisoning() {
        let m = Arc::new(Mutex::new(0u32));
        let holder = m.lock().unwrap();
        let m2 = Arc::clone(&m);
        // 持锁线程 panic → mutex 中毒（线程先阻塞在 lock 上，主线程 drop(holder) 后取得守卫再 panic）
        let t = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison while holding the guard");
        });
        drop(holder);
        let _ = t.join();
        // 锁已中毒：lock() 返回 Err(PoisonError)
        let r = m.lock();
        assert!(r.is_err());
        // recover_lock 恢复出守卫，值完整
        let recovered = recover_lock(r);
        assert_eq!(*recovered, 0);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib recover_lock`
Expected: FAIL（`recover_lock` 不存在）

- [ ] **Step 3: 实现 recover_lock 与各处替换**

`src/lib.rs` 追加：

```rust
/// 恢复被 panic 污染的锁守卫：中毒说明曾有任务在持锁时 panic，
/// 恢复后继续服务（避免连锁 panic），同时记录 error 日志留痕。
pub(crate) fn recover_lock<T>(r: std::sync::LockResult<T>) -> T {
    r.unwrap_or_else(|p| {
        tracing::error!("recovered from a poisoned lock (a task panicked while holding it)");
        p.into_inner()
    })
}
```

`src/registry.rs`：6 处 `.expect("... poisoned")` 全部替换为 `recover_lock(...)` 包裹（例）：

```rust
        *recover_lock(self.errors.lock())
            .entry(bmc.to_string())
            .or_insert(0) += count;
```

其余 5 处同理（`scrape_errors` 的 `.lock()`、`update`/`registry`/`is_empty`/`registries` 的 `.read()`/`.write()`）。顶部 `use crate::recover_lock;`。

`src/scraper.rs`：7 处 `sessions.lock().unwrap()` / `slow_state.lock().unwrap()` 全部替换为 `recover_lock(...)`（`use crate::recover_lock;`）。

`src/http.rs` `load_tls` 末尾分支：

```rust
        _ => Err(anyhow::anyhow!(
            "web: tls_cert_file and tls_key_file must be set together (validated in load_config)"
        )),
```

（替代原 `unreachable!`；`anyhow::Context` 已导入。）

`src/main.rs` 信号安装：

```rust
async fn shutdown_signal() {
    let ctrl_c = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {}
            Err(e) => {
                tracing::error!(error = %e, "failed to install Ctrl+C handler");
                std::process::exit(1);
            }
        }
    };
```

`#[cfg(unix)]` 的 SIGTERM 分支同法替换 `expect`。

`src/metrics.rs` 两处 `.expect(...)` 保留，行上方各加中文注释：

```rust
// 不变量：GaugeVec 以 'static 字面量名构造，不可能失败；失败即程序缺陷，panic 合理。
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib`; `cargo test --test scraper_test`; 全量 `cargo test --all-targets`; `cargo clippy --all-targets -- -D warnings`; `cargo fmt --check`
Expected: 全绿

- [ ] **Step 5: 提交**

```powershell
git add src/lib.rs src/registry.rs src/scraper.rs src/http.rs src/main.rs src/metrics.rs
git commit -m "feat: poison-safe lock recovery and panic-free error paths"
```

---

### Task 2: `stability` 配置节（StabilityConfig + 校验）

**Files:**
- Modify: `src/config.rs`（StabilityConfig/Raw/默认/校验/Config 字段）
- Test: `tests/config_test.rs`（追加）、`tests/http_test.rs`（test_config 字面量补字段）

**Interfaces:**
- Produces: `pub struct StabilityConfig { pub cooldown_failures: u32, pub cooldown_base: Duration, pub cooldown_max: Duration }`（`#[derive(Debug, Clone)]`）；`Config.stability: StabilityConfig`
- 后续任务消费：Task 3/4（状态机与 scraper 读 `cfg.stability`）

- [ ] **Step 1: 写失败测试**

`tests/config_test.rs` 追加：

```rust
#[test]
fn stability_defaults_are_applied() {
    let p = write_tmp(
        "stab_default",
        "bmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.stability.cooldown_failures, 3);
    assert_eq!(cfg.stability.cooldown_base, std::time::Duration::from_secs(60));
    assert_eq!(cfg.stability.cooldown_max, std::time::Duration::from_secs(300));
}

#[test]
fn stability_section_parses() {
    let p = write_tmp(
        "stab_parse",
        "stability:\n  cooldown_failures: 5\n  cooldown_base: \"30s\"\n  cooldown_max: \"600s\"\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    let cfg = load_config_with_env(&p, |_| None).unwrap();
    assert_eq!(cfg.stability.cooldown_failures, 5);
    assert_eq!(cfg.stability.cooldown_base, std::time::Duration::from_secs(30));
    assert_eq!(cfg.stability.cooldown_max, std::time::Duration::from_secs(600));
}

#[test]
fn stability_rejects_zero_failures() {
    let p = write_tmp(
        "stab_zero_f",
        "stability:\n  cooldown_failures: 0\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config_with_env(&p, |_| None), Err(ConfigError::Invalid(_))));
}

#[test]
fn stability_rejects_max_less_than_base() {
    let p = write_tmp(
        "stab_max_lt_base",
        "stability:\n  cooldown_base: \"300s\"\n  cooldown_max: \"60s\"\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config_with_env(&p, |_| None), Err(ConfigError::Invalid(_))));
}

#[test]
fn stability_rejects_zero_durations() {
    let p = write_tmp(
        "stab_zero_d",
        "stability:\n  cooldown_base: \"0s\"\nbmcs:\n  - { name: a, host: https://h1, username: u, password: \"p\" }\n",
    );
    assert!(matches!(load_config_with_env(&p, |_| None), Err(ConfigError::Invalid(_))));
}
```

`tests/http_test.rs` 的 `test_config` 中 `Config {` 字面量补 `stability: redfish_exporter::config::StabilityConfig { cooldown_failures: 3, cooldown_base: Duration::from_secs(60), cooldown_max: Duration::from_secs(300) },`（use 区已导入 `Duration`）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test config_test stability_`
Expected: FAIL（`stability` 节不存在）

- [ ] **Step 3: 实现**

`src/config.rs`：

```rust
#[derive(Debug, Clone)]
pub struct StabilityConfig {
    pub cooldown_failures: u32,
    pub cooldown_base: Duration,
    pub cooldown_max: Duration,
}
```

`Config` 增加字段 `pub stability: StabilityConfig`。

`RawConfig` 增加 `#[serde(default)] stability: RawStabilityConfig`，并新增：

```rust
#[derive(Deserialize, Default)]
struct RawStabilityConfig {
    #[serde(default = "default_cooldown_failures")]
    cooldown_failures: u32,
    #[serde(default = "default_cooldown_base", deserialize_with = "deserialize_duration")]
    cooldown_base: Duration,
    #[serde(default = "default_cooldown_max", deserialize_with = "deserialize_duration")]
    cooldown_max: Duration,
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
```

`load_config_with_env` 末尾（`Ok(Config { ... })` 前）加校验：

```rust
    if raw.stability.cooldown_failures == 0 {
        return Err(ConfigError::Invalid("stability.cooldown_failures must be > 0".into()));
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
```

`Ok(Config { ... })` 构造追加：

```rust
        stability: StabilityConfig {
            cooldown_failures: raw.stability.cooldown_failures,
            cooldown_base: raw.stability.cooldown_base,
            cooldown_max: raw.stability.cooldown_max,
        },
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test config_test`; `cargo test --test http_test`; 全量门禁
Expected: 全绿

- [ ] **Step 5: 提交**

```powershell
git add src/config.rs tests/config_test.rs tests/http_test.rs
git commit -m "feat: stability config section (cooldown thresholds and backoff bounds)"
```

---

### Task 3: 状态机模块（src/stability.rs 纯函数）

**Files:**
- Create: `src/stability.rs`
- Modify: `src/lib.rs`（`pub mod stability;`）
- Test: `tests/stability_test.rs`（新建，本任务只含纯函数测试）

**Interfaces:**
- Consumes: Task 2 的 `StabilityConfig`（`use crate::config::StabilityConfig;`）
- Produces（Task 4/5 依赖，签名精确）：

```rust
pub enum BmcState {
    Healthy { failures: u32 },
    SessionDegraded { basic_failures: u32, session_failures: u32, next_session_retry: Instant },
    Cooling { failures: u32, next_attempt: Instant },
}
pub enum RoundAction { Scrape, SessionRetry, CooldownWait }
pub struct RoundOutcome {
    pub report: Option<crate::collector::ScrapeReport>,
    pub message: String,
    pub session_recovery_failed: bool,
    pub attempted_session: bool,
}
pub fn next_backoff(failures: u32, base: Duration, max: Duration) -> Duration
pub fn round_action(state: &BmcState, now: Instant) -> RoundAction
pub fn on_round_result(state: &BmcState, outcome: &RoundOutcome, failed: bool, cfg: &StabilityConfig, now: Instant) -> BmcState
```

（`failed` 参数：由调用方归一化——`outcome.report` 为 None 或 `report.failed_resources` 非空（含快/慢组超时、up=0 轮）均视为失败轮，与 spec §4.3 发布语义表一致。）

- [ ] **Step 1: 写失败测试**（`tests/stability_test.rs`）

```rust
use redfish_exporter::config::StabilityConfig;
use redfish_exporter::stability::{BmcState, RoundAction, RoundOutcome, next_backoff, on_round_result, round_action};
use std::time::{Duration, Instant};

fn cfg() -> StabilityConfig {
    StabilityConfig {
        cooldown_failures: 3,
        cooldown_base: Duration::from_secs(60),
        cooldown_max: Duration::from_secs(300),
    }
}

fn ok_outcome() -> RoundOutcome {
    RoundOutcome {
        report: Some(redfish_exporter::collector::ScrapeReport { metrics: vec![], failed_resources: vec![] }),
        message: String::new(),
        session_recovery_failed: false,
        attempted_session: false,
    }
}

fn err_outcome(session: bool, attempted: bool) -> RoundOutcome {
    RoundOutcome {
        report: None,
        message: "fail".into(),
        session_recovery_failed: session,
        attempted_session: attempted,
    }
}

#[test]
fn backoff_sequence_matches_spec() {
    let (b, m) = (Duration::from_secs(60), Duration::from_secs(300));
    assert_eq!(next_backoff(1, b, m), Duration::from_secs(60));
    assert_eq!(next_backoff(2, b, m), Duration::from_secs(120));
    assert_eq!(next_backoff(3, b, m), Duration::from_secs(240));
    assert_eq!(next_backoff(4, b, m), Duration::from_secs(300));
    assert_eq!(next_backoff(10, b, m), Duration::from_secs(300));
}

#[test]
fn round_action_healthy_scrapes() {
    assert_eq!(round_action(&BmcState::Healthy { failures: 0 }, Instant::now()), RoundAction::Scrape);
}

#[test]
fn round_action_cooling_waits_then_scrapes() {
    let now = Instant::now();
    let c = BmcState::Cooling { failures: 3, next_attempt: now + Duration::from_secs(60) };
    assert_eq!(round_action(&c, now), RoundAction::CooldownWait);
    assert_eq!(round_action(&c, now + Duration::from_secs(59)), RoundAction::CooldownWait);
    assert_eq!(round_action(&c, now + Duration::from_secs(61)), RoundAction::Scrape);
}

#[test]
fn round_action_degraded_retries_session_on_schedule() {
    let now = Instant::now();
    let d = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 1,
        next_session_retry: now + Duration::from_secs(60),
    };
    assert_eq!(round_action(&d, now), RoundAction::Scrape);
    assert_eq!(round_action(&d, now + Duration::from_secs(61)), RoundAction::SessionRetry);
}

#[test]
fn healthy_failures_count_to_cooldown() {
    let now = Instant::now();
    let mut s = BmcState::Healthy { failures: 0 };
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 1 });
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 2 });
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert!(matches!(s, BmcState::Cooling { failures: 3, .. }));
}

#[test]
fn healthy_success_resets_counter() {
    let now = Instant::now();
    let s = BmcState::Healthy { failures: 2 };
    let s = on_round_result(&s, &ok_outcome(), false, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 0 });
}

#[test]
fn healthy_session_recovery_failure_degrades() {
    let now = Instant::now();
    let s = BmcState::Healthy { failures: 0 };
    let s = on_round_result(&s, &err_outcome(true, true), true, &cfg(), now);
    match s {
        BmcState::SessionDegraded { basic_failures: 0, session_failures: 1, .. } => {}
        other => panic!("expected SessionDegraded, got {other:?}"),
    }
}

#[test]
fn cooling_retry_success_recovers() {
    let now = Instant::now();
    let s = BmcState::Cooling { failures: 3, next_attempt: now };
    let s = on_round_result(&s, &ok_outcome(), false, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 0 });
}

#[test]
fn cooling_retry_failure_extends_backoff() {
    let now = Instant::now();
    let s = BmcState::Cooling { failures: 3, next_attempt: now };
    let s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    match s {
        BmcState::Cooling { failures: 4, next_attempt } => {
            assert_eq!(next_attempt, now + Duration::from_secs(300));
        }
        other => panic!("expected Cooling, got {other:?}"),
    }
}

#[test]
fn degraded_basic_success_stays_degraded_with_reset_basic_failures() {
    let now = Instant::now();
    let s = BmcState::SessionDegraded {
        basic_failures: 2,
        session_failures: 1,
        next_session_retry: now + Duration::from_secs(60),
    };
    let s = on_round_result(&s, &ok_outcome(), false, &cfg(), now);
    match s {
        BmcState::SessionDegraded { basic_failures: 0, session_failures: 1, .. } => {}
        other => panic!("expected SessionDegraded, got {other:?}"),
    }
}

#[test]
fn degraded_session_retry_success_recovers_to_healthy() {
    let now = Instant::now();
    let s = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 2,
        next_session_retry: now,
    };
    let mut ok = ok_outcome();
    ok.attempted_session = true;
    let s = on_round_result(&s, &ok, false, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 0 });
}

#[test]
fn degraded_session_retry_failure_but_basic_ok_keeps_degraded() {
    let now = Instant::now();
    let s = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 2,
        next_session_retry: now,
    };
    let mut ok = ok_outcome();
    ok.attempted_session = true;
    ok.session_recovery_failed = true;
    let s = on_round_result(&s, &ok, false, &cfg(), now);
    match s {
        BmcState::SessionDegraded { basic_failures: 0, session_failures: 3, .. } => {}
        other => panic!("expected SessionDegraded with session_failures=3, got {other:?}"),
    }
}

#[test]
fn degraded_basic_failures_reach_cooldown() {
    let now = Instant::now();
    let mut s = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 1,
        next_session_retry: now + Duration::from_secs(60),
    };
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert!(matches!(s, BmcState::Cooling { .. }));
}
```

（`BmcState` 需派生 `Debug, Clone, PartialEq`；`RoundAction` 派生 `Debug, PartialEq`。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test stability_test`
Expected: FAIL（模块/类型不存在）

- [ ] **Step 3: 实现 src/stability.rs**

```rust
//! per-BMC 稳定性状态机（spec `docs/superpowers/specs/2026-08-24-stability-hardening-design.md` §4）。
//! 纯函数 + 数据，供 scraper 调度与单元测试（与 scraper.rs 的 slow_due 等测试接缝同法）。

use std::time::{Duration, Instant};

use crate::collector::ScrapeReport;
use crate::config::StabilityConfig;

/// 单 BMC 稳定状态（spec §4.1 状态机）。
#[derive(Debug, Clone, PartialEq)]
pub enum BmcState {
    /// 正常采集；failures 为连续失败轮计数。
    Healthy { failures: u32 },
    /// 会话降级：basic 凭据兜底采集（每轮），session 按退避节奏重试。
    SessionDegraded {
        basic_failures: u32,
        session_failures: u32,
        next_session_retry: Instant,
    },
    /// 全停冷却：到期重试（指数退避）。
    Cooling { failures: u32, next_attempt: Instant },
}

/// 一轮调度动作（scraper 在 spawn 前按状态与时钟判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundAction {
    /// 正常采集轮（Healthy / Cooling 到期重试 / Degraded 的 basic 轮）。
    Scrape,
    /// Degraded 且 session 退避到期：先试 session 重挂，失败则 basic 兜底（同轮内）。
    SessionRetry,
    /// 冷却等待：本轮不发任何请求，直接发布 up=0 + resource=cooldown。
    CooldownWait,
}

/// 单轮执行结果（run_bmc_round 产出）。
#[derive(Debug)]
pub struct RoundOutcome {
    /// 成功轮的报告（含 up=0 的部分失败轮；失败判定由调用方用 `failed` 归一化）。
    pub report: Option<ScrapeReport>,
    /// 失败轮的描述（timeout/bmc 等，与既有失败报告语义一致）。
    pub message: String,
    /// 本轮尝试了 session 建立/重登且失败（401 重登失败或会话重挂失败）。
    pub session_recovery_failed: bool,
    /// 本轮是否尝试过 session 建立/重登（Healthy 轮/Cooling 重试/SessionRetry 为 true，basic 兜底轮为 false）。
    pub attempted_session: bool,
}

/// 指数退避：min(max, base × 2^failures)；failures 从 1 起（spec：60/120/240/300s 封顶）。
pub fn next_backoff(failures: u32, base: Duration, max: Duration) -> Duration {
    let factor = 1u64.checked_shl(failures.min(31)).unwrap_or(u64::MAX);
    base.saturating_mul(factor).min(max)
}

/// 判定本轮动作。
pub fn round_action(state: &BmcState, now: Instant) -> RoundAction {
    match state {
        BmcState::Healthy { .. } => RoundAction::Scrape,
        BmcState::Cooling { next_attempt, .. } => {
            if now >= *next_attempt {
                RoundAction::Scrape
            } else {
                RoundAction::CooldownWait
            }
        }
        BmcState::SessionDegraded {
            next_session_retry, ..
        } => {
            if now >= *next_session_retry {
                RoundAction::SessionRetry
            } else {
                RoundAction::Scrape
            }
        }
    }
}

/// 应用一轮结果推进状态机。
/// `failed`：本轮是否失败（report 为 None 或 failed_resources 非空）。
pub fn on_round_result(
    state: &BmcState,
    outcome: &RoundOutcome,
    failed: bool,
    cfg: &StabilityConfig,
    now: Instant,
) -> BmcState {
    let ok = !failed && outcome.report.is_some();
    match state {
        BmcState::Healthy { failures } => {
            if ok {
                BmcState::Healthy { failures: 0 }
            } else if outcome.session_recovery_failed {
                BmcState::SessionDegraded {
                    basic_failures: 0,
                    session_failures: 1,
                    next_session_retry: now + next_backoff(1, cfg.cooldown_base, cfg.cooldown_max),
                }
            } else {
                let f = failures + 1;
                if f >= cfg.cooldown_failures {
                    BmcState::Cooling {
                        failures: f,
                        next_attempt: now + next_backoff(f, cfg.cooldown_base, cfg.cooldown_max),
                    }
                } else {
                    BmcState::Healthy { failures: f }
                }
            }
        }
        BmcState::Cooling { failures, .. } => {
            if ok {
                BmcState::Healthy { failures: 0 }
            } else {
                let f = failures + 1;
                BmcState::Cooling {
                    failures: f,
                    next_attempt: now + next_backoff(f, cfg.cooldown_base, cfg.cooldown_max),
                }
            }
        }
        BmcState::SessionDegraded {
            basic_failures,
            session_failures,
            next_session_retry,
        } => {
            if ok {
                if outcome.attempted_session && !outcome.session_recovery_failed {
                    // session 重挂成功
                    BmcState::Healthy { failures: 0 }
                } else if outcome.attempted_session {
                    // session 仍失败但 basic 成功：继续降级，session 退避推进
                    let sf = session_failures + 1;
                    BmcState::SessionDegraded {
                        basic_failures: 0,
                        session_failures: sf,
                        next_session_retry: now + next_backoff(sf, cfg.cooldown_base, cfg.cooldown_max),
                    }
                } else {
                    // 纯 basic 轮成功
                    BmcState::SessionDegraded {
                        basic_failures: 0,
                        session_failures: *session_failures,
                        next_session_retry: *next_session_retry,
                    }
                }
            } else {
                let bf = basic_failures + 1;
                let sf = if outcome.attempted_session {
                    session_failures + 1
                } else {
                    *session_failures
                };
                let nxt = if outcome.attempted_session {
                    now + next_backoff(sf, cfg.cooldown_base, cfg.cooldown_max)
                } else {
                    *next_session_retry
                };
                if bf >= cfg.cooldown_failures {
                    BmcState::Cooling {
                        failures: bf,
                        next_attempt: now + next_backoff(bf, cfg.cooldown_base, cfg.cooldown_max),
                    }
                } else {
                    BmcState::SessionDegraded {
                        basic_failures: bf,
                        session_failures: sf,
                        next_session_retry: nxt,
                    }
                }
            }
        }
    }
}
```

`src/lib.rs`：`pub mod stability;`（在 `pub mod scraper;` 之后）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test stability_test`
Expected: 全绿（14 个测试）

- [ ] **Step 5: 提交**

```powershell
git add src/stability.rs src/lib.rs tests/stability_test.rs
git commit -m "feat: per-BMC stability state machine (cooldown backoff, session degradation)"
```

---

### Task 4: scraper 集成（run_bmc_round 提取 + 状态机接线 + 发布语义）

**Files:**
- Modify: `src/scraper.rs`（BmcHandle 派生 Clone、Scraper 增加 stability/states 字段、run_bmc_round 提取、scrape_once 改造、publish_cooldown、with_degraded_mark）
- Test: `tests/scraper_test.rs`（追加 publish seam 测试）

**Interfaces:**
- Consumes: Task 3 的 `BmcState`/`RoundAction`/`RoundOutcome`/`round_action`/`on_round_result`；Task 1 的 `recover_lock`
- Produces（Task 5 依赖）：

```rust
pub async fn run_bmc_round(
    handle: &BmcHandle,
    slow_interval: Option<Duration>,
    slow_state: &SlowState,
    sessions: &SessionStore,
    timeout: Duration,
    attempt_session: bool,
    use_basic_only: bool,
) -> RoundOutcome
pub fn cooldown_report() -> ScrapeReport            // metrics 空 + failed_resources=["cooldown"]
pub fn with_degraded_mark(report: ScrapeReport, bmc_name: &str) -> ScrapeReport  // 追加 redfish_scrape_error{resource="session-degraded"}=1
```

- [ ] **Step 1: 写失败测试**（`tests/scraper_test.rs` 追加）

```rust
use redfish_exporter::scraper::{cooldown_report, with_degraded_mark};

#[test]
fn cooldown_report_marks_cooldown_resource() {
    let r = cooldown_report();
    assert!(r.metrics.is_empty());
    assert_eq!(r.failed_resources, vec!["cooldown".to_string()]);
}

#[test]
fn degraded_mark_appends_scrape_error_series() {
    let r = ScrapeReport { metrics: vec![], failed_resources: vec![] };
    let r = with_degraded_mark(r, "bmc1");
    assert_eq!(r.failed_resources.len(), 0);
    assert_eq!(r.metrics.len(), 1);
    let m = &r.metrics[0];
    assert_eq!(m.name, "redfish_scrape_error");
    assert!(m.labels.iter().any(|(k, v)| *k == "resource" && v == "session-degraded"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test scraper_test cooldown_report`
Expected: FAIL（函数不存在）

- [ ] **Step 3: 实现（src/scraper.rs 改造）**

顶部 use 增加：

```rust
use crate::metrics::{Metric, SCRAPE_ERROR, UP};
use crate::stability::{BmcState, RoundAction, RoundOutcome, on_round_result, round_action};
```

`BmcHandle` 增加 `#[derive(Clone)]`。

`Scraper` 结构体增加字段：

```rust
    stability: StabilityConfig,
    states: std::collections::HashMap<String, BmcState>,
```

`Scraper::new` 末尾构造增加：

```rust
            stability: cfg.stability.clone(),
            states: std::collections::HashMap::new(),
```

（use 区增加 `crate::config::{AuthMethod, Config, StabilityConfig};`）

`run(self)` 中 `self.scrape_once().await;` 改为同名但签名 `&mut self`（run 拥有 self，可行）。

新增两个纯函数（放在 `slow_due` 附近）：

```rust
/// 冷却等待轮的发布报告：up=0 + redfish_scrape_error{resource="cooldown"}（经 build_registry 生成），
/// 不累计 scrape_errors_total。
pub fn cooldown_report() -> ScrapeReport {
    ScrapeReport {
        metrics: vec![],
        failed_resources: vec!["cooldown".to_string()],
    }
}

/// 会话降级标记：basic 兜底成功轮追加 redfish_scrape_error{resource="session-degraded"}=1
/// （保持 up=1——failed_resources 为空不会触发 build_registry 的 up=0 覆盖）。
pub fn with_degraded_mark(mut report: ScrapeReport, bmc_name: &str) -> ScrapeReport {
    report.metrics.push(
        Metric::gauge(SCRAPE_ERROR.0, SCRAPE_ERROR.1)
            .label("bmc", bmc_name.to_string())
            .label("resource", "session-degraded".to_string())
            .build(1.0),
    );
    report
}
```

提取 `run_bmc_round`（把现有任务闭包体移入；替换 7 处 `.unwrap()` 为 `recover_lock(...)`）：

```rust
/// 单 BMC 单轮执行（含会话建立/401 重登），提取为 pub 接缝供真实 HTTP 故障注入测试（Task 5）。
/// attempt_session：本轮是否尝试 session 建立/重登；use_basic_only：Degraded 的 basic 兜底轮（先切 basic 凭据）。
pub async fn run_bmc_round(
    handle: &BmcHandle,
    slow_interval: Option<Duration>,
    slow_state: &SlowState,
    sessions: &SessionStore,
    timeout: Duration,
    attempt_session: bool,
    use_basic_only: bool,
) -> RoundOutcome {
    let name = handle.name.clone();
    let bmc = Arc::clone(&handle.bmc);
    let username = handle.username.clone();
    let password = handle.password.clone();
    let auth = handle.auth;
    let session_established = Arc::clone(&handle.session_established);

    if use_basic_only {
        bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::username_password(
            username.clone(),
            Some(password.expose().to_string()),
        ));
    }
    if auth == AuthMethod::Session
        && attempt_session
        && !session_established.load(Ordering::SeqCst)
    {
        if let Ok(est) = establish_session(&bmc, &username, password.expose()).await {
            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(est.token));
            session_established.store(true, Ordering::SeqCst);
            recover_lock(sessions.lock()).insert(name.clone(), est.session);
            info!(bmc = %name, "session established on first round");
        } else {
            warn!(bmc = %name, "session establishment failed, continuing with basic auth");
            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::username_password(
                username.clone(),
                Some(password.expose().to_string()),
            ));
        }
    }
    match collect_round(
        Arc::clone(&bmc),
        &name,
        slow_interval,
        Arc::clone(slow_state),
        timeout,
    )
    .await
    {
        Ok(report) => RoundOutcome {
            report: Some(report),
            message: String::new(),
            session_recovery_failed: false,
            attempted_session,
        },
        Err(err) if auth == AuthMethod::Session && attempt_session && is_unauthorized(&err) => {
            info!(bmc = %name, "session rejected with 401, re-establishing session");
            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::username_password(
                username.clone(),
                Some(password.expose().to_string()),
            ));
            match establish_session(&bmc, &username, password.expose()).await {
                Ok(est) => {
                    bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(est.token));
                    recover_lock(sessions.lock()).insert(name.clone(), est.session);
                    info!(bmc = %name, "session re-established, retrying scrape");
                    match collect_round(
                        Arc::clone(&bmc),
                        &name,
                        slow_interval,
                        Arc::clone(slow_state),
                        timeout,
                    )
                    .await
                    {
                        Ok(report) => RoundOutcome {
                            report: Some(report),
                            message: String::new(),
                            session_recovery_failed: false,
                            attempted_session: true,
                        },
                        Err(e) => RoundOutcome {
                            report: None,
                            message: e.to_string(),
                            session_recovery_failed: false,
                            attempted_session: true,
                        },
                    }
                }
                Err(e) => {
                    warn!(bmc = %name, error = %e, "session re-establishment failed");
                    RoundOutcome {
                        report: None,
                        message: err.to_string(),
                        session_recovery_failed: true,
                        attempted_session: true,
                    }
                }
            }
        }
        Err(err) => RoundOutcome {
            report: None,
            message: err.to_string(),
            session_recovery_failed: false,
            attempted_session,
        },
    }
}
```

（注意：attempted_session 在 Healthy 成功轮即 `attempt_session` 入参值；Degraded basic 轮为 false——由调用方传入。）

`scrape_once` 整体改造为：

```rust
    async fn scrape_once(&mut self) {
        let started = Instant::now();
        let bmc_count = self.bmcs.len();
        let timeout = self.timeout;
        let slow_interval = self.slow_interval;
        let stability = self.stability.clone();
        let slow_state = Arc::clone(&self.slow_state);
        let sessions = Arc::clone(&self.sessions);
        let now = Instant::now();
        let mut set = tokio::task::JoinSet::new();
        let mut cooldowns = Vec::new();
        for handle in &self.bmcs {
            let state = self
                .states
                .entry(handle.name.clone())
                .or_insert(BmcState::Healthy { failures: 0 });
            let action = round_action(state, now);
            if matches!(action, RoundAction::CooldownWait) {
                cooldowns.push(handle.name.clone());
                continue;
            }
            let (attempt_session, use_basic_only) = match (action, &*state) {
                (RoundAction::SessionRetry, _) => (true, false),
                (RoundAction::Scrape, BmcState::SessionDegraded { .. }) => (false, true),
                (RoundAction::Scrape, _) => (true, false),
                (RoundAction::CooldownWait, _) => unreachable!(),
            };
            let handle = handle.clone();
            let slow_state = Arc::clone(&slow_state);
            let sessions = Arc::clone(&sessions);
            set.spawn(async move {
                let outcome = run_bmc_round(
                    &handle,
                    slow_interval,
                    &slow_state,
                    &sessions,
                    timeout,
                    attempt_session,
                    use_basic_only,
                )
                .await;
                (handle.name.clone(), outcome)
            });
        }
        for name in &cooldowns {
            // 冷却等待轮：不发请求，直接发布 up=0 + resource=cooldown，不累计 errors_total。
            match build_registry(name, &cooldown_report(), self.snapshot.scrape_errors(name)).await {
                Ok(reg) => {
                    self.snapshot.update(name, reg);
                    info!(bmc = %name, "bmc in cooldown, round skipped");
                }
                Err(e) => warn!(bmc = %name, error = %e, "failed to build cooldown registry"),
            }
        }
        while let Some(res) = set.join_next().await {
            match res {
                Ok((name, outcome)) => {
                    let state = self
                        .states
                        .entry(name.clone())
                        .or_insert(BmcState::Healthy { failures: 0 });
                    let failed = outcome
                        .report
                        .as_ref()
                        .map(|r| !r.failed_resources.is_empty())
                        .unwrap_or(true);
                    *state = on_round_result(state, &outcome, failed, &stability, Instant::now());
                    match outcome.report {
                        Some(report) => {
                            let degraded_mark = matches!(state, BmcState::SessionDegraded { .. })
                                && outcome.attempted_session
                                && outcome.session_recovery_failed;
                            let report = if degraded_mark {
                                with_degraded_mark(report, &name)
                            } else {
                                report
                            };
                            let n = report.failed_resources.len() as u64;
                            self.snapshot.record_scrape_errors(&name, n);
                            match build_registry(&name, &report, self.snapshot.scrape_errors(&name)).await {
                                Ok(reg) => {
                                    self.snapshot.update(&name, reg);
                                    info!(
                                        bmc = %name,
                                        failed = report.failed_resources.len(),
                                        state = ?state,
                                        "scrape complete"
                                    );
                                }
                                Err(e) => warn!(bmc = %name, error = %e, "failed to build registry"),
                            }
                        }
                        None => {
                            let timed_out = outcome.message.contains("timeout");
                            let report = ScrapeReport {
                                metrics: vec![
                                    Metric::gauge(UP.0, UP.1)
                                        .label("bmc", name.clone())
                                        .build(0.0),
                                ],
                                failed_resources: if timed_out {
                                    vec!["timeout".to_string()]
                                } else {
                                    vec!["bmc".to_string()]
                                },
                            };
                            self.snapshot.record_scrape_errors(&name, 1);
                            match build_registry(&name, &report, self.snapshot.scrape_errors(&name)).await {
                                Ok(reg) => self.snapshot.update(&name, reg),
                                Err(e) => warn!(bmc = %name, error = %e, "failed to build registry"),
                            }
                            if timed_out {
                                warn!(bmc = %name, state = ?state, "scrape timed out after {:?}", timeout);
                            } else {
                                warn!(bmc = %name, error = %outcome.message, state = ?state, "scrape failed");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "scrape task panicked");
                }
            }
        }
        info!(
            bmc_count,
            duration_ms = started.elapsed().as_millis(),
            "scrape round complete"
        );
    }
```

（原任务闭包体（旧 scraper.rs:133-208）删除，逻辑迁入 `run_bmc_round`；`collect_round` 与 `slow_due` 等保持不变。原 `use crate::registry::build_registry` 与 `Metric, UP` 保持导入。）

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test scraper_test`; `cargo test --test stability_test`; `cargo test --test integration_test`; 全量门禁
Expected: 全绿（既有 integration_test 全流程仍通过——Healthy 路径行为不变）

- [ ] **Step 5: 提交**

```powershell
git add src/scraper.rs tests/scraper_test.rs
git commit -m "feat: wire stability state machine into the scrape loop"
```

---

### Task 5: 故障注入矩阵（真实 401 + 乱码响应 + 端到端转换）

**Files:**
- Test: `tests/stability_test.rs`（追加）

**Interfaces:**
- Consumes: Task 4 的 `run_bmc_round`、`BmcHandle`（含 `build_http_client`/`make_bmc`）、Task 3 状态机

- [ ] **Step 1: 写失败测试**

追加到 `tests/stability_test.rs`（use 区增加：`use axum::{Router, routing::get};`、`use redfish_exporter::bmc::{BmcHandle, build_http_client, make_bmc};`、`use redfish_exporter::config::{AuthMethod, BmcConfig, SecretString};`、`use url::Url;`）：

```rust
/// 起一个真实 HTTP 服务器：所有请求返回 401。
fn spawn_server(status: axum::http::StatusCode) -> (String, tokio::task::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(move || async move {
        (status, "unauthorized")
    });
    let handle = tokio::spawn(async move {
        axum_server::Server::from_tcp(listener)
            .unwrap()
            .serve(app.into_make_service())
            .await
            .unwrap();
    });
    (format!("http://{addr}"), handle)
}

fn session_handle(host: &str) -> (BmcHandle, String) {
    let cfg = BmcConfig {
        name: "inj".into(),
        host: Url::parse(host).unwrap(),
        username: "admin".into(),
        password: SecretString::new("pw".into()),
        auth: AuthMethod::Session,
        insecure_skip_verify: false,
        ca_cert_file: None,
    };
    let client = build_http_client(&cfg, std::time::Duration::from_secs(5)).unwrap();
    (make_bmc(&cfg, client), "admin".to_string())
}

/// 401-only 服务器：首轮 401 → 重登失败 → 进入 SessionDegraded；
/// basic 轮同样 401 → basic 连续失败 3 次 → Cooling。
#[tokio::test]
async fn fault_injection_401_only_server_degrades_then_cools() {
    let (host, _server) = spawn_server(axum::http::StatusCode::UNAUTHORIZED);
    let (handle, _) = session_handle(&host);
    let slow_state = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let sessions = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let cfg = StabilityConfig {
        cooldown_failures: 3,
        cooldown_base: Duration::from_secs(60),
        cooldown_max: Duration::from_secs(300),
    };
    // 第 1 轮：Healthy 尝试 session → 401 → 重登失败 → session_recovery_failed
    let o = redfish_exporter::scraper::run_bmc_round(
        &handle, None, &slow_state, &sessions, Duration::from_secs(5), true, false,
    )
    .await;
    assert!(o.report.is_none());
    assert!(o.session_recovery_failed);
    let mut state = BmcState::Healthy { failures: 0 };
    state = on_round_result(&state, &o, true, &cfg, Instant::now());
    assert!(matches!(state, BmcState::SessionDegraded { .. }));
    // 第 2-4 轮：basic 兜底轮（attempted_session=false）→ 401 → basic_failures 累加至 Cooling
    for _ in 0..3 {
        let o = redfish_exporter::scraper::run_bmc_round(
            &handle, None, &slow_state, &sessions, Duration::from_secs(5), false, true,
        )
        .await;
        assert!(o.report.is_none());
        assert!(!o.session_recovery_failed);
        state = on_round_result(&state, &o, true, &cfg, Instant::now());
    }
    assert!(matches!(state, BmcState::Cooling { .. }));
}

/// 会话接口 401 但 basic 放行的服务器：SessionDegraded 下 basic 兜底成功 → up=1 + session-degraded 标记。
#[tokio::test]
async fn fault_injection_session_401_basic_ok_degrades_with_mark() {
    use axum::extract::State as AxumState;
    use std::sync::atomic::{AtomicUsize, Ordering as AtOrd};
    // 用服务根 401（session token 被拒）+ 后续 basic 成功模拟：简化起见，
    // 服务器对带 Authorization: Basic 的请求返回 200 最小服务根 JSON，
    // 对不带（token）请求返回 401。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/redfish/v1", get(|| async {
            axum::Json(serde_json::json!({
                "@odata.id": "/redfish/v1",
                "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            }))
        }))
        .fallback(|| async { (axum::http::StatusCode::UNAUTHORIZED, "no") });
    let _server = tokio::spawn(async move {
        axum_server::Server::from_tcp(listener).unwrap().serve(app.into_make_service()).await.unwrap();
    });
    let (handle, _) = session_handle(&format!("http://{addr}"));
    let slow_state = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let sessions = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    // 第 1 轮：401 → session_recovery_failed（root 401 触发 is_unauthorized 重登流程）
    let o = redfish_exporter::scraper::run_bmc_round(
        &handle, None, &slow_state, &sessions, Duration::from_secs(5), true, false,
    )
    .await;
    assert!(o.report.is_none());
    assert!(o.session_recovery_failed);
    // Degraded basic 轮：root 200（basic 放行），collect_fast 全空成功 → up=1
    let o = redfish_exporter::scraper::run_bmc_round(
        &handle, None, &slow_state, &sessions, Duration::from_secs(5), false, true,
    )
    .await;
    assert!(o.report.is_some());
    let report = o.report.unwrap();
    assert!(report.failed_resources.is_empty());
    let marked = redfish_exporter::scraper::with_degraded_mark(report, "inj");
    assert!(marked.metrics.iter().any(|m| m.name == "redfish_scrape_error"));
}
```

（执行者注意：第 2 个测试中 basic 请求实际会带 `Authorization: Basic` 头——服务器无条件放行 `/redfish/v1` 即模拟 basic 成功；401 路径由 session token 触发。若 root 200 后 collect_fast 因无 Chassis 链接而成功返回空报告，则断言成立。若实际观察与预期不符（例如 establish_session 在首轮先取 root 成功），按真实行为调整断言并记录理由——测试目标是钉住"重登失败→降级→兜底成功→标记"的状态流。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test stability_test fault_injection`
Expected: FAIL（`run_bmc_round` 尚不存在——若 Task 4 已完成则直接通过，本步骤在 Task 4 完成前执行）

- [ ] **Step 3: 实现**（无产品代码改动；测试基于 Task 3/4 的 seam）

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test stability_test`; 全量门禁
Expected: 全绿

- [ ] **Step 5: 提交**

```powershell
git add tests/stability_test.rs
git commit -m "test: fault injection matrix for cooldown and session degradation"
```

---

### Task 6: MockBmc 期望抽取 + soak 测试（短跑验证）

**Files:**
- Create: `tests/common/mod.rs`、`tests/soak_test.rs`
- Modify: `tests/integration_test.rs`（改用 common 模块）、`Cargo.toml`（dev-dep sysinfo）

**Interfaces:**
- Produces: `tests/common/mod.rs` 导出 `expect_service_root`/`expect_chassis_round`/`expect_chassis_collection`/`expect_sensor_payloads`/`expect_thermal_power_payloads`/`expect_systems_collection`/`expect_system`/`expect_processor_payloads`/`expect_memory_payloads`/`expect_storage_payloads`/`expect_bios_payloads`/`expect_ethernet_payloads`/`expect_pcie_payloads`/`expect_assembly_payloads`/`expect_firmware_payloads`（自 integration_test.rs 原样搬移，签名不变）

- [ ] **Step 1: 抽取 common 模块并验证不破坏**

创建 `tests/common/mod.rs`：内容为 integration_test.rs 第 20-431 行的全部 `expect_*` 辅助函数与 `type Mock` 定义（`use` 语句随函数一并搬移）。integration_test.rs 删除这些本地定义，顶部加：

```rust
mod common;
use common::*;
```

`tests/soak_test.rs` 头部同样 `mod common; use common::*;`。

Run: `cargo test --test integration_test`
Expected: 全绿（行为不变）

- [ ] **Step 2: 添加 sysinfo dev-dep 并写 soak 测试**

`Cargo.toml` dev-dependencies：

```toml
sysinfo = "0.30"
```

`tests/soak_test.rs`：

```rust
//! 长稳 soak（#[ignore]，发布前手动运行）：
//!   cargo test --release --test soak_test -- --ignored
//! 时长由 SOAK_SECS 环境变量控制（默认 14400s=4h，验收实跑 ≥7200s）。
//! 健康 MockBmc 长期循环：断言轮耗时稳定、无失败资源、输出大小恒定、RSS 无泄漏式增长。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::Bmc as MockBmc;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::registry::build_registry;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
#[ignore]
async fn soak_healthy_mock_bmc() {
    let secs: u64 = std::env::var("SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14400);
    let interval = Duration::from_secs(1);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut samples: Vec<(f64, f64, f64, usize)> = Vec::new(); // (elapsed_s, round_ms, rss_mb, bytes)
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id() as u32);

    while Instant::now() < deadline {
        let bmc = Arc::new(MockBmc::default());
        expect_service_root(&bmc, &["Chassis", "Systems", "UpdateService"]);
        expect_chassis_round(&bmc, true);
        expect_sensor_payloads(&bmc);
        expect_chassis_round(&bmc, true);
        expect_thermal_power_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Processors"]);
        expect_processor_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Memory"]);
        expect_memory_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &[]);
        expect_chassis_round(&bmc, true);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Storage"]);
        expect_storage_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["EthernetInterfaces"]);
        expect_ethernet_payloads(&bmc);
        expect_chassis_round(&bmc, true);
        expect_pcie_payloads(&bmc);
        expect_firmware_payloads(&bmc);
        expect_chassis_round(&bmc, true);
        expect_assembly_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Bios"]);
        expect_bios_payloads(&bmc);

        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let t0 = Instant::now();
        let fast = collect_fast(Arc::clone(&bmc), &root, "soak").await.unwrap();
        let slow = collect_slow(Arc::clone(&bmc), &root, "soak").await.unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report("soak", merged.metrics, merged.failed_resources, t0);
        assert!(
            report.failed_resources.is_empty(),
            "soak round failed: {:?}",
            report.failed_resources
        );
        let registry = build_registry("soak", &report, 0).await.unwrap();
        let out = redfish_exporter::metrics::encode(&registry);
        let round_ms = t0.elapsed().as_secs_f64() * 1000.0;
        sys.refresh_process(pid);
        let rss_mb = sys
            .process(pid)
            .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
            .unwrap_or(0.0);
        samples.push((deadline.elapsed().as_secs_f64().abs(), round_ms, rss_mb, out.len()));
        tokio::time::sleep(interval).await;
    }

    assert!(samples.len() >= 10, "soak too short: {} rounds", samples.len());
    let mid = samples.len() / 2;
    let first = &samples[..mid];
    let second = &samples[mid..];
    let median = |xs: &[(f64, f64, f64, usize)], idx: usize| -> f64 {
        let mut v: Vec<f64> = xs.iter().map(|s| match idx {
            1 => s.1,
            2 => s.2,
            _ => s.3 as f64,
        }).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let m_round_first = median(first, 1);
    let m_round_second = median(second, 1);
    assert!(
        m_round_second <= m_round_first * 3.0 + 10.0,
        "round time drifted: first median {m_round_first}ms, second {m_round_second}ms"
    );
    let m_rss_first = median(first, 2);
    let m_rss_second = median(second, 2);
    assert!(
        m_rss_second <= m_rss_first * 1.5 + 5.0,
        "RSS drifted: first median {m_rss_first}MB, second {m_rss_second}MB"
    );
    let m_bytes_first = median(first, 3);
    let m_bytes_second = median(second, 3);
    assert_eq!(m_bytes_first as usize, m_bytes_second as usize, "output size drifted");
    println!(
        "soak ok: {} rounds, round_ms med {}->{}, rss_mb med {}->{}, bytes {}",
        samples.len(), m_round_first, m_round_second, m_rss_first, m_rss_second, m_bytes_second
    );
}
```

- [ ] **Step 3: 短跑验证（SOAK_SECS=30）**

Run: `cargo test --release --test soak_test -- --ignored`（环境变量 `SOAK_SECS=30`；PowerShell：`$env:SOAK_SECS="30"`）
Expected: PASS + 输出 `soak ok: ...`

- [ ] **Step 4: 全量门禁与提交**

Run: 全量门禁
Expected: 全绿（soak 测试因 `#[ignore]` 不参与默认运行）

```powershell
git add tests/common/mod.rs tests/integration_test.rs tests/soak_test.rs Cargo.toml Cargo.lock
git commit -m "test: shared mock helpers and ignored long-run soak test"
```

---

### Task 7: CI Linux SIGTERM 冒烟 job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: 追加 job**

`ci.yml` 末尾追加：

```yaml
  graceful-shutdown-smoke:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.90.0
      - name: Build
        run: cargo build
      - name: SIGTERM graceful shutdown smoke
        run: |
          set -euo pipefail
          cat > /tmp/smoke-config.yaml <<'EOF'
          listen_addr: "127.0.0.1:19417"
          scrape_interval: "60s"
          scrape_timeout: "15s"
          request_timeout: "10s"
          bmcs:
            - name: unreachable
              host: http://127.0.0.1:1
              username: u
              password: "p"
          EOF
          ./target/debug/redfish-exporter -c /tmp/smoke-config.yaml > /tmp/smoke.log 2>&1 &
          pid=$!
          sleep 2
          kill -TERM "$pid"
          wait "$pid" && rc=0 || rc=$?
          echo "exporter exit code: $rc"
          test "$rc" -eq 0
          grep -q "shutdown complete" /tmp/smoke.log
```

- [ ] **Step 2: 本地等价验证（Windows 可做的部分）**

Run: `cargo run -- -c <临时 config> &`（PowerShell Start-Process），数秒后 Stop-Process 验证 Windows Ctrl+C 路径不回归（Windows 无法发 SIGTERM，Linux 行为由 CI 验证）
Expected: 进程正常启动并可被停止（无挂起）

- [ ] **Step 3: 提交**

```powershell
git add .github/workflows/ci.yml
git commit -m "ci: graceful shutdown smoke job (Linux SIGTERM)"
```

---

### Task 8: 文档交付 + 安全回归验证

**Files:**
- Modify: `docs/design.md`、`README.md`、`config.example.yaml`
- 无代码改动

- [ ] **Step 1: docs/design.md 新增「自适应调度」节**（插在「Known design trade-off」之前）

```markdown
## Adaptive scheduling (stability)

Per-BMC state machine (`src/stability.rs`, wired in `src/scraper.rs`):

- **Healthy** — normal collection; consecutive failed rounds (round error OR any `failed_resources`, i.e. `up=0` rounds) are counted.
- **Cooling** — after `stability.cooldown_failures` consecutive failures the BMC is fully skipped (no requests); an `up=0` snapshot with `redfish_scrape_error{resource="cooldown"}` is published each round without incrementing `redfish_scrape_errors_total`. Retries follow exponential backoff `min(cooldown_max, cooldown_base × 2^n)`; success resets to Healthy.
- **SessionDegraded** (session-auth BMCs only) — entered when a 401 re-login fails: basic credentials take over collection each round (up=1 achievable, marked with `redfish_scrape_error{resource="session-degraded"}=1`), while session re-establishment retries on the same backoff schedule; success returns to Healthy; consecutive basic failures also lead to Cooling.

Config (frozen semantics for 0.1.0):

```yaml
stability:
  cooldown_failures: 3     # consecutive failure threshold
  cooldown_base: "60s"     # first backoff
  cooldown_max: "300s"     # backoff cap
```

Mutex poisoning is recovered via `crate::recover_lock` (log + continue) instead of cascading panics.
```

- [ ] **Step 2: README Configuration 表新增行**

```markdown
| `stability`                | defaults 3 / 60s / 300s | Failure cooldown: consecutive failed rounds before full cooldown, first backoff, backoff cap. Session-auth BMCs fall back to basic collection while session re-login backs off |
```

- [ ] **Step 3: config.example.yaml 追加节**

```yaml
stability:                    # failure cooldown / backoff (defaults shown)
  cooldown_failures: 3        # consecutive failed rounds before full cooldown
  cooldown_base: "60s"        # first backoff after cooldown starts
  cooldown_max: "300s"        # backoff cap
```

- [ ] **Step 4: 安全回归验证**

Run: `cargo test --test config_test --test http_test --test bmc_test --test pagination_test`; `cargo audit`; `cargo deny check`
Expected: 全绿 / audit 白名单内 / deny 四类 ok

- [ ] **Step 5: 提交**

```powershell
git add docs/design.md README.md config.example.yaml
git commit -m "docs: adaptive scheduling section, config reference and example"
```

---

### Task 9: soak 实跑（≥2h）+ 验收记录

**Files:**
- Create: `docs/audit/2026-08-24-stability.md`
- 无代码改动

- [ ] **Step 1: 启动 ≥2h soak（后台）**

```powershell
$env:SOAK_SECS = "7200"
Start-Process -FilePath "cargo" -ArgumentList "test","--release","--test","soak_test","--","--ignored","--nocapture" -RedirectStandardOutput ".superpowers\soak-7200.log" -RedirectStandardError ".superpowers\soak-7200.err" -NoNewWindow -PassThru | Select-Object -ExpandProperty Id | Set-Content ".superpowers\soak.pid"
```

（工作目录为仓库根；`.superpowers/` 已被 .gitignore 忽略。）

- [ ] **Step 2: 轮询完成（每 10 分钟一次，直至进程退出）**

```powershell
$pid = Get-Content ".superpowers\soak.pid"
while (Get-Process -Id $pid -ErrorAction SilentlyContinue) { Start-Sleep -Seconds 600 }
Get-Content ".superpowers\soak-7200.log" -Tail 20
```

Expected: 日志含 `soak ok: ...` 且测试进程退出码 0（检查 log 无 `assert` 失败字样）。

- [ ] **Step 3: 撰写验收记录 `docs/audit/2026-08-24-stability.md`**

```markdown
# 0.1.0 稳定性专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-stability-hardening-design.md` §10 验收标准
范围：五维专项第 2 项（稳定性）；不破坏第 1 项（安全性）成果

## 结论

**PASS / CONDITIONAL PASS / FAIL**（执行者按证据填写）

## 决策记录

D1 防御+故障注入+本地 soak / D2 指数退避冷却 / D3 冷却期 basic 兜底采集 / D4 CI Linux SIGTERM 冒烟

## 验收标准对照

| 标准（spec §10） | 结果 | 证据 |
|---|---|---|
| 1. 故障注入矩阵全绿 + 全部门禁 | | 测试输出摘录（stability_test N 个、全量测试计数、clippy/fmt） |
| 2. soak ≥2h | | soak log 摘录（轮数、轮耗时中位数前后对比、RSS 中位数前后对比、输出字节恒定） |
| 3. CI Linux SIGTERM job | | CI 运行结果（本地不可达则注明"待推送后 CI 验证"或本地等价验证记录） |
| 4. 真机回归 | | 执行结果或未执行原因 |
| 5. 安全回归 | | 四测试文件全绿 + audit/deny 摘录 |
| 6. 文档与语义冻结 | | design.md/README/config.example 交付 + 冻结清单核对 |

## 遗留跟踪

- 真机 24-72h 长稳（投产前灰度观察，D1 既定）
- 熔断健康探测（HEAD 轻量探测）backlog
- 会话失效预检（expiration_time 提前重登）backlog
```

- [ ] **Step 4: 提交**

```powershell
git add docs/audit/2026-08-24-stability.md
git commit -m "docs: 0.1.0 stability hardening acceptance record"
```

---

## Self-Review 记录

- **Spec 覆盖**：§3 韧性（Task 1）、§4 调度（Task 3/4）、§4.2 配置（Task 2）、§5.1 故障注入（Task 5）、§5.2 soak（Task 6/9）、§5.3 CI（Task 7）、§5.4 真机（Task 9 记录）、§6 门禁（Task 8 Step 4）、§7 文档（Task 8）、§10 验收（Task 9）。
- **占位符**：无 TBD/TODO；验收记录模板中的「执行者按证据填写」为执行期证据捕获点，非占位。
- **类型一致性**：`BmcState`/`RoundAction`/`RoundOutcome` 定义（Task 3）与 Task 4/5 使用一致；`run_bmc_round(handle, slow_interval, slow_state, sessions, timeout, attempt_session, use_basic_only)` 签名在 Task 4 定义、Task 5 使用一致；`on_round_result(state, outcome, failed, cfg, now)` 参数顺序 Task 3 定义与 Task 4/5 使用一致；common 模块函数名与 integration_test 原定义一致。
- **计划内裁决**：轮失败定义 = report 为 None 或 failed_resources 非空（含 up=0 与快/慢组超时）——spec §4.3「Healthy 失败 up=0 计数」的直接推论，在 Task 3 Interfaces 与 Task 4 代码中显式落实。
