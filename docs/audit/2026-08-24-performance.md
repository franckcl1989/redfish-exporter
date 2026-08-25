# 0.1.0 性能专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-performance-hardening-design.md` §10 验收标准
范围：五维专项第 3 项（性能）；不破坏第 1/2 项（安全/稳定）成果

## 结论

**PASS**

等价性测试与全部门禁全绿；mock 基准远超阈值（余量巨大）；真机对比确认轮耗时无回归（BMC 请求主导，与 D1 取舍一致）且 `/metrics` 热路径服务器侧延迟 curl p50 2.4–10.7ms（9765 序列 / 1.16MB，测量输出落盘）；安全与稳定双回归全绿；文档与语义冻结完成。

## 决策记录

- **D1 出口与微观优化**：只改出口（预编码快照缓存）与注册路径（register_into O(labels) 查找 + 预分配），不碰 N×walk 与 per-BMC 并发。真机复测证实轮耗时完全由 BMC 请求主导（浪潮快组 44–48s、全量 150.9–155.9s 与验收基线一致），D1 取舍正确。
- **D2 mock 基准 + 真机前后对比**：mock 基准（`tests/perf_test.rs`，Task 3）提供受控管道证据（round/encode/hot_path/bytes/RSS）；真机对比（本文 §性能证据）提供端到端无回归证据与热路径改善证据。

## 验收标准对照

| 标准（spec §10） | 结果 | 证据 |
|---|---|---|
| 1. 等价性测试 + 全部门禁 | ✅ PASS | `tests/http_test.rs` 字节等价测试通过；fmt/clippy/test/audit/deny 全绿（见 §验证清单） |
| 2. mock 基准证据 | ✅ PASS | `perf: rounds=20 round_ms_med=0.33 encode_ms_med=0.06 hot_path_ms_min=0.0003/med=0.0005/max=0.0028 bytes_min=8894/med=8896/max=8896 rss_mb=10.9`（perf_test 复测输出，release + PERF_ASSERT=1；Task 3 初测见其报告；阈值 hot<10ms / encode<500ms / round<2000ms 余量巨大） |
| 3. 真机前后对比 | ✅ PASS | 见 §性能证据：轮耗时无回归；热路径服务器侧 curl p50 2.4 / 10.7ms（9765 序列 / 1.16MB，artifact 见 §性能证据） |
| 4. 安全+稳定双回归 | ✅ PASS | 安全四套件 61 例 + 稳定三套件 40 例全绿；audit/deny 通过（见 §验证清单） |
| 5. 文档与语义冻结 | ✅ PASS | `docs/design.md` 新增 Pre-encoded snapshot cache 条目；`encode()`（String 版）保留、`registry()` 返回类型不变、`Snapshot::update` 签名不变（语义冻结，Task 4 Self-Review 已核对） |

## 性能证据

真机运行：两台真机可达（Dell <dell-bmc-host> / 浪潮 <inspur-bmc-host>）；沿用验收配置（session 认证、`scrape_interval=30s`、`scrape_timeout=120s`、`slow_interval=120s`；`listen_addr: 127.0.0.1:9417` 本机访问），release 二进制实跑 ≥5 轮后测量；`/metrics` 热延迟按 10 次连续请求取 p50。热延迟复测在同配置的第二次启动上进行（`perf2.out.log`：全量轮 155802ms、快组轮 46504 / 47308ms，与首轮运行一致）。

