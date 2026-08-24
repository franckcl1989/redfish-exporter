# 性能专项 — 设计文档（v1）

日期：2026-08-24
状态：v1 已确认（设计要点在对话中通过）
目标：0.1.0 发布与投产前五维专项之第 3 项「性能」。定位：性能须超越对标竞品。方案：**出口与微观优化**——预编码快照缓存（/metrics 热路径毫秒级）+ 编码/注册微观优化 + 本地 mock 基准与真机前后对比。不触碰 N×walk 采集结构与 per-BMC 并发（风险递增项，经决策排除，保持零稳定性风险）。
约束：本专项为五维递进的第 3 项——不得破坏第 1 项（安全性）与第 2 项（稳定性）成果（§6 回归门禁），其成果构成后续两维（资源/功能）的不可破坏基线。

## 1. 已确认决策记录（对话中逐一确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 优化范围/风险边界 | 出口与微观优化——不碰 N×walk 结构、不碰 per-BMC 并发 |
| D2 | 测量方式 | 仓库内 `#[ignore]` mock 性能基准 + 验收周期真机前后对比 |

## 2. 现状盘点（探索结论）

| 热点 | 现状 | 结论 |
|---|---|---|
| /metrics 出口 | 每次请求现场编码 ~1.1MB String（9745 序列热延迟 ~310ms，验收报告 §3） | 核心优化点：预编码缓存 |
| 编码/注册路径 | `register_into` 每 metric 每 label 线性查找（O(L²)）、无预分配 | 微观优化 |
| 采集侧 | ETag 缓存已用（Dell 全量轮 7.69s 已领先对标）；快/慢组分频已有 | 保持（D1） |
| N×walk | 每 collector 重复枚举（浪潮 ~45 请求/轮） | 不触碰（D1，既有 Ruling 7c 取舍） |
| per-BMC 并发 | 恒为 1（D5 刻意设计，稳定性保障） | 不触碰（D1） |

## 3. 预编码快照缓存设计（核心）

### 3.1 存储层（registry.rs Snapshot）

- Snapshot 内值类型：`Arc<RegistryEntry>`，`RegistryEntry { registry: Arc<prometheus::Registry>, encoded: Bytes }`（Bytes = 发布时编码一次的文本）。
- `Snapshot::update(name, Arc<Registry>)` 签名不变：内部在 update 时调用 `encode(registry)` 一次并存储——调用方（scraper 成功/失败/cooldown/degraded 各发布路径）零改动。
- 编码成本从请求路径移至发布路径（每轮每 BMC 一次，~310ms 计入轮内无感知）；多 Prometheus 实例/高频抓取场景请求期 CPU 归零。

### 3.2 /metrics 出口（http.rs）

- `metrics_handler`：遍历快照，将各 BMC 的 `encoded` 字节拼接进预分配的 `Vec<u8>`（容量 = 各 entry 长度和），直接作为响应体返回；CONTENT_TYPE 与 `no-data-yet` 头路径不变。
- 热延迟：~310ms → ~ms 级（仅内存拷贝）。

### 3.3 一致性、内存与边界

- 编码文本随轮与 registry 原子替换——与既有快照一致性语义完全一致。
- 内存代价：每 BMC 多存一份编码文本（Dell ~1.1MB）；换取延迟与请求期 CPU/分配。文档记录取舍。
- 输出字节与旧实现**逐字节一致**（等价性测试钉住，§5）。
- `encode()` 保留（发布路径使用）。

## 4. 编码与注册微观优化

1. `register_into`：每 metric 先构造 label→value 映射（`HashMap<&'static str, &String>` 或等价），label_values 组装 O(L)；`Vec::with_capacity(metrics.len())` 预分配输出与 vecs 容量。
2. 发布路径编码缓冲复用：跨 BMC 复用 `String` 缓冲（`clear()` 后 `encode_utf8` 追加写入），减少每轮分配。
3. /metrics 响应体改用 `Vec<u8>` 拼接（免除 String 二次转换）。

## 5. 基准与验证设计

### 5.1 等价性测试（常规测试，进 CI）

- 同一 registry 双路径输出断言：`encode(registry)` 与快照存储的 `encoded` 字节完全相等；`metrics_handler` 响应体与旧实现（现场编码拼接）输出一致。守护「/metrics 输出字节不变」语义冻结。

