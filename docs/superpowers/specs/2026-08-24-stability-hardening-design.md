# 稳定性专项 — 设计文档（v1）

日期：2026-08-24
状态：v1 已确认（设计要点在对话中通过）
目标：0.1.0 发布与投产前五维专项之第 2 项「稳定性」。定位：作为监控组件必须持久稳定地提供服务——自身不能频繁崩溃出错，也不能给目标（远程管理卡）造成负面影响。方案：**分层稳定性模型**——运行时韧性层（崩溃面收敛）+ 自适应调度层（退避冷却 + 会话降级兜底）+ 验证层（故障注入/本地 soak/CI 冒烟）。
约束：本专项为五维递进的第 2 项——不得破坏第 1 项（安全性）成果（§6 回归门禁），且其成果构成后续三维（性能/资源/功能）的不可破坏基线。

## 1. 已确认决策记录（对话中逐一确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 验收标准 | 防御实现 + 故障注入矩阵 + 本地 soak（真机长稳推迟到投产前灰度观察） |
| D2 | BMC 保护策略 | 指数退避冷却（连续失败 3 轮触发，60s→120s→240s→上限 300s，成功即复位） |
| D3 | 会话恢复兜底 | 冷却期 basic 兜底采集：401 重登失败后 basic 凭据照常采集 + session 按退避节奏重试，两轨并行 |
| D4 | Linux 优雅停机验证 | CI 新增 Linux 冒烟 job（SIGTERM → 退出码 0 + shutdown complete 日志） |

## 2. 现状盘点（探索结论）

| 项 | 状态 |
|---|---|
| unwrap/expect 面 | 18 处：scraper.rs 7 处 `.unwrap()`（Mutex 中毒）、registry.rs 6 处 `.expect("poisoned")`、http.rs 1 处 `unreachable!`、main.rs 2 处信号 `expect`、metrics.rs 2 处静态串 `expect` |
| BMC 任务 panic | JoinSet 已隔离（warn + 该 BMC up=0），不拖垮进程——保持 |
| scraper 主任务 panic | exit 1 fail-fast（不服务陈旧数据）——保持 |
| 401 风暴 | 现状每轮重登+整轮重试 ×1，无冷却；凭据失效时每 30s 打 3+ 请求——需冷却 |
| 不可达 BMC | 每轮全量重试（5s 连接超时 + 10s 请求超时），无退避——需冷却 |
| 慢组节流 | 慢组失败按 slow_interval 节奏重试（已有）——保持 |
| 长稳基础 | MissedTickBehavior::Skip（不追时间）、快照原子替换、slow_state 有界——保持 |
| 自愈已有 | 401 单次重登（ACC-4）、up=0 可见发布、slow last-good、启动 basic 兜底——保持 |
| 优雅停机 | 路径已实现 + 单测；Linux SIGTERM 冒烟未做（验收跟踪项）——本次补（D4） |

## 3. 运行时韧性层设计（崩溃面收敛）

1. **Mutex 中毒恢复**：scraper.rs 7 处 `.unwrap()` 与 registry.rs 6 处 `.expect("poisoned")` 统一改为 `lock().unwrap_or_else(|p| p.into_inner())` 恢复语义 + `error!` 日志（中毒说明发生过任务 panic，恢复后继续服务，避免连锁 panic）。tokio RwLock 无中毒机制，不受影响。
2. **`unreachable!` 消除**：http.rs `load_tls` 的 cert/key 不配对分支改为返回 `anyhow::Error`（防御性分支，消除 panic 路径；配置校验已保证常规不可达）。
3. **信号安装**：main.rs Ctrl+C/SIGTERM 安装失败由 `expect` panic 改为 `error!` + `std::process::exit(1)`（无 panic 栈噪音，退出语义不变）。
4. **静态串 expect**：metrics.rs 2 处为不可能失败不变量（静态名构造 GaugeVec、TextEncoder 写 String），保留并加注释说明不变量依据。
5. **保持项**：BMC 任务 panic 隔离、主任务 fail-fast、panic hook 默认（安全专项已评估无敏感信息输出）。

## 4. 自适应调度层设计（BMC 保护核心）