| 指标 | before（验收报告 §3） | after | 变化 |
|---|---|---|---|
| /metrics 热延迟 | ~310ms（9745 序列，1.1MB） | curl p50 2.4 / 10.7ms（9765 序列，1.16MB，两次独立运行） | 服务器侧显著改善；残余为传输开销（见方法注） |
| Dell 快组 / 全量轮 | ~3s / ~11s | 快组 3–5.4s / 全量 ~12s | 不变（BMC 请求主导） |
| 浪潮快组轮 | 44–48s | 5 轮：round duration_ms 46411 / 48445 / 46749 / 28493 / 44052（`perf.out.log`；28493ms 为 failed=1 失败轮，其余 failed=0） | 不变 |
| 浪潮全量轮 | ~159s | 3 轮：round duration_ms 155730 / 155880 / 150999（`perf.out.log`；per-BMC 浪潮 ~152.4 / 155.9 / 150.9s 为现场 /metrics 观测值，无落盘 artifact——留存 `state-20260824-170149.txt` 仅含快组轮采样 43.3151364） | 不变 |
| mock 全量轮 / 编码 | — | 0.33ms / 0.06ms（中位数，perf_test 复测） | 新基准 |
| 内存（工作集 / 私有） | 44.3MB / 31.5MB | 44.9MB / 31.7MB（state artifact 采样） | +0.6 / +0.2MB（测量噪声内；预编码副本 ~2.3MB 理论成本未显著显现） |

- 方法注与 artifact（测量输出落盘于 `%TEMP%\opencode\redfish-perf-run\`）：acceptance §3 的 ~310ms 基线未保留测量方法，本记录 after 给出两种客户端：
  - curl.exe（每次新建连接、无客户端解析）：p50 **2.4ms**（`curl-hot-20260824-170145.txt`）/ **10.7ms**（`curl-hot-20260824-170215.txt`），单请求 1.6–55.9ms——残余时间归因于回环 TCP 建连与 1.16MB 响应体传输（每请求新连接、无 keep-alive），不含任何现场编码（预编码字节直接拼接）；p50 随并发采集轮活动波动（快照交换瞬时独占写锁 + BMC 采集占 CPU），故按两次独立运行给出区间，不对服务器侧改善倍数做单点断言。
  - PowerShell `Invoke-WebRequest`：p50 **498.8ms**（`iwr-hot-20260824-170030.txt`，bytes=1150980）——每请求含 ~490ms 客户端/代理探测与解析开销，数字由客户端主导，与服务器侧无关。
  - 受控编码成本对比由 mock 基准（hot_path_ms_min/med/max 0.0003/0.0005/0.0028，encode_ms_med 0.06，final 修复后复测；Task 3 初测见其报告）覆盖。
- 输出字节 1.16MB vs 1.1MB 为数据增长（Dell 事件日志新增条目；`state-20260824-170149.txt`：series=9765 vs 9745、bytes=1160847），非代码差异；字节等价由 `tests/http_test.rs` 钉死。
- 运行期间浪潮出现一次单资源瞬时失败（failed=1，该轮按设计发布 up=0）→ 按稳定性设计降级 SessionDegraded → basic 兜底下一轮 failed=0 恢复 up=1；全程 errors_total=0 无累计。真机会话为 Windows 强杀结束，未走优雅清理（平台限制，验收报告 §2 已记录）。

## 验证清单（2026-08-24，全绿）

- `cargo fmt --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅（0 警告）
- `cargo test --all-targets` ✅（全部通过；perf_test / soak_test 2 例按设计 ignored）
- `cargo doc --no-deps` ✅
- `cargo audit` ✅（245 依赖无新发现；quick-xml 构建期 2 项按 `.cargo/audit.toml` 白名单）
- `cargo deny check` ✅（advisories / bans / licenses / sources ok；2 条既有 license-not-encountered 警告不变）
- 安全四套件：config(27) + http(19) + bmc(7) + pagination(8) = 61 例 ✅
- 稳定三套件：stability(17) + scraper(20) + integration(3) = 40 例 ✅

## 遗留跟踪

- N×walk 共享预取与 per-BMC 并发（D1 排除，backlog）
- gzip/流式输出（backlog）
- 稳定性跟进：真机测量期间一次 exporter ~12 分钟无日志后静默退出（未复现、无 panic 痕迹），需专项排查（backlog）
