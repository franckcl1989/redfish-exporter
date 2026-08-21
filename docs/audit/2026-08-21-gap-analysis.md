# 对标差距表（idrac_exporter / fishymetrics / sapcc vs 我们）

日期：2026-08-21
来源：spec `docs/superpowers/specs/2026-08-21-engineering-quality-audit-design.md` §1（三项目源码级分析）、§3（我们现状）、§7（三关门禁预判）+ 真机审计 `docs/audit/2026-08-21-bmc-audit.md`（AUDIT-1..9）。
维度：指标族 / 调度 / 可靠性 / 运维 / 安全。

三关门禁定义：① nv 门禁（nv-redfish 0.15.1 是否提供能力或需自研）；② 依赖链门禁（是否破坏 nv 核心依赖链）；③ 收益门禁（真机证据是否支持投入）。
结论五类：实现（映射 Task 7–16/18）/ 部分实现 / backlog / 不采纳（附理由）。

## 维度一：指标族

| 功能点 | idrac | fishymetrics | sapcc | 我们现状(含审计证据) | nv 能力 | 三关门禁 | 结论 |
|---|---|---|---|---|---|---|---|
| 事件日志指标 | `events_log_entry` 系列 | 无 | 无 | 无日志指标（AUDIT-5②：Dell Sel 76/Lclog 1062 条、浪潮 SEL/AuditLog/IDL 各 100 条均未采集） | `log-services` 特性现成（`Manager::log_services()`/`LogEntry`） | ①通过（增量特性）②通过 ③通过（D 组 SEL 存在） | **实现**（Task 8，注册慢组） |
| BIOS 属性 + pending | 无 | 无 | `Systems/{id}/Bios` 每属性一指标 + `@Redfish.Settings` pending | 无 BIOS 指标（AUDIT-5③：两家 Bios 资源均在，浪潮探测 200/~166KB；BIOS 属性无口令类字段可导出，见审计共性结论④） | `bios` 特性现成（`Bios::raw()`/`attribute`） | ①通过 ②通过 ③通过（B 组两家 Bios 均 200） | **实现**（Task 9，注册慢组） |
| 内存纠错计数 | 无 | 无 | `HealthData.AlarmTrips.Correctable/UncorrectableECCError` | 无纠错指标（AUDIT-5④：Dell 4 条 DIMM MemoryMetrics 链接全部 200） | `MemoryMetrics.health_data.alarm_trips` 现成（bool） | ①通过 ②通过 ③通过（B 组 Metrics 200） | **实现**（Task 10） |
| PSU 明细与功耗统计 | PSU 效率/输入电压/容量、功耗 min/max/avg | 无 | 无 | 仅瞬时 `redfish_power_consumption_watts`（AUDIT-5①：Dell PowerMetrics 297/318/300 W 未导出；浪潮 PowerMetrics 缺失，实跑单序列） | `PowerMetrics`/`PowerSupply` 类型现成（interval/min/max/avg/efficiency/input_voltage） | ①通过 ②通过 ③通过（F 组 Dell PowerMetrics 存在） | **实现**（Task 11；legacy 路径直采与审计共性结论①一致，新 PowerSubsystem 路径不投入——两家均无，见审计 §4） |
| 驱动器 OEM 寿命/指示灯 | 驱动器寿命 + 指示灯 | 无 | 无 | 仅 `redfish_drive_capacity_bytes`/`predictive_failure`；Dell `Oem.Dell.DellPhysicalDisk` 未采集（AUDIT-5⑥） | `oem` 特性 `oem_value` 可读任意 OEM 字段 | 门控 NOT MET：Dell `PredictedMediaLifeLeftPercent` 为 **null**（H 组），浪潮无盘（存储成员 17034 错误） | **backlog**（Task 17 门控不通过，记录待有可用 OEM 寿命数据的真机型号再评估） |
| 其他 idrac 指标缺口（CPU 电压/当前频率、控制器明细/RAID 电池、系统 LED） | 有对应指标 | 无 | 无 | 实跑 17 指标族中均无（AUDIT-5 覆盖清单未含；Dell PERC H755 控制器数据存在但未采集） | ProcessorMetrics/PowerSupply/IndicatorLED 类型部分现成 | ①部分通过 ②通过 ③未证实（无对应探测组专门验证） | **backlog**（未列入 Task 7–18；无真机字段可用性证据，待后续任务评估） |
| PDU | 支持 PDU 采集 | 无 | 无 | 无 PDU 支持；真机无 PDU 设备 | `power-equipment` 特性现成 | ①通过 ②通过 ③未证实（真机无 PDU，收益未证实） | **backlog**（spec §7 既定） |
| 健康值语义 | 0/1/2 数值映射 + label 保留状态字符串 | 无 | 数值映射 | label 方案（`redfish_health_status{health,state}`，collector/mod.rs push_health） | — | ①— ②— ③— | **不采纳**（D11：数值映射破坏现有 PromQL（`redfish_health_status` 当前值为恒 1 + label），label 方案保留；现有 61 条 health 序列全量可用） |
| 厂商矩阵细节（Dell SKU 当 serial、iLO6 静默 PSU、Fujitsu 去空白、固件仅取 URL 含 Installed） | — | — | 均为真机 bug 的编码 | 无厂商分支（serial 直取；实跑 Dell 7 个 serial_number 正常） | — | ①— ②— ③未证实（当前无 iLO6/Fujitsu 机型；Dell 真机 serial 字段可用） | **backlog**（遇对应真机型号再评估；固件 Installed 过滤可并入后续固件族优化） |

