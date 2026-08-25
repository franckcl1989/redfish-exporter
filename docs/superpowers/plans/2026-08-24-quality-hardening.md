# 工程质量专项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 0.1.0 工程质量专项（收官项）——soak ≥2h 收尾 + CI 真跑验证 + 全库五面工程审计与修复 + 发布资产补齐（LICENSE/CHANGELOG/README/Docker 验证），达到生产就绪标准。

**Architecture:** 审计-修复闭环：先建仓推送（CI 自动开跑）+ 后台启动 soak 长跑，同时按审计面分批派发审计子代理；发现分级（P0/P1 必修、P2 裁决、P3 backlog；冻结语义改动报用户批准）；修复波统一实施 + 复审；最后收集 soak/CI 结果、回填稳定性记录、写验收记录与投产核对表。

**Tech Stack:** Rust 1.90 / edition 2024、gh CLI（已登录 franckcl1989）、GitHub Actions、Linux 验证机 <linux-verify-host>（<user>）、PowerShell、既有测试基建。零新依赖。

**Spec:** `docs/superpowers/specs/2026-08-24-quality-hardening-design.md`（计划依 spec 论证，执行者须先读 spec）

## Global Constraints

- rust-version 1.90、edition 2024；`#![forbid(unsafe_code)]` 保持；**零新依赖**（审计与修复均不引入新 crate；如需新依赖记 backlog 报用户裁决）
- **冻结语义**（五维冻结清单：指标命名/help/labels、配置语义、状态机行为、发布语义、/metrics 输出字节等价）：修复触达 → 停止并报控制器转用户批准，**未经批准不得实施**
- **公开仓库零机密**：推送前全库凭据扫描（git 全历史 + 工作区）；真实 BMC 凭据只在 %TEMP% 临时配置（从未入库，复核确认）
- 每任务结束 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets` 全绿后才提交（纯审计任务无代码改动时也须跑门禁确认现状绿）
- 代码注释用中文；提交信息用英文、仓库风格；诚实记录（每数字有制品引用；无发现的审计面如实记「无发现」）
- 真机只读原则不变；本维真机不做长 soak（spec D4）
- 执行沿用 Subagent-Driven + worktree 隔离；SDD workspace：`.worktrees/quality-hardening/.superpowers/sdd/2026-08-24-quality-hardening/`

## 执行前置（控制器，非任务）

- [ ] **创建 worktree**：`git worktree add .worktrees/quality-hardening -b feature/quality-hardening`（master @ 2745dc0）
- [ ] **后台启动 soak 长跑**（与审计并行，隔离 target 目录避免 cargo 锁冲突）：

```powershell
$env:CARGO_TARGET_DIR = "$env:TEMP\opencode\soak-target"
$env:SOAK_SECS = "7300"
Start-Process powershell -ArgumentList "-NoProfile","-Command","cd C:\Users\franck\Documents\opencode\redfish-exporter; cargo test --release --test soak_test -- --ignored --nocapture *> `$env:TEMP\opencode\soak-2h.log" -WindowStyle Hidden
```

（soak_test 断言逐轮完整 MockBmc 采集 + 轮耗时/RSS/输出字节；7200s+ 余量 ≥240 轮。记录启动时间；Task 7 收集结果。）

---

### Task 1: 凭据扫描 + GitHub 建仓推送

**Files:**
- 无代码改动（git 操作 + 扫描报告）

**Interfaces:**
- Produces: 公开 GitHub 仓库（remote origin）+ 推送后自动触发的 CI run；扫描结论（SDD workspace 报告）

- [ ] **Step 1: 全库凭据扫描**

- 工作区 + git 全历史：`git log -p --all | Select-String -Pattern "password|passwd|token|secret|<dell-bmc-host>|<inspur-bmc-host>|<ssh-password>" -CaseSensitive:$false` 与工作区文件 grep（config.example.yaml 为示例文件无真实凭据——确认；%TEMP% 配置不入库——确认）
- 结论写入 `%TEMP%\opencode\redfish-quality-scan.txt` 与 SDD 报告；发现任何真实凭据 → 停止并报控制器

- [ ] **Step 2: 建仓并关联**

```powershell
gh repo create redfish-exporter --public --source . --remote origin --description "Prometheus exporter for Redfish BMCs (0.1.0)"
```

（如该名已占用：`gh repo create redfish-exporter-<suffix> --public ...` 并说明。仓库归属 franckcl1989。）

- [ ] **Step 3: 推送全部历史**

```powershell
git push -u origin master
```

- [ ] **Step 4: 确认 CI 触发**

- `gh run list --limit 3` 确认 push 触发的 workflow run（SIGTERM job 在其中；结果 Task 6 收集）
- 记录 run ID 进报告

- [ ] **Step 5: 提交扫描结论**

无 git 提交（无代码改动）；报告记录仓库 URL + run ID + 扫描结论。

---

### Task 2: 代码面审计（门禁 + 关键路径复查）

**Files:**
- 无代码改动（审计产出发现清单，修复在 Task 5）

**Interfaces:**
- Consumes: 五维冻结清单（docs/superpowers/specs/2026-08-2*-design.md §9 各冻结节 + design.md）
- Produces: 代码面发现清单（SDD workspace 报告：文件:行 + 分级 P0-P3 + 修复建议）

- [ ] **Step 1: 全量门禁实跑**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`; `cargo doc --no-deps`; `cargo audit`; `cargo deny check`
Expected: 现状应全绿；任何非绿项直接列为 P0/P1 发现

