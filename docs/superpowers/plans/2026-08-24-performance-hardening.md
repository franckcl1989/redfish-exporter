# 性能专项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 0.1.0 性能专项——预编码快照缓存（/metrics 热路径 310ms→~ms 级）+ 编码/注册微观优化 + mock 性能基准与真机对比。

**Architecture:** 快照层存储 `Arc<RegistryEntry { registry, encoded: Vec<u8> }>`（`Snapshot::update` 内部编码一次，签名不变，scraper 三处发布路径零改动）；`metrics.rs` 新增 `encode_bytes`/`encode_bytes_into`（可复用缓冲）；`http.rs` metrics_handler 直接拼接预编码字节为响应体；`register_into` label 查找 O(L²)→O(L) + 预分配。不触碰 N×walk 与 per-BMC 并发。

**Tech Stack:** Rust 1.90 / edition 2024、prometheus crate（TextEncoder）、tokio/axum、既有 sysinfo（dev-dep）。零新依赖。

**Spec:** `docs/superpowers/specs/2026-08-24-performance-hardening-design.md`（计划依 spec 论证，执行者须先读 spec）

## Global Constraints

- rust-version 1.90、edition 2024；`#![forbid(unsafe_code)]` 在 src/ 必须保持
- 每任务结束必须 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets` 全绿后才提交
- 代码注释用中文（仓库惯例）；提交信息用英文、仓库风格（`feat:`/`test:`/`docs:`）
- **零新依赖**（运行时与 dev 均不新增；sysinfo 已有）
- 安全回归（config/http/bmc/pagination 四套件）+ 稳定回归（stability/scraper/integration 套件）必须全绿；cargo audit（白名单内零新增）/ deny 全绿
- **语义冻结（spec §6.3）**：/metrics 输出字节与旧实现逐字节一致（等价性测试守护）；快照内部表示为内部实现
- 不触碰：N×walk 采集结构、per-BMC 并发、稳定性状态机、web 配置语义

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `src/registry.rs` | 改 | `RegistryEntry`、Snapshot 内值类型、update 内部编码 + scratch 缓冲复用 |
| `src/metrics.rs` | 改 | `encode_bytes`/`encode_bytes_into`；`register_into` O(L) 查找 + 预分配 |
| `src/http.rs` | 改 | metrics_handler 拼接预编码字节 |
| `tests/http_test.rs` | 改 | 等价性测试（快照编码字节 vs 现场编码；handler 输出 vs 旧路径） |
| `tests/scraper_test.rs` | 改 | registries() 返回类型适配（如有需要） |
| `tests/perf_test.rs` | 新建 | `#[ignore]` mock 性能基准 |
| `docs/design.md` / `docs/audit/2026-08-24-performance.md` | 改/新建 | 文档与验收记录 |

---

### Task 1: 预编码快照缓存（registry + metrics + http 出口 + 等价性测试）

**Files:**
- Modify: `src/registry.rs`、`src/metrics.rs`、`src/http.rs:88-96`
- Test: `tests/http_test.rs`（追加）、`tests/scraper_test.rs`（如需适配）

**Interfaces:**
- Produces:
  - `pub struct RegistryEntry { pub registry: Arc<prometheus::Registry>, pub encoded: Vec<u8> }`（registry.rs）
  - `Snapshot::update(name, Arc<Registry>)` 签名不变（内部编码）；`Snapshot::registries() -> Vec<(String, Arc<RegistryEntry>)>`（返回类型变化）；`Snapshot::registry(name) -> Option<Arc<Registry>>` 不变
  - `pub fn encode_bytes(registry: &prometheus::Registry) -> Vec<u8>`；`pub fn encode_bytes_into(registry: &prometheus::Registry, out: &mut Vec<u8>)`（metrics.rs）；`encode()` 保留（String 版，供测试）
- Consumes: 无（Task 2 依赖 encode_bytes_into）

- [ ] **Step 1: 写失败测试**