## 维度二：调度

| 功能点 | idrac | fishymetrics | sapcc | 我们现状(含审计证据) | nv 能力 | 三关门禁 | 结论 |
|---|---|---|---|---|---|---|---|
| 分频采集（快/慢组） | 单周期 | 部分抓取 `?components=` + Prometheus 多 job 不同 scrape_interval | 按指标族拆 5 端点配不同 interval | 每轮全量采集全部 BMC（AUDIT-7：浪潮 45 请求×mean 1480ms≈67s 使 60s 截止必然超时，慢组应 ≥120s，审计 §5） | 纯编排层（scraper 慢组状态 + merge_reports） | ①通过 ②通过 ③通过（K 组延迟基准：Dell 23s / 浪潮 67–89s） | **实现**（Task 7：sensors/power/processors/memory/systems/chassis_health/managers 快组 + storage/network/firmware/assembly 慢组；slow_interval 默认 None 行为不变） |
| 部分抓取 `?components=` | 无 | 按组件门控发现与任务构建 | 无 | 周期快照架构，无按请求门控 | — | ①— ②— ③— | **不采纳**（与周期抓取+快照发布架构相冲突；"不同指标不同周期"目标已由 Task 7 快/慢组达成，无需 per-request 门控） |
| 共享预取（消除 N×walk） | 无（逐 collector 各自爬） | 无 | 无 | 每 collector 重复遍历 collection（D2，design.md 已承认；collector/mod.rs 顺序 await 各自发现） | 无现成（需重构 collect_fast/slow 的 root 传递） | ①— ②有重构风险 ③— | **backlog**（spec §7 既定：与 collector 隔离性冲突、有重构风险） |
| 每主机并发上限/信号量 | 加权信号量（默认 10）+ MaxIdleConnsPerHost | 串行 pool=1 | 线程无上限（硬伤） | 单 BMC 请求并发恒 1（D5 已复核：collect_all 内 collector 顺序 await，collector/mod.rs:80-123）；跨 BMC JoinSet 并行是刻意设计 | `ConcurrencyLimitedBmc`（http-extras）未用 | ①通过 ②通过 ③已消除（D5） | **不采纳**（D5 修正后无并发冲击场景；sapcc 线程无上限为硬伤不学） |
| 采集去重（同 target 并发合并） | sync.Cond 合并同 target 请求 | 无 | 无 | 无 on-demand 并发抓取场景（周期快照 + /metrics 读缓存） | — | ①— ②— ③— | **不采纳**（快照架构无同请求并发场景，去重无适用面） |
| 请求间强制睡眠/串行 | 无 | 每请求强制 100ms 睡眠；全量 60-100+ 请求≈6-10s 纯睡眠（硬伤） | 无 | 无强制睡眠；ETag/304 缓存下 Dell 全轮 7.69s（审计 §2.2） | — | ①— ②— ③— | **不采纳**（fishymetrics 硬伤：人为拖慢采集，我们以缓存与并行取胜） |

## 维度三：可靠性

