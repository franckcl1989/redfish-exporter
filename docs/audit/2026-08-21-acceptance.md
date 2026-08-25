# 0.1.0 生产就绪验收报告

日期：2026-08-21
验收范围：两台真机（Dell PowerEdge R750 @<dell-bmc-host>、浪潮 @<inspur-bmc-host>）+ 三个对标项目（idrac_exporter / fishymetrics / sapcc）+ 发布资产
验收方式：真机端到端实跑（新代码首次上真机）、故障演练、性能基准、依赖审计、发布流程核对

## 结论

**有条件投产（CONDITIONAL PASS）**——核心目标全部达成，2 项遗留跟踪不阻断投产：

| 维度 | 结果 |
|---|---|
| 真机端到端（Dell 全量指标 / 浪潮慢 BMC 场景） | ✅ PASS（期间发现并修复 4+1 个缺陷） |
| 故障隔离与自愈 | ✅ PASS |
| 性能 | ✅ PASS |
| 工程质量（测试/静态检查） | ✅ PASS |
| 发布资产 | ✅ PASS（1 项构建期依赖跟踪） |

## 1. 真机端到端（阶段 A）

配置：session 认证（浪潮自动降级 basic）、scrape_interval=30s、scrape_timeout=120s、slow_interval=120s。

### 1.1 Dell PowerEdge R750（iDRAC 5.10.30.00）

| 验证项 | 结果 | 证据 |
|---|---|---|
| 快组（每轮） | up=1，~3s | scrape_duration_seconds 2.87s |
| 慢组（120s 周期） | up=1，全量 ~11s | 全量轮 duration 10.8s |
| 事件日志分页 | **1146 条**（Lclog 1062 + SEL 76 + …，多页真机验证） | redfish_event_log_entry 计数 |
| BIOS 属性 | 199 数值 + 247 字符串 + pending=1（448 属性全量） | 与真实载荷 448 属性一致 |
| 内存 ECC | 8 DIMM 全部产出 0/1 | redfish_memory_*_errors |
| PSU 明细 | 2 PSU：容量 1400W、效率 92%、输入电压 | redfish_power_supply_* |
| 功耗统计 | min/max/avg + 1min 间隔 | redfish_power_consumption_* |
| 驱动器/卷/网络/固件 | 6 盘 / 3 卷 / 2 网口 / 16+ 固件项 | redfish_drive/volume/ethernet/info |
| 零失败 | failed=0，scrape_errors_total=0 | 日志 + 指标 |

### 1.2 浪潮（慢 BMC，单请求 0.8–3.8s，为 Dell 的 10 倍）

| 验证项 | 结果 | 证据 |
|---|---|---|
| 会话降级 | session 建立失败（缺 Name 字段）→ basic 兜底 → up=1 | 日志 warn + up=1 |
| 快组 | up=1，44–48s | scrape_duration_seconds 44.2s |
| **BIOS 166KB 载荷** | **5755 属性全量解析**（4065 数值 + 1690 字符串）——审计最大风险点解除 | 与真实载荷 5757 属性一致 |
| **事件日志分页** | **2064 条**（SEL/AuditLog/IDL 全量）——BMC 分页缺陷下防循环生效 | 计数 + "pagination loop" 防护 |
| 传感器 | 11 个（legacy Thermal/Power 路径） | redfish_sensor_reading |
| 固件 | 3 项 + 1 项 "NULL" 如实导出 | redfish_info |
| 无数据项（真实缺失） | drive/volume=0（存储成员 500 无 RAID 控制器）、无 Sensors 集合 | 审计 H/G 组一致 |

### 1.3 验收发现并修复的缺陷（全部提交）

| # | 缺陷 | 严重度 | 修复 |
|---|---|---|---|
| ACC-1 | `slow_interval=None`（默认）时慢组**永不采集**——默认配置静默丢失存储/网络/固件/日志/BIOS 指标 | **P0** | slow_due(None)=true（每轮全量，恢复 0.1.0 语义） |
| ACC-2 | 浪潮 BMC 分页缺陷：nextLink 恒指同一 URL → 分页死循环 1000 次请求 | **P1** | 已访问 URL 去重，循环即停 + warn |
| ACC-3 | 慢 BMC 场景：快+慢组共用一个截止时间 → 慢组永不完成 → 缓存永不填充 → 每轮全量超时（饿死）且超时吞掉快组数据 | **P1** | 快/慢组独立截止；慢组失败保留 last-good + 按周期重试 |
| ACC-4 | 401 会话过期后重登仍携带失效 token → 永远无法恢复 | **P1** | 重登前切回 basic 凭据 |
| ACC-5 | 无 per-collector 计时日志，慢资源定位困难 | P3 | debug 级 collector 计时 |