`tests/http_test.rs` 追加（use 区已有 `redfish_exporter::metrics::Metric` 与 `build_registry` 相关导入，需补 `redfish_exporter::metrics::encode`）：

```rust
/// 语义冻结守护：快照存储的预编码字节与现场编码逐字节一致。
#[tokio::test]
async fn snapshot_encoded_bytes_equal_live_encode() {
    let snap = Arc::new(Snapshot::new());
    let reg = redfish_exporter::registry::build_registry(
        "bmc1",
        &redfish_exporter::collector::ScrapeReport {
            metrics: vec![
                Metric::gauge("redfish_up", "up")
                    .label("bmc", "bmc1".into())
                    .build(1.0),
                Metric::gauge("redfish_scrape_error", "err")
                    .label("bmc", "bmc1".into())
                    .label("resource", "cooldown".into())
                    .build(1.0),
            ],
            failed_resources: vec![],
        },
        7,
    )
    .await
    .unwrap();
    let live = redfish_exporter::metrics::encode(&reg).into_bytes();
    snap.update("bmc1", reg.clone());
    let entries = snap.registries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.encoded, live);
    assert!(Arc::ptr_eq(&entries[0].1.registry, &reg));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test http_test snapshot_encoded_bytes_equal_live_encode`
Expected: FAIL（`RegistryEntry` 不存在 / registries() 仍返回 `Arc<Registry>`）

- [ ] **Step 3: 实现 metrics.rs**

`src/metrics.rs` 的 `encode` 之后追加（`encode` 函数体改为委托）：

```rust
/// 编码 registry 为 Prometheus 文本格式字节（快照预编码缓存使用）。
pub fn encode_bytes(registry: &prometheus::Registry) -> Vec<u8> {
    let mut buf = Vec::new();
    encode_bytes_into(registry, &mut buf);
    buf
}

/// 编码写入调用方缓冲（可复用：clear 后重复调用避免反复增长分配）。
pub fn encode_bytes_into(registry: &prometheus::Registry, out: &mut Vec<u8>) {
    use prometheus::TextEncoder;
    // 不变量：encode 写入 Vec<u8> 不会失败；失败即程序缺陷，panic 合理。
    TextEncoder::new()
        .encode(&registry.gather(), out)
        .expect("TextEncoder::encode writes to Vec<u8> and cannot fail");
}

pub fn encode(registry: &prometheus::Registry) -> String {
    String::from_utf8(encode_bytes(registry)).expect("TextEncoder output is always valid UTF-8")
}
```

（原 `encode` 函数体删除，替换为上述三函数。）

- [ ] **Step 4: 实现 registry.rs**

```rust
/// 快照条目：registry 与发布时编码一次的文本（/metrics 直接拼接，零现场编码）。
pub struct RegistryEntry {
    pub registry: Arc<prometheus::Registry>,
    pub encoded: Vec<u8>,
}
```

`Snapshot` 结构体改为：

```rust
pub struct Snapshot {
    inner: RwLock<HashMap<String, Arc<RegistryEntry>>>,
    errors: Mutex<HashMap<String, u64>>,
    /// 编码复用缓冲：跨轮增长一次后不再反复扩容（每轮 encode 写入后克隆入条目）。
    scratch: Mutex<Vec<u8>>,
}
```

`Snapshot::new` 增加 `scratch: Mutex::new(Vec::new())`；顶部 `use crate::metrics::{..., encode_bytes_into}`。

`update` 改写（其余方法照旧，`registry` 返回 `entry.registry.clone()`，`registries` 返回 `(String, Arc<RegistryEntry>)` 对）：

```rust
    /// 插入或替换指定 BMC 的快照（原子，按 BMC 隔离）。发布时编码一次存入条目，
    /// /metrics 直接拼接预编码字节（编码成本移出请求路径）。
    pub fn update(&self, bmc_name: &str, registry: Arc<prometheus::Registry>) {
        let mut scratch = recover_lock(self.scratch.lock());
        scratch.clear();
        encode_bytes_into(&registry, &mut scratch);
        let entry = Arc::new(RegistryEntry {
            registry,
            encoded: scratch.clone(),
        });
        recover_lock(self.inner.write()).insert(bmc_name.to_string(), entry);
    }
```

