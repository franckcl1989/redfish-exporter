# 0.1.0 稳定性专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-stability-hardening-design.md` §10 验收标准
范围：五维专项第 2 项（稳定性）；不破坏第 1 项（安全性）成果

## 结论

**PASS**（2026-08-25 回填）—— 实现、故障注入、全部门禁与安全回归全部完成且绿；soak ≥2h 已执行并通过（2026-08-25，五维收尾统一执行，见对照表标准 2）；CI Linux SIGTERM job 已随公开仓库推送由 Actions 真跑验证通过（graceful-shutdown-smoke ✓，质量维 Task 6）。

理由：
- 验收标准 1/5/6 已达成（本机全部门禁 + 安全回归 + 文档与语义冻结，证据见对照表）；标准 3（CI Linux SIGTERM job）已入 CI 并经 Actions 真跑验证通过（质量维 Task 6：run 32815997963，exporter exit code 0 + "shutdown complete"；Linux 机容器内手动 SIGTERM 复刻同样通过）。
- 标准 2（soak ≥2h）：soak 基建已完成并短跑验证（30s 短跑，Task 6）；≥2h 实跑已执行（SOAK_SECS=7300，7214 轮，断言全绿），本记录已回填（见对照表标准 2）。
- 标准 4（真机回归）：本轮仅做 TCP 可达性检查（两台 BMC 443 端口均可达）；完整真机回归属于验收周期，不在本任务范围。

## 决策记录

- D1 防御实现 + 故障注入矩阵 + 本地 soak（真机长稳推迟到投产前灰度观察）
- D2 指数退避冷却（连续失败 3 轮触发，60s→120s→240s→上限 300s，成功即复位）
- D3 冷却/降级期 basic 兜底采集（401 重登失败后 basic 照常采集 + session 按退避节奏重试，两轨并行）
- D4 CI Linux SIGTERM 冒烟 job（`graceful-shutdown-smoke`，commit bc6d789）

## 验收标准对照

| 标准（spec §10） | 结果 | 证据 |
|---|---|---|
| 1. 故障注入矩阵全绿 + 全部门禁 | PASS | 矩阵四行全部有测试：状态机单元（stability_test 纯函数组）；冷却发布语义（scraper_test `cooldown_publish_semantics_pin_frozen_contract`、`retry_failure_round_increments_errors_total`）；真实 401（`fault_injection_401_only_server_degrades_then_cools`、`fault_injection_session_401_basic_ok_degrades_with_mark`）；乱码响应（`fault_injection_garbled_root_mock_reaches_cooldown`：MockBmc 期望队列注入解析错误 → 轮失败 → 冷却；`fault_injection_garbled_response_round_fails_then_cools`：真实 HTTP 乱码载荷端到端同路径）。stability_test 17/17、scraper_test 20/20 全绿；`cargo test --all-targets` exit 0：138 passed / 0 failed / 1 ignored（soak 按设计 `#[ignore]`，默认不运行）；`cargo fmt --check` exit 0；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo doc --no-deps` exit 0；soak 短跑 `SOAK_SECS=20` 1 passed（RSS 采样与新增断言验证通过）（本机门禁输出见 `.superpowers/sdd/2026-08-24-stability-hardening/task-9-report.md`） |
| 2. soak ≥2h | **PASS**（2026-08-25 回填） | ≥2h 实跑已于五维收尾执行：`SOAK_SECS=7300`（tests/soak_test.rs 健康 MockBmc 长循环），总时长 7300.62s ≥2h，7214 轮（≥240 轮要求），1 passed / 0 failed；断言全绿（轮耗时稳定、无失败资源、输出字节恒定、RSS 无泄漏式增长）。日志 `%TEMP%\opencode\soak-2h.log` 终态行：`soak ok: 7214 rounds, round_ms med 0.6872999999999999->0.7364, rss_mb med 12.140625->4.5703125, bytes 10949`。soak 基建 commit 1beccd8（`SOAK_SECS` 参数化，逐轮完整 MockBmc 采集 + 轮耗时/RSS/输出字节断言）；短跑验证（Task 6，证据见 `.superpowers/sdd/2026-08-24-stability-hardening/task-6-report.md`）：`SOAK_SECS=30` → `soak ok: 30 rounds, round_ms med 0.5785->0.5937, rss_mb med 10.69921875->10.8046875, bytes 8896`，1 passed |
| 3. CI Linux SIGTERM job | 已入 CI；推送后验证 **PASS**（2026-08-25 回填，质量维 Task 6） | ci.yml `graceful-shutdown-smoke` job（ubuntu runner：构建 → 临时 config（不可达 BMC + 长 interval）→ 后台启动 → `kill -TERM` → 断言退出码 0 且日志含 `shutdown complete`，commit bc6d789；超时护栏（spec §9 风险处置）：job 级 `timeout-minutes: 10` + 退出等待 `timeout 30`，挂死即 job 红，随终审修复提交补入）。本地 Windows 无法发 SIGTERM，本机无法等价实跑 → 待推送后 CI 验证 |
| 4. 真机回归 | 可达性已确认；回归未执行（按任务范围） | TCP 连通性检查（TcpClient，2s 超时）：`<dell-bmc-host>:443 reachable`、`<inspur-bmc-host>:443 reachable`。完整真机回归（Healthy 路径与基线一致性验证）属于验收周期，本任务仅记录可达性发现 |
| 5. 安全回归 | PASS | 四测试文件全绿 exit 0：config 27 + http 18 + bmc 7 + pagination 8 = 60 passed；`cargo audit` exit 0（1225 advisories 加载、245 crate 依赖扫描、无漏洞报告）；`cargo deny check` exit 0（advisories ok, bans ok, licenses ok, sources ok；3 条 unmatched license allowance 警告为 deny.toml 配置卫生问题，非失败，延续现状）（本机回归输出见 `.superpowers/sdd/2026-08-24-stability-hardening/task-9-report.md`） |
| 6. 文档与语义冻结 | PASS | `docs/design.md` §"Adaptive scheduling (stability)"（三态状态机 + 冻结配置语义 `cooldown_failures/base/max` + 发布语义：冷却轮 up=0 + resource=cooldown 不累计 errors_total、SessionDegraded 标记）；README Configuration 表 `stability` 行；`config.example.yaml` `stability` 节 + 注释。§6.2 冻结清单核对：`stability` 配置节语义、冷却状态机行为、SessionDegraded 标记、§4.3 发布语义表——后续维度只增不改 |

## 遗留跟踪

- soak ≥2h：已完成（2026-08-25 五维收尾执行 + 本记录回填，见对照表标准 2）
- 真机 24-72h 长稳（投产前灰度观察，D1 既定）
- 熔断健康探测（HEAD 轻量探测）backlog（spec §8）
- 会话失效预检（expiration_time 提前重登）backlog（spec §8）