| 功能点 | idrac | fishymetrics | sapcc | 我们现状(含审计证据) | nv 能力 | 三关门禁 | 结论 |
|---|---|---|---|---|---|---|---|
| 404 快速跳过/重试 | 无 | 404 重试 3×2s（可关闭） | 无 | 无 404 策略，瞬时 404 直接计失败资源（D4）；collector 错误 String 化、状态码丢失 | `BmcError::InvalidResponse{status}` 可精确匹配 404；超时 `is_timeout()` | ①通过 ②通过 ③部分（真机两家 15 次抓取零 404，审计 §1.1/1.2） | **部分实现**（Task 14：仅 service-root 404 快速跳过整轮；其余 404 仍计 failed → backlog，理由：错误经 String 化丢失状态码，判别需 collector 具体类型化改造，成本大于当前收益，真机 404 发生率零佐证）；重试机制**不采纳**（不学 fishymetrics 3×2s，瞬时 404 由跳过+计数覆盖）。最终结论：部分实现：ServiceRoot 404 快速跳过（up=0 不计数）；其余 404 计失败（错误 String 化丢失状态码，判别需 collector 具体类型化，成本大于收益） |
| 分页（nextLink） | 无（硬伤） | 无 | 无 | 无分页支持（AUDIT-6：两家日志条目均以 `Members@odata.nextLink` 分页，Dell Lclog `?$skip=50`/50 页、浪潮 `?$skip=100&$top=100`；D 组只查 `@odata.nextLink` 键导致误判） | 全库无分页（spec §2.2），需 raw fetch 自研 ~10 行（patch_support/payload.rs 先例） | ①自研 ②通过（不动 nv） ③通过（门控 MET：J 组确认两家日志条目均分页） | **实现**（Task 15：raw fetch 分页 walker，`@odata.nextLink` 与 `Members@odata.nextLink` 两个键变体均支持；事件日志接入，无分页时与现状等价——超越三项目） |
| 失败/坏主机隔离与超时发布 | 组内首失败中止整组（硬伤） | 坏凭据主机 `redfish_up=2` + ignored 列表 | `redfish_up` bug 恒 0；KeyError→整次 scrape 400（硬伤） | 资源级 + BMC 级双重隔离（failed_resources + `redfish_up=0` + `redfish_scrape_error`）；但 AUDIT-1：浪潮轮超时后零序列静默丢失（无部分序列也无 up=0）；AUDIT-3：60s 超时全轮共享，Dell 7.7s 完成却被浪潮拖到 60020ms | 错误模型可区分 | ①— ②— ③P1 缺陷证实（实跑 4 轮同型） | **backlog**（AUDIT-1/3 修复：per-BMC 截止期 + 超时部分发布未列入 Task 7–18；分频（Task 7）仅缓解不根治，审计 §5 建议 per-BMC 并发或缩减请求面）；`redfish_up=2` 三值语义与 KeyError→400 **不采纳**（现有 up=0+error 计数语义清晰且 PromQL 兼容） |
| 会话认证与形状容错 | 每轮主动刷新会话，401/403/404→重登，持续失败转 basic 兜底 | 401→凭据轮换（Vault/脚本） | 每请求重建会话/重建树（硬伤）；转 basic 后 `redfish_up` 恒 0 | 401 自动重登 + 单次重试已有（scraper.rs:79-98）；AUDIT-2：启动期会话建立失败 fail-fast 拖垮全 exporter（exit 1）；AUDIT-4：浪潮 create-session 响应缺 `Name` 致 session 模式不可用；AUDIT-8：浪潮 `Sessions/1`→200 `{}` 非 404 | 无会话重登助手（已自建）；错误模型可判别 401 | ①部分 ②— ③P1 实跑证实（首跑退出码 1；浪潮回退 basic 后 4 轮超时） | **backlog**（AUDIT-2：启动失败不应拖垮全 exporter、AUDIT-4/8：浪潮会话非标准形状需容错，未列入 Task 7–18；idrac 每轮刷新+basic 兜底可作后续修复参照）；sapcc 每请求重建会话**不采纳**（复用会话 + 401 重登已覆盖） |
| 会话 shutdown 清理 | `/reset` 不删服务器端会话（泄漏，硬伤，我们不学） | 无 | 无 | 退出/重登后 BMC 端残留会话（D8） | `Session::delete` 现成（Location 回退 odata_id） | ①通过 ②通过 ③通过（A 组会话行为可测） | **实现**（Task 13：`EstablishedSession` 持有会话，stop 分支 DELETE；401 重登后的旧会话不删——记录说明） |
| 缓存架构 | 零缓存 | 零缓存（全量重爬） | 零缓存（重走树/重建会话） | 周期抓取 + 原子快照缓存（registry.rs：RwLock 按 BMC 隔离，/metrics 恒定可用） | ETag 缓存（CacheSettings，已用） | ①— ②— ③— | **不采纳**（对标零缓存为三项目共性硬伤（spec §1.4），快照架构为已领先项，保持现状）；对标零测试（idrac/sapcc）同样不学——我们以 nv mock 驱动的 e2e 测试（tests/）为准 |
| 厂商探测/端点存在性预检 | 客户端创建时 HEAD 探测，缺失端点静默跳过 | 部分抓取按组件发现 | 无 | 无预检，collector 失败计 failed_resources；真机两家 root/Systems/Chassis 全 200 零 404（审计 §1.1/1.2） | 无预检 API | ①— ②— ③收益未证实（真机零 404；且 AUDIT-8 显示依赖 404 探测存在性的逻辑在浪潮上失真） | **backlog**（无真机需求证据；若实施需绕开浪潮 404 语义失真陷阱） |
| 请求取消与优雅停机 | `context.Background()` 无取消传播（硬伤） | 无请求取消传播 | 无 | 轮级 `tokio::time::timeout` + watch 信号优雅停机（scraper.rs run；http.rs with_graceful_shutdown） | — | ①— ②— ③— | **不采纳**（对标做法（无取消）为硬伤；我们已具备取消传播与优雅停机，per-BMC 粒度提升已记入 AUDIT-3 backlog 行） |

