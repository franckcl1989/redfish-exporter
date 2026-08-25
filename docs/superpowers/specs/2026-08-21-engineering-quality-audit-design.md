# 工程质量审计 — 设计文档（深度修订版 v2）

日期：2026-08-21
状态：v2 已确认（设计要点在对话中通过）
目标：以三个社区常用的开源 Redfish exporter 为对标，在真实硬件（Dell R750 + 浪潮）上找到并修复我们的缺陷、吸纳对方的优点 —— 前提是不破坏 nv-redfish 核心依赖链，且 nv 已经提供的能力不做重复自研。

## 1. 对标项目深度研究结论（源码级）

### 1.1 mrlhansen/idrac_exporter（Go，16 个源文件）

**值得吸纳的设计**
- **每主机加权信号量**（默认 10 并发）+ `MaxIdleConnsPerHost` 绑定：即使多指标组并行采集，BMC 并发连接数有硬上限（redfish.go:80-87）
- **每轮采集前主动刷新会话**：GET session 资源，401/403/404 → 重登录；持续失败 → basic 认证兜底（redfish.go:201-243, 92-97）
- **采集去重**：同 target 并发请求经 sync.Cond 合并为一次 BMC 爬取，后者直接复用前者结果（collector.go:716-736）
- **运维端点**：`/reload`（fsnotify 配置热加载，凭据变更自动 reset 目标）、`/discover`（Prometheus HTTP SD）、`/reset`、`/health`
- **`scrape_errors_total`** 累计错误计数器（collector.go:712）
- **厂商探测**：客户端创建时 HEAD 探测各端点存在性，缺失端点静默跳过不报错（client.go:90-265）
- **新旧 schema 双路径**：legacy Power/Thermal vs 新 PowerSubsystem/ThermalSubsystem，优先 legacy（client.go:267-386, 543-740）
- **健康值 0/1/2 映射**（OK/Warning/Critical）+ 状态字符串保留在 label 中（metrics.go:18-30）

**硬伤（我们不学的）**
- 全库无 `@odata.nextLink` 分页；TLS 无条件 `InsecureSkipVerify: true`；`/reset` 不删服务器端会话（泄漏）；`context.Background()` 无取消传播；`collectors` 只按 target 建键忽略 auth 参数；组内首个失败即中止整组；**零测试文件**。

### 1.2 comcast/fishymetrics（Go v0.19.2）

**值得吸纳的设计**
- **部分抓取 `?components=thermal,power`**：按组件门控发现阶段与任务构建阶段，热采集只打 4-5 个请求 vs 全量 60-100+；配合 Prometheus 多 job 不同 `scrape_interval` 实现**不同指标不同周期**（thermal 1m + full 5m）—— 即用户记忆中的设计（partial_exporter.go:66-410, docs/partial-scraping.md:71-135）
- **404 重试循环**：非 2xx 的 404 重试最多 3 次×2s，可用 `--disable-404-retry` 关闭（BMC 固件升级/启动瞬时 404 场景）（common/util.go:55-73）
- **401 → 凭据轮换**：Vault/脚本凭据缓存按 401 失效重取（common/util.go:74-108）
- **坏凭据主机隔离**：`redfish_up=2` + ignored 列表 + 测试连接/移除 UI（exporter.go:513-521）
- **厂商形状容错**：自定义 UnmarshalJSON 处理单数/复数、`Links`/`links` 大小写、string/int/bool 多态（oem/chassis.go:70-96, oem/power.go:51-89, oem/drive.go:97-200）
- **可观测性**：`/info` build info、每请求 trace id、路由指标、运行时可调日志级别（GET/PUT /verbosity）
- **代理支持**：HTTP(S)_PROXY/NO_PROXY + 每请求 `proxy_host` 覆盖

**硬伤（我们不学的）**
- 全库零缓存：每个请求重建 exporter、全量重爬；串行采集（pool=1）+ 每请求强制 100ms 睡眠 → 全量 scrape 60-100+ 请求 ≈ 6-10s 纯睡眠；`BMC_TIMEOUT` 配置是死配置（硬编码 30s）；重试逻辑三处重复且不一致；`common.ExtraParamsAliases` 全局可变存在数据竞争；无分页；无请求取消传播。

### 1.3 sapcc/redfish-exporter（Python/Falcon）