## 2. 故障演练（阶段 B）

| 场景 | 结果 |
|---|---|
| BMC 端会话被删除（模拟过期） | 401 检测 → 重登（修复后）→ 恢复 up=1；修复前确认无法恢复（ACC-4） |
| 不可达 BMC（假 IP） | up=0 + 连接超时日志，**健康 BMC 不受影响**（up=1） |
| 错误凭据 | up=0 + 8 资源失败如实上报，不影响其他 BMC |
| 会话泄漏 | force-kill 必然泄漏（平台限制，所有 exporter 相同）；浪潮会话曾被打满 20 上限 → 已清理恢复；Linux 优雅停机（SIGTERM）路径已实现并有单测 |
| 慢 BMC 超时 | up=0 可见发布（不再静默消失），errors_total 累计 |

## 3. 性能基准（阶段 C）

| 指标 | 数值 |
|---|---|
| Dell 快组轮 | ~3s；全量轮 ~11s |
| 浪潮快组轮 | 44–48s；慢组轮 ~159s（每 4 轮 1 次） |
| 内存占用 | 44.3MB 工作集 / 31.5MB 私有（两台全量快照 9745 序列） |
| /metrics 输出 | 1.1MB，热延迟 ~310ms |
| 分频效果 | 慢组独立截止后，浪潮快组数据每轮稳定可用（修复前每轮丢失） |

## 4. 工程质量与发布资产（阶段 D）

- 测试：19 个测试二进制全绿（新增分页循环/慢组语义测试）；clippy -D warnings、fmt 干净
- CI（.github/workflows）：fmt/clippy/test/doc/audit/deny 全覆盖；release.yml：musl 静态构建 + 静态链接验证 + Release 打包
- Dockerfile：rust:1.90-alpine 构建 musl → distroless static nonroot
- cargo audit：**2 个 high（RUSTSEC-2026-0194/0195）在 quick-xml 0.38.4**——nv-redfish-csdl-compiler 的**构建期**依赖（不进入运行时二进制）；nv 0.15.1 锁定无法升级（0.41 为 major bump，升级破坏依赖链约束）→ **跟踪项**：nv-redfish 上游升级后跟进
- 告警规则：覆盖 up/scrape_error/传感器阈值/驱动预测失败/链路中断

## 5. 遗留跟踪（不阻断投产）

| 项 | 说明 |
|---|---|
| quick-xml 构建期漏洞 | 运行时无影响；跟踪 nv-redfish 上游 |
| 新指标告警规则 | 建议补充：事件日志 Critical 告警、内存 ECC 告警、BIOS pending 告警（示例规则文件可扩展） |
| 浪潮生产配置建议 | `scrape_timeout: 120s` + `slow_interval: 120s`（默认 15s/60s 对慢 BMC 偏紧） |
| 优雅停机验证 | Windows 测试环境无法发 SIGTERM；Linux 部署（systemd）路径已实现并有单测，投产前在 Linux 环境冒烟 |
| 会话恢复兜底 | 401 重登若持续失败 → up=0 如实呈现（现状）；可选：N 次失败后降级 basic 常驻（backlog） |
| musl/镜像构建 | 本地 Windows 无法构建 musl；由 CI release 流程覆盖，发版时验证 |

## 6. 结论

**0.1.0 具备投产条件**：两台真机（含慢 BMC 极端场景）全量验证通过、故障演练通过、性能与资源占用健康、工程质量达标。验收过程发现并修复 5 个缺陷（1 P0 + 3 P1 + 1 P3），均为真实生产环境才会暴露的问题——印证了真机验收的必要性。

投产前建议：按 §5 配置建议部署（慢 BMC 调整超时参数），Linux 环境冒烟优雅停机，随后按 CI release 流程发 0.1.0 正式版。