## 维度四：运维

| 功能点 | idrac | fishymetrics | sapcc | 我们现状(含审计证据) | nv 能力 | 三关门禁 | 结论 |
|---|---|---|---|---|---|---|---|
| 自观测指标（build_info / scrape_errors_total / 响应时长 / 信息型指标） | `scrape_errors_total` | `/info` build info | `redfish_response_duration_seconds`；固件/版本 label 携带值恒 1 | 无 build_info/累计错误计数（D6；metrics.rs 中 SCRAPE_ERROR 注释过时，D10）；`redfish_info{key,value}` 已有（实跑 57 序列） | 自研（exporter 层） | ①自研 ②通过 ③— | **实现**（Task 12：`redfish_build_info{version}` + `redfish_scrape_errors_total{bmc}` 跨轮累计）；sapcc 响应时长指标 **backlog**（低优先，未列入任务）；信息型指标现状已等效（label 携带值恒 1） |
| 运维端点 /discover /reload /info | /reload（fsnotify 热加载+凭据变更 reset）、/discover（HTTP SD）、/reset、/health | /info、/verbosity 运行时调日志级别 | 无 | 仅 /metrics + /healthz（D7，http.rs） | 自研 | ①自研 ②通过 ③通过（审计"运维体验"无反对理由） | **实现**（Task 16：/info、/discover、/reload；**reload 部分实现**——配置对象热替换供 /discover 使用，BMC 增删/凭据变更需重启生效，记 backlog）；fishymetrics /verbosity **不采纳**（需运行时日志级别切换机制，收益低）；idrac /reset **不采纳**（会话泄漏反面教材，见会话清理行） |
| Grafana 面板 / Helm chart | 无 | 无 | 无 | 无 | — | ①— ②— ③— | **backlog**（未列入 Task 7–18，社区镜像/部署体验后续迭代） |
| 文档与验证 | — | — | — | docs/metrics.md 缺失新增指标族（D10 注释过时） | — | ①— ②— ③— | **实现**（Task 18：docs/metrics.md、README、config.example.yaml、docs/design.md 更新 + metrics.rs 注释清理 + 全量验证） |

## 维度五：安全