**值得吸纳的设计**
- **按指标族拆 5 端点**（/health /firmware /performance /sensors /bios）：Prometheus 可对每族配独立 scrape_interval，实现分频采集（main.py:38-43, collector.py:479-529）
- **内存可纠错/不可纠错错误计数**：DIMM `Metrics` → `HealthData.AlarmTrips.Correctable/UncorrectableECCError`（health_collector.py:461-503）
- **BIOS 属性导出**：`Systems/{id}/Bios` → 每属性一个指标 + `@Redfish.Settings` 存在即 pending（bios_collector.py:87-109）
- **厂商矩阵细节**（每个都是真机 bug 的编码）：
  - Dell：`Manufacturer ^Dell` 时用 SKU 字段作 serial（collector.py:369-372）
  - HPE iLO6：per-PSU 活功率恒 0 → "静默 PSU"规则丢弃全零读数；`Status.State==Absent` 故意不过滤（iLO6 全标 Absent）（performance_collector.py:194-250）
  - Fujitsu：字符串字段去尾空白（health_collector.py:31-40）
  - 固件清单仅取 URL 含 `Installed` 的条目（过滤 staged/pending）（firmware_collector.py:48）
- **`redfish_response_duration_seconds`**：首个未认证 GET /redfish/v1 的耗时
- **信息型指标**：固件/版本用 label 携带值、值恒 1，控制基数

**硬伤（我们不学的）**
- 全库零缓存（每个请求重走树、重建会话、重新发现）；**`redfish_up` bug**：会话建立失败转 basic 兜底后 `redfish_up` 恒 0 且非 health 端点输出为空；无分页；异常（KeyError）→ 整次 scrape 400；TLS 全局禁用校验；线程无上限；BIOS 路径重复抓取 Systems 集合。

### 1.4 三项目共有的硬伤（我们已领先）
- 全部零缓存架构（我们是周期快照缓存）；全部无分页；idrac/sapcc 零测试（fishymetrics 有 24 个 handler 单测 + Vault docker 集成测试）。

## 2. nv-redfish 0.15.1 能力边界（决定我们能否"直接用 nv"）

来源：本地 crate 源码（index.crates.io-1949cf8c6b5b557f\nv-redfish-*0.15.1）。

### 2.1 已有但未启用的特性（启用纯增量、零破坏 —— 只增加生成类型）
| 特性 | 提供能力 | 用途 |
|---|---|---|
| `log-services` | `LogService::raw/entries/filter_entries`，LogEntry/Event 类型；入口 `Manager::log_services()` / `ComputerSystem::log_services()` | 事件日志指标（对标 idrac `events_log_entry`） |
| `bios` | `Bios::raw/attribute`，属性值 EdmPrimitiveType，`ComputerSystem::bios()` | BIOS 属性指标 + `@Redfish.Settings` pending（对标 sapcc） |
| `event-service` | `EventService::events()` SSE 流（未来可做事件推送） | backlog |
| `task-service` / `power-equipment` | 异步任务、PDU 类型 | backlog（PDU 可缓做） |

### 2.2 nv 明确没有的能力（需自研或放弃）
- **`@odata.nextLink` 分页：全库不支持**（facade/core 零命中）。三个对标项目也都没有。若要支持，需用 raw fetch 模式自研：定义 `EntityTypeRef` wrapper + `serde_json::Value`，约 10 行（核心先例 patch_support/payload.rs:160-190）。
- 会话过期检查/自动重登助手：无（我们已自建 401 重登流程，bmc.rs/scraper.rs）。
- 按请求错误重试：无（bmc-http 有 opt-in `RetryPolicy{classifier, max_retries, delay}`，固定延迟无退避，传输错误不重试，仅收到响应后判定）。
- 日志/指标埋点：无（crate 内零 tracing 调用）。

### 2.3 nv 现成可用的
- 错误模型：非 2xx 统一 `BmcError::InvalidResponse{status,..}` → 404/401/5xx 可精确匹配；超时是 `ReqwestError::is_timeout()`。
- `ConcurrencyLimitedBmc`（bmc-http `http-extras` 特性）：per-BMC 并发上限（对标 idrac 信号量）。
- 会话：`SessionCollection::create_session`、`Session::delete`（Location 回退 odata_id）、`Session.expiration_time`。
- `$expand`（`NvBmc::expand_property`）带内置厂商 quirk 处理（Dell/AMI/HPE/…）；`$filter` 可用。
- 传感器模型：`Sensor.reading/reading_units(UCUM 字符串)/thresholds(6 阈值+user 变体)/ranges`；`EnvironmentMetrics` 内联读数 + `sensor_links()`。
- OEM 扩展：`oem` 特性下 `oem_value/oem_object` 读任意 `serde_json::Value`（Dell/浪潮 OEM 字段采集通道）。
- BmcMock：期望队列，可注入任意 JSON 形状与错误；**局限：不能产生真实 HTTP 状态码**（404 路径测试需 wiremock + HttpBmc，bmc-http 自身测试即此法 reqwest.rs:1375-1496）。
- ETag 缓存（CacheSettings，默认 100 条，304 走缓存）—— 已在用。