- [ ] **Step 2: 关键路径复查**

逐项读代码并核对（每项产出结论与证据，或「无发现」）：
1. 状态机（src/stability.rs）：三态转移、冷却计数、SessionDegraded 语义与冻结清单一致
2. 锁序（src/registry.rs）：scratch → inner 写锁顺序无环；poison 恢复语义（recover_lock）
3. 错误模型：BMC 级/资源级隔离、failed_resources、up=0 语义、errors_total 累计
4. 发布语义：scraper 三处发布路径（成功/失败/冷却）与 build_registry 一致性
5. 编码等价：预编码缓存 vs 现场编码（等价性测试存在且有意义）
6. OEM 读取路径（processors/storage/systems）：oem_value 与 raw JSON 导航的一致性、字符串解析容错
7. 优雅停机路径（http.rs serve_on / scraper run）：watch 信号、超时、会话清理（Task 13 先例）
8. 分页 walker（pagination.rs）：双 nextLink 键变体、防御上限、循环防护

- [ ] **Step 3: 发现清单**

汇总为发现清单（每项：文件:行、分级、证据、修复建议）；无发现的路径如实记「无发现」。报告落 SDD workspace `task-2-report.md`。

- [ ] **Step 4: 门禁确认提交**

无代码改动则无提交；若 Step 1 暴露门禁问题（如 doc 警告），修复并提交（小改，非 Task 5 范围）。

---

### Task 3: 文档一致性 + 语义/命名一致性审计

**Files:**
- 无代码改动（审计产出发现清单）

**Interfaces:**
- Consumes: README.md、docs/design.md、docs/metrics.md、docs/security.md、config.example.yaml、docs/audit/ 六维记录、五维 spec 冻结清单
- Produces: 两个审计面的发现清单（SDD workspace 报告）

- [ ] **Step 1: 文档一致性核对**

- docs/metrics.md 指标名清单 vs src/metrics.rs 常量：53 个指标名逐一核对（名称/help/labels/来源；本维新 8 族已在列——确认）
- config.example.yaml vs src/config.rs：每个配置节（web/stability/bmc/慢组）字段、默认值、注释与代码一致；示例文件可被加载（`cargo run -- --config config.example.yaml --dry-run` 如存在，否则解析级验证）
- docs/design.md 各节（快照/调度/稳定性/资源/安全）与实现一致；docs/security.md 威胁模型与实现一致
- README：安装/配置/运行/端点/指标清单与实现一致；告警示例（deploy/prometheus）与新指标族对应
- docs/audit/ 六维验收记录之间无矛盾（基线数字、遗留跟踪、冻结声明）

- [ ] **Step 2: 语义/命名一致性核对**