| 功能点 | idrac | fishymetrics | sapcc | 我们现状(含审计证据) | nv 能力 | 三关门禁 | 结论 |
|---|---|---|---|---|---|---|---|
| TLS 校验 | 无条件 `InsecureSkipVerify: true`（硬伤） | 正常校验（无证据反例） | TLS 全局禁用校验（硬伤） | per-BMC `insecure_skip_verify` opt-in + `ca_cert_file` CA bundle + rustls 无 OpenSSL + `#![forbid(unsafe_code)]`；SecretString 脱敏（config.rs） | 已具备 | ①— ②— ③— | **不采纳**（idrac/sapcc 无条件禁用校验为硬伤；我们 per-BMC opt-in 安全基线已领先，保持现状） |
| 凭据管理（Vault/凭据脚本/环境变量覆盖） | 无 | Vault/脚本凭据缓存按 401 失效重取 | 环境变量凭据覆盖 | 仅 config 文件（SecretString 内存脱敏）；无凭据轮换/外部注入 | — | ①— ②新依赖+大工程 ③— | **backlog**（spec §7 既定：新依赖+大工程；环境变量覆盖同线评估） |
| 代理支持 | 无 | HTTP(S)_PROXY/NO_PROXY + 每请求 proxy_host 覆盖 | 无 | reqwest 默认读环境变量代理，nv 的 client 构建是否覆盖需实测确认（spec §2.4） | 待真机确认 | ①待确认 ②— ③— | **backlog**（先真机确认 reqwest/nv 代理行为，再决定是否需要显式配置） |

---

## 结论汇总

**实现（Task 7–16、18）**：
- Task 7 分频采集（快/慢组）— 指标族/调度
- Task 8 事件日志、Task 9 BIOS、Task 10 内存纠错、Task 11 PSU/功耗统计 — 指标族（AUDIT-5 覆盖缺口 ①②③④）
- Task 12 build_info + scrape_errors_total — 运维
- Task 13 会话 shutdown 清理 — 可靠性（D8）
- Task 14 404 快速跳过（**部分**：仅 service-root 404；其余见 backlog）— 可靠性
- Task 15 分页（门控 MET，双 nextLink 键变体）— 可靠性（AUDIT-6，超越三项目）
- Task 16 /discover /reload /info（**部分**：reload 仅配置热替换）— 运维
- Task 18 文档与全量验证 — 运维

**评估后定（Task 15/16/17 门控裁决，引用本表对应行）**：
- Task 15 分页：J 组门控 **MET**（两家日志条目均 `Members@odata.nextLink`）→ 定为实现。
- Task 16 端点：运维维度无反对理由 → 定为实现（reload 部分实现记 backlog）。
- Task 17 驱动器 OEM：L 组门控 **NOT MET**（Dell 标准字段 null、浪潮无盘）→ 转 backlog。
- Task 18 收尾：随实现任务推进执行。

**backlog（含理由）**：
- Task 17 驱动器 OEM（真机无可用寿命数据）；
- AUDIT-1/3 修复：per-BMC 截止期 + 超时部分发布（零序列静默丢失与全轮共享超时，P1，未列入本期任务）；
- AUDIT-2/4/8 修复：启动 fail-fast 容错与浪潮会话形状兼容（P1）；
- Task 14 其余部分：collector 级 404 判别（错误 String 化丢状态码，成本大于收益，真机零 404）；
- Task 16 reload 的 BMC 热生效（配置可替换，BMC 变更需重启）；
- 指标族：其他 idrac 缺口（CPU 电压/频率、控制器明细/RAID 电池、系统 LED）、厂商矩阵细节（遇真机型号再评估）、PDU、SSE 事件推送（nv `event-service` 现成，真机无场景）；
- 调度：共享预取（与 collector 隔离性冲突）、部分抓取替代方案（见不采纳）；
- 可靠性：厂商探测预检（真机零 404，收益未证实）、sapcc 响应时长指标（低优先）；
- 安全：Vault/凭据脚本/环境变量覆盖、代理确认（spec §2.4 待测）；
- 运维：Grafana 面板、Helm chart。

**不采纳（含理由）**：
- 健康值数值映射（D11 — 破坏现有 PromQL，label 方案保留）；
- fishymetrics 部分抓取 `?components=`（与周期快照架构冲突，目标已由 Task 7 达成）；
- fishymetrics 100ms 强制睡眠、每请求重建会话（sapcc）、组内首失败中止整组（idrac）—— 我们以 ETag 缓存、会话复用、per-resource 隔离为更优做法；
- 并发信号量（D5 复核后无场景）、采集去重（无 on-demand 并发场景）；
- `redfish_up=2` 三值语义、KeyError→整次 400（sapcc）、无条件 insecure TLS（idrac/sapcc）、`/reset` 会话泄漏（idrac）、零缓存/零测试架构（spec §1.4 共性硬伤）、无取消传播（idrac context.Background）—— 我们均已领先，保持现状。