### 2.4 待真机确认项
- 代理：reqwest 默认读环境变量代理，但 nv 的 reqwest::Client 构建是否覆盖需实测确认。

## 3. 我们现状评估

### 3.1 优势（相对三对标项目）
- 周期抓取 + 原子快照缓存（`registry.rs`），BMC 负载可控且 /metrics 恒定可用 —— 三项目均为 on-demand 零缓存
- 资源级 + BMC 级双重故障隔离（failed_resources 不影响其他资源）
- 会话 401 自动重登 + 单次重试（scraper.rs:79-98）
- 安全：SecretString 脱敏、rustls 无 OpenSSL、`#![forbid(unsafe_code)]`、CA bundle
- 测试：nv mock 驱动的 e2e 测试（tests/）—— 对标项目中只有 fishymetrics 有少量单测

### 3.2 缺陷清单（草案，待真机验证补充）
| # | 缺陷 | 严重度预判 | 对标来源 |
|---|---|---|---|
| D1 | 每轮所有 collector 全量采集，无分频能力 | P1（性能/负载） | fishymetrics/sapcc |
| D2 | 每 collector 重复遍历 collection（N×walk，design.md 已承认） | P2 | — |
| D3 | 无 nextLink 分页，大集合截断 | P2（大 SEL/多盘场景） | 三项目也无，属超越 |
| D4 | 无 404 快速跳过/重试策略，BMC 瞬时 404 直接计入失败 | P2 | fishymetrics |
| D5 | ~~无 per-BMC 并发上限：JoinSet 7+ collector 并行打同一 BMC~~ **已复核修正**：`collect_all` 内各 collector 是顺序 await 的（collector/mod.rs:80-123），单 BMC 请求并发恒为 1，不存在冲击 BMC 问题；真正的负载问题是每轮请求总数高（同 D2）。跨 BMC 并行（JoinSet）是刻意设计，保留 | 已消除 | — |
| D6 | 无 build_info / scrape_errors_total 自观测 | P2 | idrac/fishymetrics |
| D7 | 无 /discover、/reload、/info 端点 | P3 | idrac/fishymetrics |
| D8 | 会话不清理：退出/重登后 BMC 端残留会话 | P2（安全/资源） | — |
| D9 | 指标族缺口：事件日志、BIOS、内存纠错计数、PSU 效率/输入电压、功耗 min/max/avg、驱动器寿命、系统 LED、CPU 电压/当前频率、控制器明细 | P1-P2 | 三项目 |
| D10 | metrics.rs 中 SCRAPE_ERROR 等常量注释声称未实现但 registry.rs 实际使用（注释过时） | P3（维护性） | — |
| D11 | 健康值语义：我们用 label（health/state），对标项目用数值映射，PromQL 兼容性需评估 | P3 | idrac/sapcc |

## 4. 真机探测矩阵（详细版）