### 4.1 per-BMC 状态机

```
               连续失败 ≥ cooldown_failures
Healthy ───────────────────────────────▶ Cooling ──到期重试成功──▶ Healthy
  ▲                                          │  到期重试失败 → 继续退避
  │            session 重试成功               │
  └──────────────────────────────────────────┤
SessionDegraded ◀── 401 重登失败（仅 session BMC）
  │   basic 采集成功：up=1 + session-degraded 标记（继续此状态）
  └── basic 连续失败 ≥ cooldown_failures ──▶ Cooling
```

- **Healthy**：现状采集行为（session BMC 的 401 → 单次重登 + 整轮重试一次；不改变）。
- **SessionDegraded**（仅 session BMC，401 重登失败后进入）：basic 凭据照常每轮采集（快/慢组调度不变）；session 重试按退避节奏穿插（同 Cooling 的退避序列）；重挂成功 → Healthy；basic 连续失败达阈值 → Cooling。
- **Cooling**：全停采集。不发任何请求；每轮发布 `redfish_up{bmc}=0` + `redfish_scrape_error{resource="cooldown"}=1`（序列保持可见）；不累计 `redfish_scrape_errors_total`（冷却不是新错误）；退避 `min(cooldown_max, cooldown_base × 2^n)`；到期执行重试轮：成功 → Healthy（计数清零）；失败 → 计数 +1、按新退避继续。
- 成功即复位；与慢组节流、per-BMC 截止时间正交共存（恢复后组调度照常）。

### 4.2 配置（新增 `stability` 节，语义 0.1.0 冻结）

```yaml
stability:
  cooldown_failures: 3     # 连续失败阈值（默认 3）
  cooldown_base: "60s"     # 首次退避（默认 60s）
  cooldown_max: "300s"     # 退避上限（默认 300s）
```

- 校验：三者均须非零；`cooldown_max ≥ cooldown_base`；否则配置错误。
- 状态机抽为纯函数 seam 便于单测（如 `next_backoff(failures, base, max)`、`on_round_result(state, result) -> (state, action)`，与 scraper.rs 现有 `slow_due`/`apply_slow_result` pub 测试 seam 惯例一致）。

### 4.3 发布语义汇总（0.1.0 冻结）

| 状态 | up | scrape_error resource | errors_total |
|---|---|---|---|
| Healthy 成功 | 1 | 无 | 不增 |
| Healthy 失败 | 0 | 具体失败资源 | +1/轮 |
| SessionDegraded（basic 成功） | 1 | `session-degraded`（持续提示） | 不增 |
| SessionDegraded（basic 失败） | 0 | 具体失败资源 | +1/轮 |
| Cooling（等待） | 0 | `cooldown` | 不增 |
| Cooling（到期重试失败） | 0 | 具体失败资源 | +1/轮 |

## 5. 验证层设计

### 5.1 故障注入矩阵（tests/，全部进常规 CI）

| 组 | 覆盖点 |
|---|---|
| 状态机单元（scraper 纯函数） | 连续失败阈值触发；退避序列 60/120/240/300 封顶；到期重试成功复位；失败计数递增；SessionDegraded 转换全路径 |
| 冷却发布语义 | up=0 + resource=cooldown；errors_total 不累计；到期重试失败按正常失败计数 |
| 真实 401 场景 | 本地 axum mock server + reqwest + HttpBmc 构造真实 401（MockBmc 错误类型无法表达 HTTP 状态码）：401 → 重登失败 → SessionDegraded basic 采集 up=1 + 标记 → basic 连续失败 → Cooling |
| 错误/乱码响应 | MockBmc 期望队列注入解析错误（乱码 JSON）→ 轮失败 → 连续触发冷却 |

### 5.2 本地 soak（发布前跑一次，不进 CI）

- `tests/soak_test.rs`：`#[ignore]`，`SOAK_SECS` 参数化（默认 14400s，短跑模式可配）；健康 MockBmc 长期循环
- 断言/记录：轮耗时统计无递增漂移、错误计数零增长、指标输出大小稳定、RSS 采样曲线（dev-dep `sysinfo`，跨平台）
- 证据记入验收记录（运行时长、采样曲线摘要）

