# 工程质量审计 — 设计文档

日期：2026-08-21
状态：已确认（设计第 1–3 节在对话中通过）
目标：以三个社区常用的开源 Redfish exporter 为对标，在真实硬件上找到并修复我们的缺陷、吸纳对方的优点 —— 前提是不破坏 nv-redfish 核心依赖链，且 nv 已经提供的能力不做重复自研。

## 对标项目

| 项目 | 技术栈 | 主要优点 |
|---|---|---|
| mrlhansen/idrac_exporter | Go | 指标族最丰富（事件日志、控制器、RAID 电池、PSU 明细、PDU、Dell OEM）；`/reload` 热加载、`/discover`（Prometheus 服务发现）、build info + `scrape_errors_total`；Helm chart、Grafana 面板 |
| comcast/fishymetrics | Go | 部分抓取（partial scraping）；404 重试策略（`disable-404-retry`）；Vault/凭据脚本；HTTP 代理支持；`/info` 端点；模块排除（exclude）标志 |
| sapcc/redfish-exporter | Python | 厂商测试矩阵广（22+ 机型）；按指标族分组端点；内存可纠错/不可纠错错误计数；BIOS 属性导出为指标；`redfish_response_duration_seconds`；环境变量覆盖凭据 |

## 约束

1. **nv 门禁**：nv-redfish 0.15 已提供的能力（类型、端点、解析）一律直接使用。只有 nv 不提供的才在 exporter 层自研（编排、缓存、HTTP 层策略）。
2. **依赖链门禁**：绝不 fork 或修改 nv-redfish 源码；绝不替换核心抓取管线里的 nv 类型；不为 nv 已做的事引入新的重量级依赖。
3. **收益门禁**：候选改进项只有在真机探测证明实际存在可采集的数据时，才进入实施计划 —— 不为对齐而对齐。
4. 对真实 BMC 只做只读 Redfish 操作：不对在线资源做 POST/PATCH/CREATE。凭据只出现在 gitignore 的临时文件中，绝不进仓库、绝不进日志。

## 真机测试目标

- `https://10.10.90.70/` — Dell PowerEdge R750（iDRAC），账号 `root`
- `https://10.10.90.80/` — 浪潮服务器，账号 `admin`
- 凭据由操作方提供，仅存放在 `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\`（已 gitignore）。

## 第 1 节 — 实证审计执行方式

**A. 真机 API 探测（Python 3 标准库一次性脚本，放临时目录）**
- 对两台 BMC 用 basic 认证探测：遍历 `ServiceRoot -> 各 Collection`，对关键资源递归抓取 JSON 快照：
  Systems / Chassis / Managers / Thermal / Power / Sensors / Processors / Memory / Storage（Drives、Volumes）/ NetworkAdapters（EthernetInterfaces、Ports）/ PCIeDevices / UpdateService（FirmwareInventory）/ LogService（EventLog）/ Bios / EnvironmentMetrics / Controls。
- 逐资源输出报告：HTTP 状态、延迟、字段存在性。
- 快照存于 `Temp\opencode\redfish-audit\<bmc-name>\`。

**B. 实跑 exporter 交叉对比**
- 本地 `cargo run`（Windows 直接可跑），配置两台 BMC 用 `session` 认证，`scrape_interval=30s`。
- 采集 >=3 轮 `/metrics` 输出和 `info/debug` 日志，逐 BMC 记录：指标族数量、`redfish_up`、`failed_resources` 报错、scrape 时长。
- 把 A（API 表面）与 B（导出指标）交叉对比：API 存在但无指标 -> 覆盖缺陷；数值异常 -> 解析缺陷；请求失败/超时 -> 可靠性缺陷。
- session 认证为默认测试模式；若某厂商拒绝建会话（浪潮是已知风险点），该 BMC 回退 basic 认证，并把该行为记为缺陷（兼容性问题），而非配置错误。

**C. 缺陷分级**：
- P0：数据损坏 / 崩溃
- P1：核心指标族缺失 / 数值错误
- P2：边缘缺失 / 厂商兼容性
- P3：体验 / 打磨
- 每条缺陷附脱敏证据（请求/响应/日志片段）。

## 第 2 节 — 对标差距表结构

产出物：`docs/audit/2026-08-21-gap-analysis.md`。行 = 从三个对标项目提取的候选功能点；列 = 对标项目现状、我们现状、nv 能力映射、差距等级、建议。

维度与候选功能点：

| 维度 | 候选功能点（来源） |
|---|---|
| 指标覆盖 | 事件日志条目时间戳（idrac）、内存可纠错/不可纠错计数（sapcc）、CPU 电压/当前频率（idrac）、PSU 效率/输入电压、功耗 min/max/avg + 间隔（idrac）、驱动器剩余寿命/指示灯（idrac）、控制器/RAID 电池健康（idrac）、BIOS 属性 + pending 标志（sapcc）、PDU（idrac，可缓做）、网络端口状态/速率（idrac）、系统 LED/指示灯（idrac） |
| 可靠性 | 404 快速跳过与可配置重试（fishymetrics）、分页处理、请求级失败统计、`redfish_response_duration_seconds`（sapcc） |
| 运维 | 配置热加载 `/reload`（idrac）、`/discover` Prometheus 服务发现（idrac）、build_info + `scrape_errors_total`（idrac）、`/info`（fishymetrics）、Grafana 面板、Helm chart |
| 性能 | 部分抓取（fishymetrics）、共享 collection 预取（消除各 collector 重复遍历）、404 不重试 |
| 安全 | 环境变量凭据覆盖（sapcc）、Vault/凭据脚本（fishymetrics，评估成本后大概率入 backlog）、HTTP 代理支持（fishymetrics）、监听加固 |

每个候选功能点过三关门禁（见"约束"1–3）。

## 第 3 节 — 交付物与验收标准

1. `docs/audit/2026-08-21-bmc-audit.md` — 真机审计报告：两台 BMC 的 API 基线、exporter 实跑结果、P0–P3 缺陷清单（附证据）、交叉对比结论。
2. `docs/audit/2026-08-21-gap-analysis.md` — 差距表（含三关门禁结论）。
3. 本文档已提交至 `docs/superpowers/specs/`。
4. 实施计划通过 writing-plans 流程产出。

验收标准：
- 每条 P0/P1 缺陷在真机或 nv mock 上可复现。
- 差距表每一行有三关门禁的明确结论与建议（实现 / backlog / 不采纳 + 理由）。
- 每个结论都有原始数据支撑，不虚构。