两台 BMC：`https://<dell-bmc-host>/`（Dell R750, <user>）+ `https://<inspur-bmc-host>/`（浪潮, admin）。
工具：Python 3 标准库脚本（urllib+ssl+json），basic 认证，只读。快照存 `Temp\opencode\redfish-audit\<bmc-name>\`（gitignore，凭据不落盘不入库）。
每项记录：HTTP 状态码、耗时 ms、关键头（OData-Version/ETag/Content-Type）、字段存在性、值、`@odata.nextLink` 出现与否、`Oem.*` 键清单。

| 组 | 探测项 | 目的 |
|---|---|---|
| A. 认证与会话 | 匿名/basic GET /redfish/v1；SessionService GET + 错误凭据行为；POST session（状态码？X-Auth-Token？Location？session_timeout 值）；token 过期行为（401?）；会话数上限；**basic vs session 双模式可用性**（浪潮已知风险） | 认证兼容性、会话生命周期 |
| B. Systems | 身份字段（Manufacturer/Model/SerialNumber/SKU/PartNumber/BIOSVersion/HostName）；PowerState；ProcessorSummary/MemorySummary；IndicatorLED；Links(Chassis/ManagedBy)；子资源存在性：**Bios(200? 404?)**、**Memory/{id}/Metrics(AlarmTrips)**、**Processors/{id}/Metrics**、EthernetInterfaces | 指标可采性 + Dell SKU 当 serial 规则验证 |
| C. Chassis | 身份+类型；**legacy Power/Thermal vs 新 PowerSubsystem/ThermalSubsystem 存在性矩阵**；Sensors 集合（成员数、类型分布 Temperature/Voltage/Power/Fan、Reading 缺失处理、阈值存在性）；EnvironmentMetrics 字段；Controls；PowerSupplies/{id}/Metrics；NetworkAdapters/PCIeDevices/Drives；LED | 双路径兼容、传感器覆盖、环境指标 |
| D. Managers | Model/FirmwareVersion/Status/DateTime；EthernetInterfaces(IP)；**LogServices：SEL 条目数、severity 值分布、Created 格式、members 分页**；ManagerType | 事件日志可采性 |
| E. UpdateService | FirmwareInventory：条目数、Name/Version/Updateable/Status、URL 是否含 Installed | 固件清单（sapcc 过滤规则验证） |
| F. 电源详情 | legacy Power：PowerControl[]（PowerConsumedWatts/PowerCapacityWatts/**PowerMetrics{Min/Max/AverageConsumedWatts,IntervalInMin}**）、PowerSupplies[]（Model/Status/PowerInputWatts/PowerOutputWatts/**EfficiencyPercent**/InputVoltage/CapacityWatts）；新路径 PowerSupplies/{id}/Metrics；EnvironmentMetrics.PowerWatts/EnergykWh | PSU 明细、功耗 min/max/avg、能量计数 |
| G. 传感器 | Sensors 集合类型分布；Reading+ReadingUnits；ReadingRangeMin/Max；Thresholds（UpperCritical/UpperWarning/Lower*）存在性与非空；null Reading 处理；legacy Fan/Temperature 双路径 | 传感器指标 + 阈值告警完整性 |
| H. 存储 | StorageControllers（Model/FirmwareVersion/Status/SerialNumber）；Drives（CapacityBytes/MediaType/Protocol/FailurePredicted/Revision/**Oem 寿命字段——Dell/浪潮**）；Volumes（CapacityBytes/RAIDType/Status）；磁盘数量 | 存储指标 + OEM 差异 |
| I. 网络 | EthernetInterfaces（LinkStatus/SpeedMbps/MAC/IP）；NetworkAdapters（Model/Serial）；**NetworkPorts vs Ports 路径**（HPE 教训）；PCIeDevices 数量类型 | 网络覆盖、路径差异 |
| J. 分页 | 所有集合响应扫描 nextLink；SEL/传感器/磁盘/固件成员数 vs 单页上限；`$top/$skip` 支持性探测 | 分页缺陷实证 |
| K. 延迟基准 | 每请求耗时分布；一次"模拟完整采集轮"（按我们的 collector 请求序列）总耗时与请求数 | 性能基准、慢组/快组分频依据 |
| L. OEM 差异 | Dell：Oem.Dell 键清单、iDRAC 特有路径形状；浪潮：**Oem.Public**（idrac_exporter 源码记录浪潮 power+drive life 在 Oem.Public，client.go:667-669,834-837） | 厂商特有指标通道 |

**实跑 exporter**（探测后进行）：本地 `cargo run`，两台 BMC session 认证（浪潮失败则 basic 并记录为缺陷），3+ 轮 `/metrics` + info/debug 日志；记录每 BMC 指标族数量、failed_resources、scrape 时长；与探测基线交叉对比（API 存在但无指标=覆盖缺陷；值异常=解析缺陷；请求失败=可靠性缺陷）。

## 5. 分频采集设计（新架构能力，D1 修复）

对标 fishymetrics partial + sapcc 端点，保留快照架构，做**快/慢两组**：

```
config:
  scrape_interval: 30s        # 快组周期（现有语义）
  slow_interval: 300s          # 慢组周期（新，= N×快组）