- 指标命名规则：`redfish_` 前缀 + 单位后缀（_watts/_percent/_seconds/_mhz/_volts/_bytes）跨 26 族自洽；状态类 label 模式（值恒 1 + label 携带状态）统一（health_status/indicator_led/controller_status/drive_oem_status）
- label 惯例：bmc/system/storage/id/resource_type/state 各族的 label 集合与顺序语义一致
- 配置语义：web（listen/auth_token/tls）、stability（cooldown_failures/base/max）与冻结清单一致
- 错误语义：up=0、failed_resources、cooldown 轮发布、SessionDegraded、errors_total 跨文档与代码一致

- [ ] **Step 3: 发现清单**

每项：文件:行、分级、证据、修复建议；无发现如实记录。报告落 SDD workspace `task-3-report.md`。

- [ ] **Step 4: 门禁确认**

无代码改动；跑 `cargo test --all-targets` 确认现状绿。

---

### Task 4: 哲学统一审计 + 发布资产补齐

**Files:**
- Create: `LICENSE`（Apache-2.0 全文）、`CHANGELOG.md`（0.1.0 变更）
- Modify: `README.md`（如需）、`.github/workflows/release.yml`（如审查发现缺陷）、`.gitignore`/`.dockerignore`（如需）
- 审计产出：哲学统一发现清单（SDD workspace 报告）

**Interfaces:**
- Consumes: 五维验收记录（变更来源）、差距表、spec/plan 系列
- Produces: LICENSE、CHANGELOG、README 修订、release.yml 审查结论

- [ ] **Step 1: 哲学统一审计**

- 社区资源优先：盘点 src/ 与 Cargo.toml——手写实现 vs 社区 crate 覆盖；确认无手写编码器/存储替换（资源维 D1）、无 fork 依赖；记录任何「手写 vs 现成 crate」候选（如 JSON 导航——记 P3 或「无发现」按实际情况）
- YAGNI：无死代码、无未用依赖（`cargo machete` 若不可用则人工核对 Cargo.toml 依赖使用）、无过度抽象
- 诚实记录惯例：六维验收记录的制品引用可复核（抽查 2-3 个数字）
- 中文注释惯例：抽查 src/ 注释语言一致性
- 重复代码盘点：expect_full_round 复制（既有裁决）、LED 双块（既有 minor）、push_value 多份（各 collector 惯例——评估是否属「合理惯例」还是「应抽公共」）

- [ ] **Step 2: LICENSE（Apache-2.0）**

- 写入 Apache-2.0 标准全文（2026 年 Apache Software Foundation 官方文本，NOTICE 文件不建）；仓库名/版权行按项目惯例（不虚构版权人——用通用表述或用户已定名 franckcl1989，采用「Copyright 2026 redfish-exporter contributors」风格）

- [ ] **Step 3: CHANGELOG（0.1.0）**

- 按五维验收记录 + 审计维汇总 0.1.0 完整变更：初始实现（18 指标族/快照架构/分频/分页/会话管理）+ 六维专项逐维要点 + 已知限制/backlog 摘要。Keep a Changelog 风格。

- [ ] **Step 4: README 完整性审查与修订**

- 核对：简介/特性/安装（cargo install + Docker）/配置示例/运行/端点表（/metrics /healthz /info /discover /reload）/指标清单指引/告警与 Grafana（deploy/）/许可证/致谢。缺什么补什么；与实际一致。

- [ ] **Step 5: release.yml 审查 + gitignore 复核**

- release.yml：触发（tag）、构建矩阵、资产上传（二进制/Docker）、gh release 流程逐行审查；缺陷修复（若涉及冻结语义——release 流程不属冻结清单，直接修）
- .gitignore/.dockerignore 复核：target/.superpowers/临时制品均被忽略；.dockerignore 不遗漏

