# 工程质量专项 — 设计文档（v1）

日期：2026-08-24
状态：v1 已确认（设计要点在对话中通过）
目标：0.1.0 发布与投产前五维专项之后的收官项——**全库工程审计 + 两件收尾合并 + 发布资产补齐**。使整个项目实现优雅、哲学统一、语义一致、逻辑自洽、真实有效、完整可靠，遵循项目定义的目标/规范/设计原则，达到生产就绪标准。
约束：本专项为收官项——不得破坏前五维（安全性/稳定性/性能/资源/功能）成果；**不改冻结语义**（五维冻结清单：指标命名/help/labels、配置语义、状态机行为、发布语义、/metrics 输出字节等价），触达冻结清单的修复须用户批准。

## 1. 已确认决策记录（对话中逐一确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | CI 验证方式 | gh（已登录 franckcl1989）创建**公开**新仓库并关联 remote、推送全部历史；GitHub Actions 跑 SIGTERM 冒烟 job；Linux 验证机 <linux-verify-host>（<user>）手动佐证 |
| D2 | 审计范围 | 全维度工程审计：代码门禁扩展 + 关键路径复查、文档一致性、语义/命名一致性、哲学统一、发布资产 |
| D3 | 许可证 | Apache-2.0 |
| D4 | soak 执行 | 本地 mock 跑满 ≥2h（240+ 轮）；真机不做长 soak（BMC 保护：2h×30s 轮 ≈ 24k 请求/BMC 冲击大） |
| D5 | 审计发现处置 | 分级 P0（投产阻断必修）/P1（必修）/P2（修或 backlog）/P3（backlog）；触达冻结语义的修复须用户批准；无问题的审计面如实记「无发现」，不凑数 |

## 2. 现状盘点（探索结论）

| 面 | 现状 | 结论 |
|---|---|---|
| git remote | 无（从未推送） | gh 建公开新仓库 + 关联推送（D1） |
| CI | ci.yml 含 SIGTERM 冒烟 job（64 行）未真跑过；release.yml 存在 | 推送后 Actions 真跑 + Linux 机佐证（S2） |
| 许可证 | 无 LICENSE | 补 Apache-2.0（D3） |
| 变更记录 | 无 CHANGELOG | 补 0.1.0 完整变更（S4） |
| 发布资产 | Dockerfile、deploy/（Grafana 面板、K8s manifests、Prometheus 告警）、.dockerignore、rust-toolchain.toml 齐全 | Docker 构建验证在 Linux 机（本机无 docker）；release.yml 审查 |
| soak | 基建已就绪（tests/soak_test.rs，SOAK_SECS 参数化，30s 短跑已验证）；≥2h 未跑 | 本维跑满 ≥2h 并回填稳定性验收（D4） |
| 审计基线 | 五维全部验收记录 + 冻结清单齐备；全部门禁绿 | 按 §3 审计面逐一核查 |

## 3. 审计方法

### 3.1 审计面

1. **代码面**：全量门禁（`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets`、`cargo doc --no-deps`、`cargo audit`、`cargo deny check`）全绿；关键路径复查——状态机（stability.rs）、锁序（registry.rs scratch/inner）、错误模型（Bmc 级/资源级隔离）、发布语义（scraper 三处发布路径）、编码等价（预编码缓存）、OEM 读取路径（oem_value/raw JSON 导航）、优雅停机路径。
2. **文档一致性**：README / docs/design.md / docs/metrics.md / docs/security.md / config.example.yaml 与代码、冻结清单逐项核对（指标族 26 个全列、配置节全列、行为描述与实现一致、示例文件可直接运行）。
3. **语义/命名一致性**：指标命名规则（`redfish_` 前缀 + 单位后缀 + 状态类 label 模式）跨六维自洽；label 惯例（bmc/system/storage/id/resource_type/state）统一；配置语义（web/stability/bmc 节）与文档一致；错误语义（up=0/failed_resources/cooldown/SessionDegraded）跨记录一致。
4. **哲学统一**：社区资源优先（盘点手写实现 vs 社区 crate 覆盖；无手写编码器/存储替换——资源维 D1 既定）；YAGNI（无死代码、无未用依赖、无过度抽象）；诚实记录惯例（每验收数字有制品引用）；中文代码注释惯例；重复代码盘点（expect_full_round 复制等既有裁决项复核）。
5. **发布资产**：LICENSE（Apache-2.0 全文）、CHANGELOG（0.1.0 变更）、README 完整性（安装/配置/运行/指标清单/告警示例）、Dockerfile 构建验证（Linux 机）、release.yml 审查（标签触发/资产上传/签名）、.dockerignore/.gitignore 合理性。

