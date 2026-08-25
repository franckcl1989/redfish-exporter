# 资源专项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 0.1.0 资源专项——实测建档「资源利用率超越对标」证据链（四轴：内存/CPU/连接/请求量）+ 内存模型与上限文档化 + 决策门微优化留痕；不替换 prometheus 存储、不手写编码器。

**Architecture:** 压缩形态专项，默认零生产代码改动：mock 资源基准（RSS-vs-BMC 数曲线，验证每 BMC 内存系数与预编码缓存增量）+ 真机四轴实测（PowerShell 系统工具采样，制品落盘）+ 对标对比与内存模型文档 + 验收记录。实测热点走决策门：先查社区方案 → 用户批准才改，否则记 backlog。

**Tech Stack:** Rust 1.90 / edition 2024、sysinfo（既有 dev-dep）、PowerShell 系统工具（Get-Process/Get-NetTCPConnection）、既有 mock 测试基建。零新依赖。

**Spec:** `docs/superpowers/specs/2026-08-24-resource-hardening-design.md`（计划依 spec 论证，执行者须先读 spec）

## Global Constraints

- rust-version 1.90、edition 2024；`#![forbid(unsafe_code)]` 在 src/ 必须保持
- **生产代码零改动（默认）**：本计划 4 个任务均不改 src/；若实测暴露热点，走决策门（spec D2）——先查社区 crate/现成方案，用户批准后才允许改代码（批准后追加任务，超出本计划）
- **零新依赖**（运行时与 dev 均不新增；sysinfo 已有）
- 每任务结束 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets` 全绿后才提交
- 代码注释用中文（仓库惯例）；提交信息用英文、仓库风格（`test:`/`docs:`）
- **语义冻结**：/metrics 输出字节不变（本维不改编码/存储路径，等价性测试守护为确认性验证）
- **诚实记录**：验收记录中每个数字必须有落盘制品引用（文件路径）；真机不可达/凭据不可用须如实记录原因
- 制品落盘路径：`%TEMP%\opencode\redfish-resource-run\`（沿用性能维 `%TEMP%\opencode\redfish-perf-run\` 惯例）
- 全维回归门禁（spec §6）：安全四套件（config/http/bmc/pagination）+ 稳定套件（stability/scraper/integration/soak 基建）+ 性能基准（perf_test ±5% 容差）+ cargo audit（白名单内零新增）/ deny 全绿

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `tests/resource_test.rs` | 新建 | `#[ignore]` mock 资源基准：RSS-vs-BMC 数曲线、每 BMC 内存系数、每 BMC 编码字节稳定性 |
| `docs/design.md` | 改 | 新增「Resource model (resources)」节（内存构成/线性增长/上限/CPU/连接/请求量结论） |
| `docs/audit/2026-08-24-resource.md` | 新建 | 验收记录（四轴证据 + 对标表 + 决策门留痕 + S1-S5 对照） |
| `%TEMP%\opencode\redfish-resource-run\` | 临时制品（不入库） | 真机采样原始输出、汇总文件 |

---

### Task 1: mock 资源基准（tests/resource_test.rs，#[ignore]）

**Files:**
- Create: `tests/resource_test.rs`

**Interfaces:**
- Consumes: `tests/common` 期望集（expect_service_root/expect_chassis_round/expect_sensor_payloads/expect_thermal_power_payloads/expect_systems_collection/expect_system/expect_processor_payloads/expect_memory_payloads/expect_storage_payloads/expect_ethernet_payloads/expect_pcie_payloads/expect_firmware_payloads/expect_assembly_payloads/expect_bios_payloads）；`redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports}`；`redfish_exporter::registry::{Snapshot, build_registry}`；sysinfo（dev-dep）
- Produces: mock 证据数据——RSS 随 BMC 数（1/2/3/4）增长值、每 BMC RSS 增量系数、每 BMC 预编码字节长度；供 Task 3/4 引用

- [ ] **Step 1: 写基准测试**

`tests/resource_test.rs`（`expect_full_round` 函数体从 `tests/perf_test.rs:33-65` 逐字复制；`median` 不需）：

```rust
//! 资源基准（#[ignore]，本地跑）：mock 环境 RSS-vs-BMC 数曲线与每 BMC 内存系数，
//! 验证性能维移交的预编码缓存增量（真机 ~1.1MB/BMC 在 mock 规模下对应编码字节稳定性）。
//!   cargo test --release --test resource_test -- --ignored --nocapture
//!   $env:RES_ASSERT="1" 时启用软阈值断言（本地验证用，不进 CI）。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::Bmc as MockBmc;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::registry::{Snapshot, build_registry};
use std::sync::Arc;
use std::time::Instant;