- [ ] **Step 6: 门禁与提交**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`
Expected: 全绿

```powershell
git add LICENSE CHANGELOG.md README.md .github/workflows/release.yml .gitignore .dockerignore
git commit -m "docs: Apache-2.0 license, 0.1.0 changelog, README and release workflow review"
```

---

### Task 5: 修复波（审计 P0/P1 修复 + P2 裁决）

**Files:**
- 按 Task 2/3/4 发现清单确定（任务开始时由控制器汇总去重后下发）

**Interfaces:**
- Consumes: Task 2/3/4 的发现清单（控制器汇总去重后写入 `task-5-findings.md`）
- Produces: 修复提交 + 复审闭环；P2 裁决与 P3 backlog 登记

- [ ] **Step 1: 冻结语义筛查**

- 每个 P0/P1 发现判定是否触达五维冻结清单（指标命名/help/labels、配置语义、状态机、发布语义、输出字节）——触达项**停止修复**，列出报控制器转用户批准；未触达项直接修

- [ ] **Step 2: 实施修复（TDD）**

- 每个修复：先写/改失败测试（如适用）→ 实现 → 覆盖测试绿 → 全量门禁绿
- 文档类修复：改后全库 grep 一致性核对
- 一次提交一个修复主题（或按面分组，英文提交信息）

- [ ] **Step 3: P2 裁决与 P3 登记**

- P2：按成本/收益裁决修或 backlog（理由记录）；P3：全部 backlog
- 裁决表写入 `docs/audit/2026-08-24-quality.md`（Task 7 转述）

- [ ] **Step 4: 报告**

修复清单（发现 → 修复 commit → 测试证据）；冻结语义待批清单报控制器。

---

### Task 6: CI SIGTERM 验证 + Docker 构建验证（Linux 机）

**Files:**
- 无代码改动（验证 + 佐证记录；如 CI 配置缺陷则修复提交）

**Interfaces:**
- Consumes: Task 1 的仓库 URL + run ID；Linux 验证机 <linux-verify-host>（<user>）
- Produces: CI 真跑结论 + Linux 手动 SIGTERM 佐证 + Docker 构建验证结论（SDD workspace 报告）

- [ ] **Step 1: GitHub Actions 结果收集**

- `gh run view <run-id>` / `gh run list`：SIGTERM graceful shutdown smoke job 结果（通过/失败 + 日志摘录）；全 workflow 绿/红如实记录
- 失败：读日志定位（环境 vs 代码）——代码缺陷直接修（属 CI 配置/行为，非冻结语义），重推重跑

- [ ] **Step 2: Linux 机连通性与工具链**

- 连通性：`Test-NetConnection <linux-verify-host> -Port 22`；SSH 登录（密码认证；Windows 端可用 plink/ssh 交互——若子代理环境无法非交互输密码，如实记录并跳过手动佐证，以 Actions 结果为准）
- 工具链：`which docker rustc cargo`；无则按序尝试 docker（最省）→ rustup 安装（`curl https://sh.rustup.rs -sSf | sh -s -- -y`）→ 均不可用则如实记录
- 环境不可用：降级记录（Actions 结果为准），不阻塞

- [ ] **Step 3: Docker 构建验证（Linux 机）**

- 推送/上传 Dockerfile 与源码（git clone 新仓库或 scp），`docker build -t redfish-exporter:test .`；构建通过 + 镜像可启动（`docker run --rm redfish-exporter:test --help` 或等价）→ S4 证据
- 构建失败：修复 Dockerfile（发布资产，非冻结语义），重跑

- [ ] **Step 4: 手动 SIGTERM 佐证（Linux 机）**

- 读 `.github/workflows/ci.yml` 的 SIGTERM job 步骤，在 Linux 机上手动复刻：构建 → 启动 exporter（mock 或最小配置）→ 发送 SIGTERM → 断言优雅停机（日志/退出码）
- 结果与 Actions 对照，结论写报告

- [ ] **Step 5: 报告**

CI 结果、手动佐证结果、Docker 验证结果、任何修复提交；全部证据落报告。

---

### Task 7: 验收记录 + 投产核对表 + 全维回归 + soak 收集回填

**Files:**
- Create: `docs/audit/2026-08-24-quality.md`
- Modify: `docs/audit/2026-08-24-stability.md`（soak 回填：CONDITIONAL PASS → PASS/FAIL）

**Interfaces:**
- Consumes: Task 1-6 全部产出；soak 后台日志（%TEMP%\opencode\soak-2h.log）；五维验收记录
- Produces: 验收记录 + 投产核对表 + 稳定性记录回填

- [ ] **Step 1: soak 结果收集**