### 3.2 发现分级与处置

- P0：投产阻断（漏洞、崩溃、数据错误、发布阻断）——必修，修复经审查闭环。
- P1：必修（文档与代码矛盾、语义不一致、资产缺失）。
- P2：修或 backlog（按成本/收益裁决）。
- P3：backlog（润色、优化类）。
- 触达五维冻结清单的修复：**报用户批准后才实施**。
- 无问题的审计面：如实记录「无发现」，不凑数。

### 3.3 执行方式

- 子代理按审计面分批（每批一至两个面），产出发现清单（文件:行 + 分级 + 修复建议）→ 汇总去重 → 修复波（子代理实施，每波带测试与门禁）→ 复审闭环。
- 审计记录落 `docs/audit/` 与 SDD workspace。

## 4. 验收标准（定义「完成」）

1. **S1**：soak ≥2h 完成（≥240 轮，轮耗时/RSS/输出字节断言稳定）→ 回填 `docs/audit/2026-08-24-stability.md`（CONDITIONAL PASS 转定论；若失败则如实记 FAIL 并修复后重跑）。
2. **S2**：GitHub 公开仓库建好并推送全部历史；CI SIGTERM 冒烟 job 真跑通过（Actions 绿）；Linux 机（<linux-verify-host>）手动 SIGTERM 优雅停机佐证（或如实记录环境不可用原因，以 Actions 结果为准）。
3. **S3**：全库审计完成——五个审计面发现清单 + 分级处置全部落地（P0/P1 全修并经审查；P2/P3 登记 backlog）；「无发现」面如实记录。
4. **S4**：发布资产齐备——LICENSE（Apache-2.0）、CHANGELOG（0.1.0）、README 完整；Docker 构建验证通过（Linux 机，或如实记录）；release.yml 审查结论。
5. **S5**：全维回归绿——六维全部测试 + perf/resource 基准 ±5% + audit/deny 全绿。
6. **S6**：验收记录 `docs/audit/2026-08-24-quality.md` + 投产核对表（production readiness checklist：构建/配置/文档/安全/监控/运维/回滚逐项）。

## 5. 约束

- 不破坏前五维成果；冻结语义改动须用户批准（D5）。
- **公开仓库零机密**：推送前全库扫凭据（git 历史 + 工作区）；真实 BMC 凭据只在 %TEMP% 临时配置（从未入库，复核确认）。
- 真机只读原则不变（本维真机不做长 soak，见 D4）。
- 零新依赖（审计与修复均不引入新 crate；如审计发现建议新依赖，记 backlog 报用户裁决）。
- 执行沿用 Subagent-Driven + git worktree 隔离；验收记录沿用 `docs/audit/` 惯例。

## 6. 风险与依赖

| 风险 | 处置 |
|---|---|
| 公开仓库暴露（代码/历史含机密） | 推送前凭据扫描 + 历史复核；发现则清理后再推 |
| Linux 验证机环境不可用（Rust 工具链/网络/磁盘） | 如实记录，SIGTERM 佐证降级为 Actions 结果为准 |
| soak 2h 长跑中断（后台任务） | 断点记录；中断则重跑或如实记录中断原因 |
| gh 建仓失败（配额/网络） | 如实记录并降级：CI 静态审查 + 登记待推送（D1 备选） |
| 审计发现过多导致修复波过大 | 分级处置 + 修复波分批；P2/P3 按成本/收益入 backlog |

## 7. 与项目既有规范的一致性

- 审计不改冻结语义（§5）；修复遵循仓库惯例（中文注释、英文提交、TDD、全量门禁）。
- 哲学统一核查以项目既定原则为准：减少手写代码/优先社区资源、YAGNI、诚实记录、`#![forbid(unsafe_code)]`、零 OpenSSL、依赖最小化。
- 验收记录沿用六维 `docs/audit/` 系列；执行沿用 Subagent-Driven + worktree 隔离。