- [ ] **Step 5: 实现 http.rs metrics_handler**

```rust
async fn metrics_handler(State(state): State<AppState>) -> Response {
    if state.snapshot.is_empty() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-redfish-exporter"),
            HeaderValue::from_static("no-data-yet"),
        );
        return (StatusCode::OK, headers, "").into_response();
    }
    // 预编码字节直接拼接（发布时已编码）；registries() 按 BMC 名排序，输出确定性不变。
    let entries = state.snapshot.registries();
    let total: usize = entries.iter().map(|(_, e)| e.encoded.len()).sum();
    let mut body: Vec<u8> = Vec::with_capacity(total);
    for (_, entry) in &entries {
        body.extend_from_slice(&entry.encoded);
    }
    (StatusCode::OK, [(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], body).into_response()
}
```

（`use crate::metrics::encode;` 删除；`String` 导入如无他用则移除。）

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test --test http_test`; `cargo test --test scraper_test`; `cargo test --test integration_test`; `cargo test --test soak_test`（短跑不需，编译即可——`cargo test --test soak_test --no-run`）
Expected: 全绿（scraper_test 的 `registries().len()` 与新返回类型兼容；soak/integration 用 `encode(&registry)` 不受影响）

- [ ] **Step 7: 全量门禁与提交**

Run: 全量门禁（fmt/clippy/test/audit/deny + 安全四套件 + 稳定三套件）
Expected: 全绿

```powershell
git add src/registry.rs src/metrics.rs src/http.rs tests/http_test.rs tests/scraper_test.rs
git commit -m "feat: pre-encoded snapshot cache for /metrics hot path"
```

---

### Task 2: 编码与注册微观优化（register_into + 预分配）

**Files:**
- Modify: `src/metrics.rs:141-184`（register_into）
- Test: `tests/metrics_test.rs`（追加）

**Interfaces:**
- Produces: `register_into` 行为不变（签名不变），内部 O(L) 取值 + 预分配

- [ ] **Step 1: 写失败测试**（等价守护——优化后行为必须与旧实现一致）

`tests/metrics_test.rs` 追加（该文件已有 `encode`/`register_into` 相关导入；执行者先读该文件对齐 import）：

```rust
#[test]
fn register_into_label_lookup_equivalent_for_mixed_label_sets() {
    // 同一指标名下不同 label 集合（模拟多资源序列）：缺失 label 以空值兜底，注册不冲突。
    let registry = prometheus::Registry::new();
    let m1 = Metric::gauge("redfish_health_status", "h")
        .label("bmc", "b1".into())
        .label("resource_type", "system".into())
        .build(1.0);
    let m2 = Metric::gauge("redfish_health_status", "h")
        .label("bmc", "b2".into())
        .build(1.0);
    redfish_exporter::metrics::register_into(&[m1, m2], &registry).unwrap();
    let out = encode(&registry);
    assert!(out.contains("redfish_health_status{bmc=\"b1\",resource_type=\"system\"} 1"), "{out}");
    assert!(out.contains("redfish_health_status{bmc=\"b2\",resource_type=\"\"} 1"), "{out}");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --test metrics_test register_into_label_lookup_equivalent`
Expected: 视现实现而定——若现实现已通过（O(L²) 查找行为等价），本测试为守护型测试直接 PASS（记录为等价守护，非 RED）。此时跳过 Step 3 的"失败预期"，直接进入实现并确保测试仍绿。

- [ ] **Step 3: 实现 register_into 优化**

`src/metrics.rs` 的 `register_into` 函数体替换（保持行为等价）：

```rust
pub fn register_into(
    metrics: &[Metric],
    registry: &prometheus::Registry,
) -> Result<(), MetricsError> {
    use std::collections::HashMap;

    let mut vecs: HashMap<(&'static str, &'static str), prometheus::GaugeVec> =
        HashMap::with_capacity(metrics.len());
    for m in metrics {
        let mut names: Vec<&'static str> = m.labels.iter().map(|(k, _)| *k).collect();
        names.sort();
        names.dedup();
        let gv = vecs.entry((m.name, m.help)).or_insert_with(|| {
            // 不变量：GaugeVec 以 'static 字面量名构造，不可能失败；失败即程序缺陷，panic 合理。
            prometheus::GaugeVec::new(prometheus::Opts::new(m.name, m.help), &names)
                .expect("GaugeVec construction with &'static str names cannot fail")
        });
        // 微观优化：每 metric 先建 label 映射，取值 O(L)（原实现每 label 线性查找 O(L²)）。
        let lookup: HashMap<&'static str, &str> =
            m.labels.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let label_values: Vec<String> = names
            .iter()
            .map(|n| lookup.get(n).copied().unwrap_or_default().to_string())
            .collect();
        debug_assert_eq!(
            label_values.len(),
            names.len(),
            "label cardinality mismatch for '{}'",
            m.name
        );
        gv.with_label_values(&label_values).set(m.value);
    }
    for (_, gv) in vecs {
        match registry.register(Box::new(gv)) {
            Ok(()) => {}
            // 同一 registry 中同名指标重复注册视为合并（collect 阶段已分组），忽略
            Err(prometheus::Error::AlreadyReg) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --test metrics_test`; 全量门禁
Expected: 全绿

- [ ] **Step 5: 提交**

```powershell
git add src/metrics.rs tests/metrics_test.rs
git commit -m "feat: O(labels) lookup and preallocation in register_into"
```

---

### Task 3: mock 性能基准（tests/perf_test.rs，#[ignore]）

**Files:**
- Create: `tests/perf_test.rs`
- Modify: `tests/common/mod.rs`（如需要导出增量辅助）

**Interfaces:**
- Consumes: Task 1 的 `encode_bytes`/`encode`、`RegistryEntry`；tests/common 的期望集
- Produces: `#[ignore]` 基准（本地执行：`cargo test --release --test perf_test -- --ignored --nocapture`；`PERF_ASSERT=1` 开启软阈值断言）

- [ ] **Step 1: 写基准测试**

```rust
//! 性能基准（#[ignore]，本地跑）：确定性 mock 环境的端到端轮耗时、编码耗时、
//! 预编码 /metrics 热路径耗时与 RSS 采样。
//!   cargo test --release --test perf_test -- --ignored --nocapture
//!   $env:PERF_ASSERT="1" 时启用软阈值断言（本地验证用，不进 CI）。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::Bmc as MockBmc;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::metrics::{encode_bytes};
use redfish_exporter::registry::{Snapshot, build_registry};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn expect_full_round(bmc: &MockBmc) {
    expect_service_root(bmc, &["Chassis", "Systems", "UpdateService"]);
    expect_chassis_round(bmc, true);
    expect_sensor_payloads(bmc);
    expect_chassis_round(bmc, true);
    expect_thermal_power_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Processors"]);
    expect_processor_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Memory"]);
    expect_memory_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &[]);
    expect_chassis_round(bmc, true);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Storage"]);
    expect_storage_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["EthernetInterfaces"]);
    expect_ethernet_payloads(bmc);
    expect_chassis_round(bmc, true);
    expect_pcie_payloads(bmc);
    expect_firmware_payloads(bmc);
    expect_chassis_round(bmc, true);
    expect_assembly_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Bios"]);
    expect_bios_payloads(bmc);
}

#[tokio::test]
#[ignore]
async fn perf_benchmark_mock_pipeline() {
    const ROUNDS: usize = 20;
    let mut round_ms = Vec::with_capacity(ROUNDS);
    let mut encode_ms = Vec::with_capacity(ROUNDS);
    let mut hot_ms = Vec::with_capacity(ROUNDS);
    let mut bytes_len = 0usize;
    let snap = Arc::new(Snapshot::new());

    for _ in 0..ROUNDS {
        let bmc = Arc::new(MockBmc::default());
        expect_full_round(&bmc);
        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let t0 = Instant::now();
        let fast = collect_fast(Arc::clone(&bmc), &root, "perf").await.unwrap();
        let slow = collect_slow(Arc::clone(&bmc), &root, "perf").await.unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report("perf", merged.metrics, merged.failed_resources, t0);
        assert!(report.failed_resources.is_empty());
        let registry = build_registry("perf", &report, 0).await.unwrap();
        round_ms.push(t0.elapsed().as_secs_f64() * 1000.0);

        let te = Instant::now();
        let encoded = encode_bytes(&registry);
        encode_ms.push(te.elapsed().as_secs_f64() * 1000.0);
        bytes_len = encoded.len();

        // 预编码 /metrics 热路径：拼接各 BMC 条目字节（单 BMC）
        snap.update("perf", registry);
        let th = Instant::now();
        let entries = snap.registries();
        let total: usize = entries.iter().map(|(_, e)| e.encoded.len()).sum();
        let mut body: Vec<u8> = Vec::with_capacity(total);
        for (_, e) in &entries {
            body.extend_from_slice(&e.encoded);
        }
        hot_ms.push(th.elapsed().as_secs_f64() * 1000.0);
    }

    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id() as u32);
    sys.refresh_process(pid);
    let rss_mb = sys
        .process(pid)
        .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
        .unwrap_or(0.0);

    let (rm, em, hm) = (median(&round_ms), median(&encode_ms), median(&hot_ms));
    println!(
        "perf: rounds={ROUNDS} round_ms_med={rm:.2} encode_ms_med={em:.2} hot_path_ms_med={hm:.4} bytes={bytes_len} rss_mb={rss_mb:.1}"
    );
    if std::env::var("PERF_ASSERT").is_ok() {
        assert!(hm < 10.0, "hot path median {hm}ms >= 10ms");
        assert!(em < 500.0, "encode median {em}ms >= 500ms");
        assert!(rm < 2000.0, "round median {rm}ms >= 2000ms");
    }
}
```

（`sysinfo` 已在 dev-deps；`Snapshot`/`build_registry`/`encode_bytes` 均为 pub。若 `PERF_ASSERT` 环境变量比较在 Windows PowerShell 下需 `$env:PERF_ASSERT="1"`，测试内 `std::env::var("PERF_ASSERT").is_ok()` 与值无关，任一值即开启。）

- [ ] **Step 2: 运行基准（release，无断言）**

Run: `cargo test --release --test perf_test -- --ignored --nocapture`
Expected: 输出 `perf: rounds=20 ...` 一行；hot_path 应为亚毫秒级

- [ ] **Step 3: 运行基准（PERF_ASSERT=1）**

Run: `$env:PERF_ASSERT="1"; cargo test --release --test perf_test -- --ignored --nocapture`
Expected: PASS（阈值满足）；记录输出进报告

- [ ] **Step 4: 全量门禁与提交**

Run: 全量门禁（perf_test 因 #[ignore] 不进默认运行）
Expected: 全绿

```powershell
git add tests/perf_test.rs
git commit -m "test: ignored mock-based performance benchmark"
```

---

### Task 4: 文档 + 真机对比测量 + 验收记录 + 全量验证

**Files:**
- Modify: `docs/design.md`
- Create: `docs/audit/2026-08-24-performance.md`
- 无代码改动

- [ ] **Step 1: docs/design.md 快照节更新**（Data flow and error model 节的「Snapshot consistency」条目后追加）

```markdown
- **Pre-encoded snapshot cache**: each published registry is encoded to Prometheus text bytes once at publish time (`Snapshot::update` stores `RegistryEntry { registry, encoded }` with a reused scratch buffer); `/metrics` concatenates the stored bytes with zero live encoding (hot path ~ms vs ~310ms live encoding at ~10k series). Cost: one extra encoded copy per BMC in memory (~1.1MB for the Dell snapshot). Output is byte-identical to live encoding (pinned by the equivalence test in `tests/http_test.rs`).
```

- [ ] **Step 2: 真机对比测量（有条件）**

两真机可达（<dell-bmc-host> Dell / <inspur-bmc-host> 浪潮；stability 专项已确认 TCP 可达）：
- 用既有验收配置（session、scrape_interval=30s、slow_interval=120s；新默认绑定需显式 `listen_addr: 0.0.0.0:9417` 或本机访问）启动优化后二进制
- 采集 ≥2 轮后测量：`/metrics` 热延迟（连续 10 次请求取 p50，Invoke-WebRequest 计时）、全量轮耗时（日志 `scrape round complete` duration_ms）
- 「before」基线引用验收报告 §3（热延迟 ~310ms、Dell 快组 ~3s/全量 ~11s、浪潮快组 44-48s）
- 真机不可达或凭据不可用则如实记录原因（延续惯例）

- [ ] **Step 3: 撰写验收记录 `docs/audit/2026-08-24-performance.md`**

```markdown
# 0.1.0 性能专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-performance-hardening-design.md` §10 验收标准
范围：五维专项第 3 项（性能）；不破坏第 1/2 项（安全/稳定）成果

## 结论

**PASS / CONDITIONAL PASS / FAIL**（执行者按证据填写）

## 决策记录

D1 出口与微观优化（不碰 N×walk 与 per-BMC 并发）/ D2 mock 基准 + 真机前后对比

## 验收标准对照

| 标准（spec §10） | 结果 | 证据 |
|---|---|---|
| 1. 等价性测试 + 全部门禁 | | 测试输出摘录 |
| 2. mock 基准证据 | | perf_test 输出（round_ms/encode_ms/hot_path_ms/bytes/rss） |
| 3. 真机前后对比 | | 测量结果或未执行原因（before=验收报告 §3 基线） |
| 4. 安全+稳定双回归 | | 套件与 audit/deny 摘录 |
| 5. 文档与语义冻结 | | design.md 更新 + 字节等价守护核对 |

## 性能证据

| 指标 | before（验收报告 §3） | after | 变化 |
|---|---|---|---|
| /metrics 热延迟 | ~310ms（9745 序列） | | |
| Dell 快组/全量轮 | ~3s / ~11s | | |
| 浪潮快组轮 | 44-48s | | |
| mock 全量轮/编码 | — | | 新基准 |

## 遗留跟踪

- N×walk 共享预取与 per-BMC 并发（D1 排除，backlog）
- gzip/流式输出（backlog）
```

- [ ] **Step 4: 全量验证**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`; `cargo audit`; `cargo deny check`; 安全四套件点名; 稳定三套件点名
Expected: 全绿

- [ ] **Step 5: 提交**

```powershell
git add docs/design.md docs/audit/2026-08-24-performance.md
git commit -m "docs: 0.1.0 performance hardening acceptance record"
```

---

## Self-Review 记录

- **Spec 覆盖**：§3 预编码缓存（Task 1）、§4 微观优化（Task 2）、§5.1 等价性（Task 1）、§5.2 mock 基准（Task 3）、§5.3 真机对比（Task 4）、§6 门禁（Task 4）、§7 文档（Task 4）、§10 验收（Task 4）。
- **占位符**：无 TBD/TODO；验收记录模板的「执行者按证据填写」为执行期证据捕获点。
- **类型一致性**：`RegistryEntry`/`registries()` 新返回类型在 Task 1 定义，Task 3（`e.encoded`）使用一致；`encode_bytes`/`encode_bytes_into` 定义与调用一致；`Snapshot::update` 签名不变与 scraper 三处调用点一致；tests/common 的 expect_* 函数名与 perf_test/soak_test 使用一致。
- **兼容性核查**：`encode()`（String 版）保留——soak_test/integration_test/metrics_test/scraper_test 的既有调用不受影响；`registry()` 返回类型不变——scraper_test 的 Arc::ptr_eq 断言不受影响；`registries()` 仅 http.rs 与 scraper_test `.len()` 消费，已适配。