```

- **快组（每轮）**：`sensors`、`power`、`processors`、`memory`、`systems`（collect_systems）、`chassis_health`、`managers`（up/健康/信息）
- **慢组（每 slow_interval）**：`storage`（驱动/卷明细）、`network`（适配器/端口明细）、`firmware`（固件清单）、`assembly`、新增 `event_log`、新增 `bios`
- 实现：scraper.rs 中每个 BMC 记录 `last_slow_scrape`，每轮先采快组，慢组到期才采；`ScrapeReport` 增加来源标记或直接合并两组结果进同一 registry 原子更新（快照一致性不破坏）
- 慢组期间 `/metrics` 继续提供旧快照（快组新数据 + 慢组旧数据共存于一次轮换中 → 需保证合并时序：慢组轮先采慢组再采快组，一次性 build_registry 发布）
- 此设计不动 collector 接口与 nv 依赖链，纯编排层改动

## 6. 差距分析维度（产出物结构）

`docs/audit/2026-08-21-gap-analysis.md`，行 = 功能点，列 = idrac/fishymetrics/sapcc 现状、我们现状（含真机证据）、nv 能力映射、三关门禁结论、建议（实现/backlog/不采纳+理由）。维度：

1. **指标族**：事件日志（idrac）、内存纠错计数（sapcc）、BIOS 属性+pending（sapcc）、PSU 效率/输入电压（idrac）、功耗 min/max/avg+间隔（idrac）、驱动器寿命/指示灯（idrac+浪潮 OEM）、CPU 电压/当前频率（idrac）、控制器明细/RAID 电池（idrac）、系统 LED（idrac）、PDU（idrac，backlog）
2. **调度**：分频采集（fishymetrics/sapcc）、部分抓取（fishymetrics，评估后定）、共享预取（backlog 评估）
3. **可靠性**：404 策略（fishymetrics）、分页（自研，超越三项目）、并发上限（idrac）、会话清理、响应时长指标（sapcc）
4. **运维**：build_info+scrape_errors_total（idrac）、/discover（idrac）、/reload（idrac）、/info（fishymetrics）、Grafana 面板、Helm chart
5. **安全**：环境变量凭据覆盖（sapcc）、Vault/凭据脚本（fishymetrics，backlog）、代理确认、会话泄漏治理（idrac 反面教材）

## 7. 改进项候选与三关门禁预判

| 候选 | nv 门禁 | 依赖链门禁 | 收益门禁（真机证据待确认） | 预判 |
|---|---|---|---|---|
| 分频采集（快/慢组） | 纯编排层，不涉 nv | 通过 | 探测延迟基准后确认 | 实现 |
| 事件日志 | `log-services` 特性现成 | 通过（增量特性） | SEL 存在性探测 D 组 | 实现 |
| BIOS 属性 | `bios` 特性现成 | 通过 | Bios 200/404 探测 B 组 | 实现 |
| 内存纠错计数 | MemoryMetrics.health_data.alarm_trips 现成（bool） | 通过 | Metrics 存在性探测 B 组 | 实现 |
| PSU 明细/功耗 min-max-avg | Power/PowerSupply/PowerMetrics 类型现成 | 通过 | F 组探测 | 实现 |
| ~~并发上限~~ | ~~ConcurrencyLimitedBmc~~ | — | — | **已取消**（collector 顺序执行，无并发冲击；见 D5 修正） |
| build_info/scrape_errors_total | 自研（exporter 层） | 通过 | — | 实现 |
| 会话 shutdown 清理 | `Session::delete` 现成 | 通过 | 会话行为探测 A 组 | 实现 |
| 404 快速跳过+计数 | 错误模型可区分 | 通过 | J/K 组实测 404 发生率 | 实现（轻量版：跳过+计数；重试不学 fishymetrics 的 3×2s） |
| 分页 | raw fetch 自研 ~10 行 | 通过（不动 nv） | J 组实测是否有分页响应 | 评估后定 |
| /discover、/reload、/info | 自研 | 通过 | — | 评估后定 |
| 驱动器寿命/指示灯 | Drive 类型 + oem_value 读 OEM | 通过 | H/L 组 OEM 探测 | 评估后定 |
| 共享预取（消除 N×walk） | 与 collector 隔离性冲突 | 有重构风险 | 请求数基准 K 组 | backlog |
| Vault/凭据脚本 | 新依赖+大工程 | 有新增依赖 | — | backlog |
| PDU、SSE 事件推送 | power-equipment/event-service | 通过 | 真机无 PDU | backlog |

## 8. 交付物与验收

1. `docs/audit/2026-08-21-bmc-audit.md`：两台 BMC 探测基线（矩阵 A-L 全量结果）+ exporter 实跑对比 + P0-P3 缺陷清单（附脱敏证据）
2. `docs/audit/2026-08-21-gap-analysis.md`：差距表（每行三关门禁结论）
3. 本设计文档（已提交）
4. 实施计划（writing-plans 流程产出）

验收标准：
- 每缺陷有原始证据（请求/响应/日志，凭据脱敏）；P0/P1 在真机或 mock 可复现
- 差距表每行有明确三关门禁结论
- 结论基于真机数据，不虚构

安全约束（贯穿）：只读 API 操作；凭据仅在 gitignore 临时目录；日志脱敏；不提交任何真实凭据。
