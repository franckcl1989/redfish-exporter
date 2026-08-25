# 功能专项 — 设计文档（v1）

日期：2026-08-24
状态：v1 已确认（设计要点在对话中通过）
目标：0.1.0 发布与投产前五维专项之第 5 项「功能」（最后一项）。定位：对标指标覆盖补齐——以真机证据门控实现对标竞品（idrac_exporter）有而我们有真机数据的指标缺口；无真机数据的候选如实记 backlog。方案：**全清单探测门控**——真机一次性探测全部候选字段，MET 的实现、NOT MET 的记 backlog。
约束：本专项为五维递进的第 5 项——不得破坏第 1 项（安全性）、第 2 项（稳定性）、第 3 项（性能）、第 4 项（资源）成果（§6 回归门禁）。**只增不改**：既有指标族语义冻结，新族只追加不修改。

## 1. 已确认决策记录（对话中逐一确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 专项范围 | 指标覆盖补齐（不包含 P1 可靠性 backlog，其维持 backlog） |
| D2 | 候选与门控 | 全清单一次性探测：CPU 电压/频率、PERC 控制器/RAID 电池、驱动器 OEM 非寿命字段、系统 LED；MET 实现、NOT MET 记 backlog |
| D3 | 指标设计 | 沿用仓库惯例：`redfish_` 前缀 + 单位后缀、label 复用既有、驱动器/控制器归慢组、CPU/LED 归快组 |
| D4 | 验证方式 | mock e2e 测试（期望集扩展）+ 真机实跑验证（新指标族出现在 live-metrics 且数值合理） |

## 2. 现状盘点（探索结论，差距表 backlog）

| 候选 | 真机证据现状 | 结论 |
|---|---|---|
| CPU 电压/当前频率 | Dell/浪潮 ProcessorMetrics 字段可用性未探测（审计 B 组仅确认 Processor 存在性） | 探测门控 |
| PERC 控制器明细/RAID 电池 | Dell 存储 OEM（H 组 H-storage.json）有 DellControllers/ControllerBattery 数据，未采集 | 探测门控（已有数据佐证） |
| 驱动器 OEM 非寿命字段 | Dell `Oem.Dell.DellPhysicalDisk` PPID/WWN/RaidStatus/PowerStatus 有数据（H 组）；寿命字段 null；浪潮无盘 | 探测门控（预期 MET） |
| 系统 LED | Systems/Chassis IndicatorLED 两家存在性未探测 | 探测门控 |
| PDU | 真机无 PDU 设备 | 不做（无收益证据） |
| SSE 事件推送 | 真机无场景 | 不做（nv event-service 现成但无场景） |
| 驱动器寿命字段 | Dell PredictedMediaLifeLeftPercent 为 null（H 组） | 不做（NOT MET，维持 Task 17 backlog） |
| 既有 18 指标族 | 已实现且语义冻结（前四维+审计维） | 只增不改（§8 语义冻结；前四维冻结清单：安全 §9.3 / 稳定 §6.2 / 性能 §6.3 / 资源 §6.4） |

## 3. 探测方案

### 3.1 范围与环境

- 两台真机（Dell <dell-bmc-host> / 浪潮 <inspur-bmc-host>），只读 GET 探测（沿用审计探测组惯例；不 POST/PATCH/DELETE）。
- 探测目标：
  1. ProcessorMetrics：Dell/浪潮 Systems/Processors/{id}/Metrics 的 Voltage（Volts）、PowerConsumedWatts、频率相关字段（如 TotalCores/CurrentSpeed 等按实际 schema 探测）
  2. Dell 存储 OEM：DellControllers（控制器型号/固件/状态）、ControllerBattery（RAID 电池状态）、DellPhysicalDisk 非寿命字段（PPID/WWN/RaidStatus/PowerStatus）
  3. IndicatorLED：Systems（两家）与 Chassis（Dell）