### 5.3 CI Linux SIGTERM 冒烟（ci.yml 新 job）

- ubuntu runner：cargo build → 临时 config（不可达 BMC + 长 interval）→ 后台启动 → sleep → `kill -TERM` → 等待退出 → 断言退出码 0 且日志含 `shutdown complete`
- 会话删除路径已有单测覆盖，不重复

### 5.4 真机回归（有条件）

- Dell/浪潮可达则验证：默认配置（Healthy 路径）行为与基线一致——冷却机制对健康 BMC 零影响；不可达则如实记录原因（延续验收报告惯例）

## 6. 回归门禁（不破坏上一项成果 + 冻结语义）

1. **安全回归**：安全专项测试套件（config_test/http_test/bmc_test/pagination_test）全量重跑；cargo audit（白名单内零新增）/ deny 保持全绿。
2. **稳定性语义冻结**（0.1.0）：`stability` 配置节语义、冷却状态机行为、SessionDegraded 标记、§4.3 发布语义表——后续维度只增不改；任何改动须在对应维度设计中显式声明并重跑稳定性测试。
3. **验收记录**：`docs/audit/2026-08-24-stability.md`（延续惯例），含 soak 证据与真机回归结果。

## 7. 文档交付

- `docs/design.md`：新增「自适应调度」节（状态机 + 发布语义表）
- README：Configuration 表新增 `stability` 行
- `config.example.yaml`：`stability` 节示例 + 注释
- 验收记录（§6.3）

## 8. 明确不做（YAGNI / backlog）

| 项 | 理由 |
|---|---|
| 真机 24-72h 长稳挂机 | D1 已定推迟到投产前灰度观察 |
| 请求预算调度器（per-BMC 并发/请求数配额） | 单 BMC 并发恒 1 已确认；请求数问题属性能维度 |
| 熔断后健康探测降级（HEAD 轻量探测） | 冷却已全停请求；探测收益低，backlog |
| Windows 本地 soak 的 RSS 平台差异处理 | sysinfo 跨平台已覆盖 |
| panic hook 定制 | 安全专项已评估默认行为足够 |
| 会话失效预检（expiration_time 提前重登） | 401 自愈路径已覆盖，预检收益未证实，backlog |

## 9. 风险与依赖

| 风险 | 处置 |
|---|---|
| 状态机改动影响 Healthy 路径（健康 BMC 行为漂移） | Healthy 分支零改动（仅失败路径新增状态）；真机回归验证 |
| 真实 401 测试的本地 server 依赖 | Task 5 已有真实 server 测试先例；若复杂度过高，退回 MockBmc + 状态机注入 seam 方案 |
| sysinfo dev-dep 引入 | 仅 dev-dependency；依赖树审查保持零 OpenSSL |
| 冷却期间监控盲区（up=0 持续到重试成功） | 设计语义：冷却轮 up=0 可见即"盲区可见化"；告警规则已覆盖 up=0 |
| CI Linux job 的进程管理（bash 后台/信号） | 脚本化 + 超时护栏（timeout 命令），失败即 job 红 |

## 10. 验收标准（定义「完成」）

1. §5.1 故障注入矩阵全绿；全部测试 + clippy -D warnings + fmt + doc 全绿。
2. §5.2 soak 证据（≥2h 连续运行，采样曲线记录）记入验收记录。
3. §5.3 CI Linux SIGTERM 冒烟 job 绿。
4. §5.4 真机回归通过或如实记录未执行原因。
5. §6.1 安全回归全绿（不破坏第 1 项成果）。
6. §7 文档交付齐全且与实现一致；§6.2 语义冻结清单生效。

## 11. 与项目既有规范的一致性

- 不破坏 design.md 既有架构（快照缓存、认证流、优雅停机、per-BMC 截止）；稳定性为调度层增量（scraper 状态机）。
- 状态机 seam 遵循 scraper.rs 既有 pub 纯函数测试惯例；配置项沿用 humantime/serde 约定与 `ConfigError::Invalid` 语义。
- 保持 `#![forbid(unsafe_code)]`、零 OpenSSL、依赖最小化三条既有基线。
