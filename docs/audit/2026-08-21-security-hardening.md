# 0.1.0 安全专项验收记录

日期：2026-08-21
依据：spec `docs/superpowers/specs/2026-08-21-security-hardening-design.md` §11 验收标准
范围：五维专项第 1 项（安全性）；成果构成后续四维回归基线

## 结论

**PASS**——§11 四条验收标准全部满足：质量门禁与供应链门禁全绿、文档交付齐全且与实现一致、两台真机回归通过（重定向策略与 TLS 改动未破坏采集）、安全语义冻结清单生效。仅保留 3 项既定遗留跟踪（见下），均不阻断。验证执行于 2026-08-24（与 spec/计划同批日期 2026-08-21 对应）。

## 决策记录

| # | 决策点 | 结论 | 状态 |
|---|---|---|---|
| D1 | 威胁模型 | 混合环境 → 分级安全能力（默认安全 + 按需开启） | 已实施 |
| D2 | 凭据注入 | 配置文件 + 环境变量覆盖（不引入外部密钥系统） | 已实施 |
| D3 | 入站保护机制 | 可选 Bearer Token + 可选自身 TLS（rustls）；不做 mTLS/Basic | 已实施 |
| D4 | 供应链 audit 处置 | 保留跟踪 + CI 白名单化（不 [patch] 覆盖、不升级 nv-redfish） | 已实施 |
| D5 | 验收深度 | 威胁模型文档 + 加固实现 + 安全测试 + 安全基线文档 | 已实施 |
| D6 | 默认监听地址 | `0.0.0.0:9417` → `127.0.0.1:9417`（0.1.0 未发布，无兼容包袱） | 已实施 |
| D7 | /reload 未认证时 | 保持现状（仅重读本地文件并校验，不改变 scraper 行为）+ 文档警告 | 已实施 |
| D8 | 出站代理 | 环境变量透传（reqwest 默认行为）+ 文档化，不加配置项 | 已实施 |

## 验收标准对照

| 标准（spec §11） | 结果 | 证据 |
|---|---|---|
| 1. 测试全绿 + clippy/fmt/deny/audit | **PASS** | `cargo fmt --check` exit 0；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo test --all-targets` exit 0（18 个测试二进制、108 项测试、0 失败）；`cargo doc --no-deps` exit 0；`cargo deny check` exit 0（`advisories ok, bans ok, licenses ok, sources ok`；4 条 `license-not-encountered` 为未命中的宽限项警告，非失败）；`cargo audit` exit 0：232 依赖 × 1225 条通告（db commit `bf5c0d2`），仅白名单内 RUSTSEC-2026-0194/0195（quick-xml 0.38.4 构建期依赖）被 ignore，无新增漏洞；`cargo tree` 中 openssl/native-tls 匹配数 0 |
| 2. 文档交付齐全一致 | **PASS** | `docs/security.md`（威胁模型 + 三档基线矩阵 + 运维说明）、README（默认绑定变更提示、认证/TLS 用法、env 覆盖命名约定）、`config.example.yaml`（`web` 节 + 显式 `0.0.0.0` 注释示例）、`docs/design.md` 安全设计节均已交付，行为与实测一致（Bearer 冒烟结果与文档描述相符） |
| 3. 真机回归 | **PASS** | Dell `https://<dell-bmc-host>` / 浪潮 `https://<inspur-bmc-host>` TCP 443 均可达；按验收报告 §1 配置实跑（session 认证、scrape_interval=30s、scrape_timeout=120s、slow_interval=120s，另按新默认显式 `listen_addr: 0.0.0.0:9417`）：快组 up=1（Dell 单轮 2.7s、浪潮 43.2s，与验收基线 ~3s / 44–48s 一致）；Dell 4 轮、浪潮 3 轮全部 failed=0，`redfish_scrape_errors_total` 均为 0；浪潮 session→basic 降级 warn 与验收报告一致；指标产出与验收基线逐项一致（BIOS 数值 4264 = 199+4065、字符串 1937 = 247+1690 精确吻合；事件日志 3226 条 ≈ 基线 3210 + 自然增长；drive 6 / volume 3 / ethernet 2 / memory ECC 8+8 / PSU / sensor 38 / health 83 / info 78 齐全；/metrics 约 1.15MB，基线 1.1MB）。Bearer 冒烟（web.auth_token）：无 token → 401 + `WWW-Authenticate: Bearer`；错 token → 401；无 token 访问 /healthz → 401；带 token → 200 |
| 4. 语义冻结清单生效 | **PASS** | spec §9.3 七项冻结语义（`web` 节配置语义、认证/TLS 行为、SecretString/zeroize、host userinfo 拒绝、重定向降级阻断、防御性上限、name 校验）均有实现与点名测试覆盖：`cargo test --test config_test --test http_test --test bmc_test --test pagination_test` 全绿（20+17+7+8=52 项，0 失败）；TLS 端到端冒烟由 `http_test::tls_end_to_end_with_self_signed_cert` 覆盖（含在 http_test 17 项内）；后续四维回归基线即此四文件 + `integration/scraper` 全套 |

## 遗留跟踪

- quick-xml 构建期漏洞（RUSTSEC-2026-0194/0195，白名单内，跟踪 nv-redfish 上游；运行时二进制不含 quick-xml）
- 分页响应上限为反序列化后校验（nv 无流式 fetch），流式上限 backlog
- 认证/TLS 配置变更需重启生效（与 reload 语义一致，设计既定）