- 制品：`%TEMP%\opencode\redfish-func-probe\` 原始响应落盘；门控结论表（逐字段 MET/NOT MET + 证据文件引用）。

### 3.2 门控规则

- 字段有真实数据（非 null、非空、非错误体）→ MET，纳入实现清单。
- null/缺失/无设备/错误体 → NOT MET，记 backlog（延续 Task 17 先例）。
- 真机不可达或凭据不可用 → 如实记录，该候选转 backlog。

## 4. 指标设计约定

- 命名：`redfish_` 前缀 + 单位后缀（如 `_volts`、`_watts`、`_mhz`）；状态/枚举类字段不加单位。
- label 复用既有惯例：`bmc`、`id`（资源 id）、必要时 `resource_type`/`state` 等。
- 状态类字段（RaidStatus/PowerStatus/IndicatorLED）→ label 模式：值恒 1 + label 携带状态字符串（`redfish_health_status` 先例）。
- 标识类字段（PPID/WWN/控制器型号）→ info 模式：`redfish_info` 族扩展或专属 info 族（按实现时惯例定，保持与既有 `redfish_info{key,value}` 风格一致）。
- 注册分组：驱动器/控制器 OEM → 慢组（storage，与既有 storage 慢组一致）；CPU/LED → 快组（processors/systems/chassis，与既有快组一致）。
- 具体指标清单（名称/help/labels/来源字段）在探测结果确定后写入实现计划（探测先行是计划的前置输入）。

## 5. 验证设计

### 5.1 mock e2e 测试

- 复用 tests/common 期望集扩展：为每个新指标族添加期望载荷（nv-redfish-bmc-mock 的 OEM/ProcessorMetrics/IndicatorLED 载荷按需扩展），断言 `/metrics` 输出含新族且数值正确。

### 5.2 真机实跑验证

- 两台真机运行新二进制，抓取 /metrics：新指标族出现、数值与探测数据一致（同 BMC 同资源字段对照）。

## 6. 回归门禁（不破坏前四项成果 + 冻结语义）

1. **安全回归**：config/http/bmc/pagination/auth 测试全绿；cargo audit（白名单内零新增）/ deny 全绿。
2. **稳定回归**：stability/scraper/integration/soak 基建全绿；状态机、冷却、发布语义不受影响（本维为增量指标，回归为确认性验证）。
3. **性能回归**：perf_test（`#[ignore]` 本地）±5% 容差；/metrics 既有输出不变（等价性测试守护——只增新族，既有序列字节不变）。
4. **资源回归**：resource_test（`#[ignore]` 本地）系数在既有范围内；新增序列数如实记录进验收。
5. **验收记录**：`docs/audit/2026-08-24-function.md`。

## 7. 文档交付

- `docs/metrics.md`：新增指标族条目（名称/help/labels/来源字段）。
- `docs/design.md`：如指标注册路径有新说明则同步。
- 验收记录（§6.5）。

## 8. 明确不做（YAGNI / backlog）

| 项 | 理由 |
|---|---|
| PDU / SSE 事件推送 | 真机无设备/无场景（D1 范围外） |
| 驱动器寿命字段 | 真机 null（Task 17 门控 NOT MET 维持） |
| P1 可靠性 backlog（AUDIT-1/3、2/4/8） | 属可靠性域，非指标覆盖（D1 排除，维持 backlog） |
| 厂商矩阵细节（iLO6/Fujitsu） | 无对应真机型号（遇机型再评估） |
| 健康值数值映射、idrac 命名对齐 | 已有结论不采纳/与惯例冲突（D3） |
| 修改既有 18 指标族 | 语义冻结（本 spec 引言「只增不改」：指标命名/help/labels 冻结） |

## 9. 风险与依赖

| 风险 | 处置 |
|---|---|
| 探测结果 MET 项过少（真机字段普遍 null） | 如实记录：MET 少则实现少，门控结论即本维结论（诚实优先） |
| 真机不可达/凭据不可用 | 如实记录未执行原因；对应候选转 backlog |
| mock 不支持新字段载荷 | 按需扩展 nv-redfish-bmc-mock 期望或以其现有能力覆盖可测面（真机验证兜底） |
| 新指标族使既有 /metrics 输出变化 | 只增新族、既有序列不动；等价性测试守护字节级不变（新族追加在末尾） |
| 指标基数膨胀（info 模式标识字段） | 限定 id 粒度、不引入高基数字段（如日志条目）；新增序列数进验收记录 |

## 10. 验收标准（定义「完成」）

1. **S1**：§3 探测完成，门控结论表（逐字段 MET/NOT MET + 证据引用）落盘。
2. **S2**：MET 项全部实现 + mock e2e 测试全绿（期望集扩展）。
3. **S3**：真机实跑验证完成（新指标族出现、数值与探测数据一致；或如实记录未执行原因）。
4. **S4**：§6 全维回归绿；perf ±5% 容差内；/metrics 既有输出不变。
5. **S5**：§7 文档齐全且与实现一致；§8 语义冻结（只增不改）与前四维冻结清单（安全 §9.3 / 稳定 §6.2 / 性能 §6.3 / 资源 §6.4）生效。

## 11. 与项目既有规范的一致性

- 新指标沿用 18 指标族既有命名/label/分组惯例；collector 按资源分模块惯例。
- 测试沿用 tests/ 集成测试 + 期望集惯例；探测沿用审计探测组惯例。
- 保持 `#![forbid(unsafe_code)]`、零 OpenSSL、依赖最小化（预期零新依赖；如需 mock OEM 载荷扩展优先复用既有依赖）。
- 验收记录沿用 `docs/audit/` 惯例；执行沿用 Subagent-Driven + git worktree 隔离。