### 5.2 mock 性能基准（`tests/perf_test.rs`，`#[ignore]`，本地跑）

- 确定性 mock 环境（复用 tests/common 期望集）：
  - 全量轮耗时：collect_fast + collect_slow + build_registry + encode，median of N=20
  - encode 耗时（发布路径）
  - 预编码 /metrics 热路径耗时（拼接路径）
  - RSS 采样（复用 sysinfo dev-dep）
- 输出表格；`PERF_ASSERT=1` 时启用软阈值断言（热路径 <10ms、编码路径单次 <500ms @ mock 规模），本地执行不进 CI。

### 5.3 真机前后对比（验收周期）

- Dell/浪潮可达：/metrics 热延迟多次请求 p50、轮耗时、内存的优化前后对比；与验收报告 §3 基线（热延迟 310ms、Dell 快组 ~3s/全量 ~11s）对照。
- 证据目标：热延迟 310ms → <10ms（~30 倍）；请求期 CPU/分配消除；结合既有领先项（ETag 缓存、快照架构、分页）构成超越对标证据链。

## 6. 回归门禁（不破坏前两项成果 + 冻结语义）

1. **安全回归**：config_test/http_test/bmc_test/pagination_test 全绿；cargo audit（白名单内零新增）/ deny 保持全绿。
2. **稳定回归**：stability_test/scraper_test/integration_test/soak 基建全绿；状态机、冷却、发布语义不受影响（本专项不触碰调度层，回归为确认性验证）。
3. **性能语义冻结**：/metrics 输出字节不变（等价性测试守护）；快照内部表示（预编码缓存）为内部实现、不构成对外语义；mock 基准与真机数据构成性能证据基线，供后续两维（资源/功能）回归参照。
4. **验收记录**：`docs/audit/2026-08-24-performance.md`。

## 7. 文档交付

- `docs/design.md`：快照节更新（预编码缓存机制 + 内存/延迟取舍）
- README：无需变更（无配置/行为变化）；如内存占用说明有更新则同步
- 验收记录（§6.4）

## 8. 明确不做（YAGNI / backlog）

| 项 | 理由 |
|---|---|
| N×walk 共享预取重构 | D1 排除；打破 collector 隔离性，风险高，收益归性能维度大改 |
| per-BMC 并发提升 | D1 排除；与稳定性专项的 BMC 保护冲突 |
| criterion 微基准套件 | D2 已选 mock 端到端基准；新依赖+维护面，粒度收益低 |
| 指标输出压缩（gzip/流式） | Prometheus 抓取侧无需；backlog |
| 采集侧请求合并/`$expand` 优化 | 属结构改动（D1 排除）；backlog |
| 自定义分配器/无锁快照 | 收益未证实；当前 RwLock 读路径仅 Arc 克隆已足够；backlog |

## 9. 风险与依赖

| 风险 | 处置 |
|---|---|
| 预编码缓存内存增量（每 BMC ~1.1MB） | 文档记录；资源维度（第 4 项）将复核总量 |
| 编码移至发布路径后轮耗时增加 ~310ms | 轮内无感知（轮以秒计）；perf 基准记录前后对比 |
| 输出字节不一致（编码路径改动引入差异） | 等价性测试（§5.1）钉住逐字节一致 |
| perf 基准在 CI 环境抖动 | `#[ignore]` 不进 CI；仅本地+验收周期执行 |
| update() 内部编码失败路径 | encode 为不可失败不变量（既有 expect 注释）；保持 |

## 10. 验收标准（定义「完成」）

1. §5.1 等价性测试全绿；全部测试 + clippy -D warnings + fmt + doc 全绿。
2. §5.2 mock 基准证据表格记录进验收记录（轮耗时/encode/热路径/RSS）。
3. §5.3 真机前后对比完成（或如实记录未执行原因）。
4. §6.1/6.2 安全+稳定双回归全绿。
5. §7 文档交付齐全且与实现一致；§6.3 语义冻结清单生效。

## 11. 与项目既有规范的一致性

- 不破坏 design.md 既有架构（快照缓存语义、调度、发布路径）；改动为 registry/http/metrics 三文件的增量优化。
- 测试沿用 tests/ 集成测试 + `#[ignore]` 惯例（soak 先例）；pub 测试接缝惯例沿用。
- 保持 `#![forbid(unsafe_code)]`、零 OpenSSL、依赖最小化（本专项零新依赖）。