- 检查后台 soak 进程与日志：断言通过/失败、轮数、轮耗时/RSS/字节稳定性摘录
- 通过 → 回填 `docs/audit/2026-08-24-stability.md` 标准 2（soak ≥2h）为 PASS，结论 CONDITIONAL PASS → PASS；失败/中断 → 如实记录并报告控制器（重跑或按失败处置）

- [ ] **Step 2: 撰写验收记录 `docs/audit/2026-08-24-quality.md`**

```markdown
# 0.1.0 工程质量专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-quality-hardening-design.md` §4 验收标准
范围：五维专项收官项（工程质量）；不破坏前五维成果

## 结论
**PASS / CONDITIONAL PASS / FAIL**（执行者按证据填写）

## 决策记录
D1 公开仓库+Actions 验证 / D2 全维度审计 / D3 Apache-2.0 / D4 mock 2h soak / D5 分级处置

## 验收标准对照（S1-S6）
| 标准 | 结果 | 证据 |
|---|---|---|
| S1 soak ≥2h + 稳定性记录回填 | | soak 日志摘录 + stability.md 回填行 |
| S2 仓库+CI 真跑+Linux 佐证 | | 仓库 URL + run 结果 + 佐证记录 |
| S3 全库审计完成 | | 五面发现清单汇总 + 处置状态 |
| S4 发布资产 | | LICENSE/CHANGELOG/README + Docker 验证 |
| S5 全维回归 | | 六维测试 + perf/resource ±5% + audit/deny |
| S6 投产核对表 | | 下表 |

## 审计发现汇总
| 面 | 发现数 | P0/P1 修复 | P2 裁决 | P3 backlog |
|---|---|---|---|---|

## 投产核对表（production readiness）
| 项 | 状态 | 证据 |
|---|---|---|
| 构建（Windows/Linux/Docker） | | |
| 配置（示例可运行、语义冻结） | | |
| 文档（README/metrics/design/security/CHANGELOG） | | |
| 安全（审计结论、威胁模型、扫描） | | |
| 监控（指标/告警/Grafana） | | |
| 运维（端点/优雅停机/CI/release） | | |
| 回滚（版本/镜像/配置回退说明） | | |

## 遗留跟踪
（P2/P3 backlog + 冻结语义待批项 + 待推送项已闭环清单）
```

- [ ] **Step 3: 全维回归**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`; `cargo audit`; `cargo deny check`
点名：安全四套件 + 稳定套件 + `cargo test --release --test perf_test -- --ignored --nocapture`（基线：round 0.37-0.38 / encode 0.081-0.085 / hot 0.0005 / bytes 10949 ±5%）+ `cargo test --release --test resource_test -- --ignored --nocapture`
Expected: 全绿 + 容差内

- [ ] **Step 4: 提交**

```powershell
git add docs/audit/2026-08-24-quality.md docs/audit/2026-08-24-stability.md
git commit -m "docs: 0.1.0 quality hardening acceptance record and stability soak backfill"
```

---

## Self-Review 记录

- **Spec 覆盖**：§3.1 五审计面（Task 2 代码面 / Task 3 文档+语义 / Task 4 哲学+资产）、§3.3 执行方式（修复波 Task 5 + 复审）、§4 S1（前置 soak + Task 7 收集回填）/S2（Task 1+6）/S3（Task 2-5）/S4（Task 4+6）/S5（Task 7）/S6（Task 7）、§5 零机密（Task 1 Step 1）、§6 风险（各任务降级路径内置）。
- **占位符**：无 TBD；修复波文件清单在任务开始时由控制器汇总去重下发（流程性指令，非内容占位）；仓库名冲突备选已给。
- **类型一致性**：soak 日志路径（%TEMP%\opencode\soak-2h.log）在控制器前置与 Task 7 一致；CI run ID 由 Task 1 产出、Task 6 消费；P2/P3 裁决表 Task 5 产出、Task 7 转述。
- **边界核查**：Task 1 推送为已批准操作（spec D1）；Task 5 冻结语义批准门（Global Constraints + Step 1）；soak 隔离 CARGO_TARGET_DIR 避免与审计任务 cargo 锁冲突；Linux 机不可用降级路径明确（Actions 为准）。