fn expect_full_round(bmc: &MockBmc) {
    // （从 tests/perf_test.rs 的 expect_full_round 函数体逐字复制，含全部 expect_* 调用）
}

fn rss_mb() -> f64 {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id() as u32);
    sys.refresh_process(pid);
    sys.process(pid)
        .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
        .unwrap_or(0.0)
}

/// RSS-vs-BMC 数曲线：1..=4 个 BMC 逐个入快照，记录每步 RSS 与预编码字节长度。
#[tokio::test]
#[ignore]
async fn resource_memory_per_bmc_curve() {
    let snap = Arc::new(Snapshot::new());
    let mut rss = Vec::with_capacity(4);
    let mut encoded = Vec::with_capacity(4);
    for n in 1..=4u32 {
        let bmc = Arc::new(MockBmc::default());
        expect_full_round(&bmc);
        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let fast = collect_fast(Arc::clone(&bmc), &root, "res").await.unwrap();
        let slow = collect_slow(Arc::clone(&bmc), &root, "res").await.unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report("res", merged.metrics, merged.failed_resources, Instant::now());
        assert!(report.failed_resources.is_empty(), "mock 全量采集不应有失败资源");
        let registry = build_registry("res", &report, 0).await.unwrap();
        snap.update(&format!("bmc{n}"), registry);
        let entries = snap.registries();
        let entry = entries
            .iter()
            .find(|(name, _)| name == &format!("bmc{n}"))
            .expect("刚更新的 BMC 必须在快照中");
        encoded.push(entry.1.encoded.len());
        rss.push(rss_mb());
    }
    let coef = (rss[3] - rss[0]) / 3.0;
    println!(
        "resource: rss_1_2_3_4_mb={} encoded_bytes_per_bmc={}",
        rss.iter()
            .map(|v| format!("{v:.2}"))
            .collect::<Vec<_>>()
            .join(","),
        encoded
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    println!("resource: per_bmc_rss_delta_mb={coef:.2}");
    // 同一 mock 规模的预编码字节必须逐 BMC 稳定（预编码缓存确定性）。
    assert!(
        encoded.windows(2).all(|w| w[0] == w[1]),
        "per-BMC encoded bytes must be stable: {encoded:?}"
    );
    assert!(rss.windows(2).all(|w| w[1] >= w[0]), "RSS 应随 BMC 数单调不减: {rss:?}");
    if std::env::var("RES_ASSERT").is_ok() {
        // 软阈值：mock 规模每 BMC 增量 < 25MB（registry 结构 + 编码缓存；真机全量快照才 ~44MB WS）。
        assert!(coef < 25.0, "per-BMC RSS delta {coef}MB >= 25MB");
    }
}
```

- [ ] **Step 2: 运行基准（release，无断言）**

Run: `cargo test --release --test resource_test -- --ignored --nocapture`
Expected: 输出 `resource: rss_1_2_3_4_mb=... encoded_bytes_per_bmc=...` 与 `resource: per_bmc_rss_delta_mb=...` 两行；无断言失败

- [ ] **Step 3: 运行基准（RES_ASSERT=1）**

Run: `$env:RES_ASSERT="1"; cargo test --release --test resource_test -- --ignored --nocapture`
Expected: PASS（系数软阈值满足）；记录输出进报告

- [ ] **Step 4: 全量门禁与提交**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`（resource_test 因 #[ignore] 不进默认运行）
Expected: 全绿

```powershell
git add tests/resource_test.rs
git commit -m "test: ignored mock-based resource benchmark (RSS per-BMC curve)"
```

---

### Task 2: 真机四轴实测（内存/CPU/连接/请求量）

**Files:**
- 制品（不入库）：`%TEMP%\opencode\redfish-resource-run\`（采样输出 + `resource-summary.txt`）
- 测量脚本（临时）：`%TEMP%\opencode\redfish-resource-run\measure-mem.ps1`、`measure-cpu.ps1`、`measure-conn.ps1`（执行期现场编写，不入库——性能维先例）

**Interfaces:**
- Consumes: spec §3 四轴定义；既有验收配置（两台真机 Dell <dell-bmc-host> / 浪潮 <inspur-bmc-host>，scrape_interval=30s、slow_interval=120s，性能维真机测量同款）；性能维真机基线（验收记录 `docs/audit/2026-08-24-performance.md`：hot path curl p50 2.4/10.7ms、轮耗时 155.7s 等）
- Produces: 四轴实测数值（内存 WS/私有、CPU 三态、连接复用判定、每轮请求量）+ 落盘制品；供 Task 3/4 引用

- [ ] **Step 1: 启动 exporter 并确认采集稳定**

- 用既有验收配置启动当前 master 二进制（首次先 `cargo build --release`；config 需显式 `listen_addr` 便于本机抓取；会话模式沿用验收配置）
- 日志重定向 `%TEMP%\opencode\redfish-resource-run\resource.out.log`
- 等待 ≥2 个全量轮完成（日志出现 `scrape round complete`），确认两 BMC up=1

- [ ] **Step 2: 内存轴采样**

- idle 窗口（两轮之间）与轮中窗口各采样 ≥5 次，取中位数：
  ```powershell
  Get-Process redfish-exporter | Select-Object Id, @{n='WS_MB';e={[math]::Round($_.WorkingSet64/1MB,1)}}, @{n='Private_MB';e={[math]::Round($_.PrivateMemorySize64/1MB,1)}}
  ```
- 与验收报告 §3 基线（44.3MB WS / 31.5MB 私有，两 BMC 全量快照）对比 → 预编码缓存增量复核（预期 ≈ +2.2MB，两台 × ~1.1MB）
- 输出追加到 `resource-summary.txt`（每次采样带时间戳与窗口标签）

- [ ] **Step 3: CPU 三态采样**

- 三态各测 ≥2 个 10s 窗口（轮中窗口与轮日志时间对齐）：
  ```powershell
  $p = Get-Process redfish-exporter; $c1 = $p.TotalProcessorTime.TotalSeconds; Start-Sleep 10; $p.Refresh(); $c2 = $p.TotalProcessorTime.TotalSeconds; [math]::Round(($c2-$c1)/10*100,1)
  ```
- 三态：idle / 采集轮进行中 / 连续 /metrics 抓取（Invoke-WebRequest 循环，10s 内计数）
- 输出：`idle_cpu_pct / round_cpu_pct / serve_cpu_pct` 写入 `resource-summary.txt`

- [ ] **Step 4: 连接轴采样**

- 轮次时间窗内按 BMC IP 过滤，每 5s 采样一次 ≥6 次：
  ```powershell
  Get-NetTCPConnection -RemoteAddress <dell-bmc-host>,<inspur-bmc-host> -ErrorAction SilentlyContinue | Group-Object RemoteAddress,State | Select-Object Name,Count
  ```
- 判定：采样间 Established 五元组（RemoteAddress+RemotePort）持续复用 → keepalive/池化；每轮新建（SynSent→Established 反复出现、RemotePort 变化）→ 未复用
- 结论写入 `resource-summary.txt`（判定 + 置信度标注，spec §9）

- [ ] **Step 5: 请求量轴**

- 从 /metrics 抓取一次，统计 `redfish_scrape_duration_seconds{bmc=...}` 序列数 = 最近一轮每 BMC 资源抓取次数（佐证）
- 采集面推算：按 src/collector 快/慢组资源清单列出每轮请求构成（快组清单 + 慢组清单），与佐证数核对
- 写入 `resource-summary.txt`：`requests_per_round_dell / requests_per_round_inspur`（推算值与佐证值）

- [ ] **Step 6: 制品归档与提交**

- 全部原始输出落 `%TEMP%\opencode\redfish-resource-run\`；`resource-summary.txt` 汇总四轴结论
- 本任务无代码改动，无 commit（制品不入库）；如真机不可达，如实记录原因并跳过对应轴（spec §9）

---

### Task 3: 对标对比表 + 内存模型文档（design.md 资源节）

**Files:**
- Modify: `docs/design.md`（新增 Resource model 节）

**Interfaces:**
- Consumes: Task 1 的 mock 系数/编码字节、Task 2 的四轴实测；差距表既有结论（`docs/audit/2026-08-21-gap-analysis.md`：对标零缓存全量重爬/重建会话）
- Produces: `docs/design.md` 资源节 + backlog 登记清单（Task 4 引用）

- [ ] **Step 1: 写 design.md 资源节**

在 `docs/design.md` 的「Pre-encoded snapshot cache」条目后新增（占位值由执行者以 Task 1/2 实测值替换，且必须与制品一致）：

```markdown
## Resource model (resources)

- **Per-BMC memory**: each published BMC snapshot holds its prometheus registry (series/label storage) plus one pre-encoded text copy (~1.1MB at Dell scale, added by the performance hardening). Mock measurement: RSS grows linearly with BMC count, {mock_coef} MB per additional BMC (tests/resource_test.rs). Real machines: {after_ws} MB working set / {after_private} MB private for two full snapshots (acceptance baseline before the pre-encoded cache: 44.3MB WS / 31.5MB private), confirming the encoded-cache delta of ~2.2MB for two BMCs.
- **Bounded growth**: snapshot memory grows linearly with the BMC count; the worst-case per-BMC snapshot size is bounded by the pagination defense limits (64MiB per page, 1000 pages, 200,000 members — security hardening).
- **CPU**: request-period CPU/allocations were eliminated by the pre-encoded cache (performance hardening); collect-period parse cost is serde-bound. Measured three-state profile: idle {idle}% / scrape round {round}% / /metrics serving {serve}%.
- **Connections**: {reuse verdict — keepalive/pooling confirmed or not reused, nv-redfish internal client behavior; backlog entry if not reused}.
- **Request volume**: ~{n_dell}/{n_inspur} requests per round per BMC (fast+slow collection surface); ETag caching, fast/slow split and snapshot caching keep BMC-side load below re-walking benchmarks (zero-cache full re-crawl).
```

- [ ] **Step 2: 核对文档一致性**

- design.md 资源节数值与 `resource-summary.txt` / mock 输出逐一核对（诚实记录约束）
- 检查不与其他节矛盾（快照节、稳定性节、安全节）

- [ ] **Step 3: 全量门禁与提交**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`
Expected: 全绿（纯文档改动）

```powershell
git add docs/design.md
git commit -m "docs: resource model section (per-BMC memory, bounds, measured axes)"
```

---

### Task 4: 验收记录 + 决策门留痕 + 全量回归

**Files:**
- Create: `docs/audit/2026-08-24-resource.md`

**Interfaces:**
- Consumes: Task 1/2 证据、Task 3 文档与 backlog 清单；spec §5（S1-S5）、§6（回归门禁）
- Produces: 验收记录（结论 + S1-S5 对照 + 对标表 + 决策门记录 + 遗留跟踪）

- [ ] **Step 1: 决策门执行（spec D2）**

- 汇总 Task 2 全部发现；对每个候选热点执行判定：①是否有社区 crate/现成方案可解决 ②是否属零风险边界内
- **若存在符合决策门条件的热点**：暂停本任务，向用户提交微优化提案（社区方案优先，标注改动面与回归影响）；用户批准 → 追加修复任务（超出本计划），用户否决 → 记 backlog 继续
- **若无**：全部发现记 backlog，继续 Step 2

- [ ] **Step 2: 撰写验收记录**

```markdown
# 0.1.0 资源专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-resource-hardening-design.md` §5 验收标准
范围：五维专项第 4 项（资源）；不破坏第 1/2/3 项（安全/稳定/性能）成果

## 结论

**PASS / CONDITIONAL PASS / FAIL**（执行者按证据填写）

## 决策记录

D1 压缩形态（实测建档 + 微优化，不替换 prometheus 存储/不手写编码器）
D2 微优化决策门（先查社区方案 → 用户批准才改，否则 backlog）

## 验收标准对照（spec §5）

| 标准 | 结果 | 证据 |
|---|---|---|
| S1 四轴实测完成、证据落盘 | | %TEMP%\opencode\redfish-resource-run\ 制品清单 |
| S2 对标对比表 + 结论 | | 下表 |
| S3 内存模型与缓存增量复核文档化 | | design.md 资源节 + 增量复核对照 |
| S4 零回归（安全/稳定/性能 + perf ±5%） | | 门禁输出摘录 |
| S5 决策门执行记录完整 | | 决策门记录节 |

## 对标对比表（资源维度）

| 资源特性 | idrac | fishymetrics | sapcc | 我们（实测） | 结论 |
|---|---|---|---|---|---|
| 缓存 | 零缓存（全量重爬） | 零缓存 | 零缓存（重走树/重建会话） | 快照 + ETag + 预编码 | 领先 |
| 请求量/轮 | 全量重爬 | 全量重爬 | 全量重爬+重连 | {实测值} | 领先 |
| 连接 | {源码级结论} | {源码级结论} | 重建会话 | {Task 2 判定} | {如实标注} |
| 运行时内存 | Go 运行时 | Python | Python | {WS/私有} | {如实标注} |

（对标列内容复用 gap-analysis 既有结论，标注「源码级定性」；我们列必须为实测。）

## 四轴证据

| 轴 | 实测值 | 制品引用 |
|---|---|---|
| 内存 | {WS/私有/系数} | resource-summary.txt + mock 输出 |
| CPU | {三态} | resource-summary.txt |
| 连接 | {判定} | resource-summary.txt |
| 请求量 | {每轮请求数} | resource-summary.txt |

## 决策门记录

| 发现 | 判定（社区方案？风险？） | 处置 |
|---|---|---|
| {发现 1} | | backlog / 用户批准后实施（引用后续任务） |

## 遗留跟踪

- {Task 2 全部 backlog 登记}
- prometheus 存储替换/驻留压降（D1 排除，违反「优先社区资源」规则）
- N×walk 共享预取 / per-BMC 并发（性能维已排除）
- gzip/流式输出、流式响应上限（既有 backlog 维持）
```

- [ ] **Step 3: 全量回归验证**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`; `cargo audit`; `cargo deny check`
再点名：安全四套件（`cargo test --test config_test --test http_test --test bmc_test --test pagination_test`）、稳定套件（`cargo test --test stability_test --test scraper_test --test integration_test`、`cargo test --test soak_test --no-run`）
性能基准对比：`cargo test --release --test perf_test -- --ignored --nocapture`，与性能维基线（round_ms_med 0.33 / encode_ms_med 0.06 / hot_path_ms_med 0.0005 / bytes 8896 / rss_mb 10.9）对比，±5% 容差内
Expected: 全绿 + perf 容差内

- [ ] **Step 4: 提交**

```powershell
git add docs/audit/2026-08-24-resource.md
git commit -m "docs: 0.1.0 resource hardening acceptance record"
```

---

## Self-Review 记录

- **Spec 覆盖**：§3.2 内存轴（Task 1 mock + Task 2 真机）、§3.3 CPU（Task 2）、§3.4 连接（Task 2）、§3.5 请求量（Task 2）、§4.1 对标表（Task 4）、§4.2 内存模型（Task 3）、§5 S1-S5（Task 4）、§6 回归门禁（Task 4）、§7 文档（Task 3/4）、§8 明确不做（计划零 src/ 改动，backlog 登记在 Task 4）。
- **占位符**：模板中 `{实测值}`/`{发现 N}` 为执行期证据捕获点（性能维先例）；无「TODO/适当处理」类指令性空位；`expect_full_round` 指定从 perf_test.rs 逐字复制，无未定义引用。
- **类型一致性**：Task 1 输出命名（`per_bmc_rss_delta_mb`、`encoded_bytes_per_bmc`）在 Task 3 文档槽位（`{mock_coef}`）与 Task 4 证据表对应；四轴制品路径统一 `%TEMP%\opencode\redfish-resource-run\`；决策门流程在 Task 4 Step 1 与 spec D2 一致。
- **边界核查**：Task 2 测量脚本现场编写不入库（性能维 `measure-hot.ps1` 先例）；Task 2 无 commit 合理（制品不入库）；本计划任何任务均不触碰 src/，与 D1「默认零生产代码改动」一致。
