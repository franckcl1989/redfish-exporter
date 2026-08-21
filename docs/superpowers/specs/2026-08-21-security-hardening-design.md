# 安全加固专项 — 设计文档（v1）

日期：2026-08-21
状态：v1 已确认（设计要点在对话中通过）
目标：0.1.0 发布与投产前五维专项之第 1 项「安全性」。定位：对接远程管理卡（BMC）的监控组件不得成为被攻击和被突破的点。威胁模型为**混合环境**（可信内网 / 半信任 / 高暴露并存），方案为**分层可配置安全模型**——默认安全基线（零成本）+ 可选认证 + 可选自身 TLS，零 OpenSSL、无重量级新依赖。
约束：本专项为五维递进的第 1 项，成果构成后续四维（稳定性/性能/资源/功能）的不可破坏基线（见 §9 回归门禁）。

## 1. 已确认决策记录（对话中逐一确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 威胁模型 | 混合环境 → 分级安全能力（默认安全 + 按需开启） |
| D2 | 凭据注入 | 配置文件 + 环境变量覆盖（不引入外部密钥系统） |
| D3 | 入站保护机制 | 可选 Bearer Token + 可选自身 TLS（rustls）；不做 mTLS/Basic |
| D4 | 供应链 audit 处置 | 保留跟踪 + CI 白名单化（不 [patch] 覆盖、不升级 nv-redfish） |
| D5 | 验收深度 | 威胁模型文档 + 加固实现 + 安全测试 + 安全基线文档 |
| D6 | 默认监听地址 | `0.0.0.0:9417` → `127.0.0.1:9417`（0.1.0 未发布，无兼容包袱） |
| D7 | /reload 未认证时 | 保持现状（仅重读本地文件并校验，不改变 scraper 行为）+ 文档警告 |
| D8 | 出站代理 | 环境变量透传（reqwest 默认行为）+ 文档化，不加配置项 |

## 2. 威胁模型与攻击面

### 2.1 入站面（exporter 自身 HTTP，攻击者视角）

| 攻击面 | 现状 | 加固 |
|---|---|---|
| 默认监听 `0.0.0.0:9417` 全网卡 | 任何可达者均可访问全部端点 | 默认收紧为 `127.0.0.1`（D6）；远程部署需显式配置 |
| `/metrics` 无认证 | 监控数据泄露 | 可选 Bearer Token（D3） |
| `/discover` 无认证 | 内网 BMC 拓扑泄露 | 认证开启时统一保护 |
| `/reload` 未认证 POST | 可触发配置重载 | 认证开启统一保护；未开启保持现状 + 文档警告（D7） |
| 无自身 TLS | 明文传输 | 可选 rustls TLS（证书/密钥 PEM） |
| 无 header read timeout | 慢连接占用服务资源 | 加 10s http1 header read timeout |
| BMC name 含控制字符 | 日志注入（伪造日志行） | name 校验：拒绝 C0 控制字符（含 \n \r \t）与 DEL |
| token 字符串比较 | 时序侧信道 | XOR 折叠常量时间比较（手写 ~10 行，零新依赖） |

### 2.2 出站面（到 BMC）

| 攻击面 | 现状 | 加固 |
|---|---|---|
| host URL 可携带 `user:pass@` | 密码绕过 SecretString 且可能入日志 | 配置校验拒绝 host 含 userinfo |
| SecretString 无 zeroize | 密码残留内存 | `zeroize` crate（RustCrypto，零依赖链），Drop 清零 |
| 凭据仅文件明文 | 需落盘 | 环境变量覆盖（D2，§4.3） |
| 重定向 https→http | TLS 降级风险 | 自定义 redirect policy 阻断跨协议降级（§4.4） |
| 代理环境变量隐式生效 | 出站流量可被代理截获 | 保持透传 + 文档化（D8）：BMC 网段应入 NO_PROXY 或依赖 TLS |
| 分页/响应无上限 | 恶意/异常 BMC 致内存膨胀 | 响应体上限 64MB + 分页页数上限 1000（§4.5） |
| `insecure_skip_verify` | per-BMC opt-in 已领先对标 | 保持 + 基线矩阵文档明确其风险边界 |

### 2.3 运行时 / 供应链

- `#![forbid(unsafe_code)]` 保持（lib.rs / main.rs）；TLS 仅 rustls（零 OpenSSL/native-tls）保持。
- cargo audit 2 个 high（RUSTSEC-2026-0194/0195，quick-xml 0.38.4）白名单化（D4，§5）。
- 配置文件权限过宽：Unix 启动时 warn（不 fail，兼容存量部署）；Windows 跳过。
- panic 行为保持默认：`SecretString` Debug 已脱敏、错误路径不含凭据，自定义 panic hook 为 YAGNI（记录于 §8）。

## 3. 入站加固设计

### 3.1 配置 schema（新增 `web` 节，`listen_addr` 保留顶层兼容）

```yaml
web:
  auth_token: null       # 静态 Bearer token，≥16 字符；与 auth_token_file 二选一
  auth_token_file: null  # 从文件读 token（推荐：防进程列表/env 泄露）；两者同时设置报配置错误
  tls_cert_file: null    # PEM 证书；与 tls_key_file 必须同时出现
  tls_key_file: null
```

- 默认监听改为 `127.0.0.1:9417`；`config.example.yaml` 与 `deploy/kubernetes/` 示例改为显式 `0.0.0.0` 并注释提示配认证/TLS；README 注明该破坏性变更。
- `auth_token` 长度 <16 报配置错误（防弱 token）；`auth_token_file` 读取失败启动失败（fail-fast），读出的 token 同样过 <16 字符校验（两个来源语义一致）。
- token 校验：axum middleware 包裹全部路由（含 /healthz 与 /reload，语义统一）；未携带/错误 → `401` + `WWW-Authenticate: Bearer`；比较用 XOR 折叠常量时间算法（对长度不同输入不早退，长度差异折叠进累加器）。
- TLS：`axum-server`（default-features=false，features=`tls-rustls`）承载 rustls；证书/密钥解析或读取失败启动即失败（fail-fast）。实施时确认 axum-server 版本与 axum 0.8 兼容且依赖链无 OpenSSL。
- header read timeout：serve 层配置 10s（hyper-util `http1_header_read_timeout`）。

### 3.2 BMC name 校验

拒绝 C0 控制字符（U+0000–U+001F）与 DEL（U+007F）——防日志注入，与 Prometheus label 转义语义对齐（TextEncoder 已正确转义，校验是第一道防线）。允许正常 Unicode。

## 4. 出站与凭据加固设计

### 4.1 host URL 校验

`Url::parse` 后若 `host` 含 userinfo（`url.username()` 或 `url.password()` 非空）→ 配置错误，引导使用 `username`/`password` 字段。堵住密码绕过 SecretString、经 URL Debug 输出入日志的路径。

### 4.2 SecretString zeroize

引入 `zeroize` crate（^1）：`SecretString` 实现 `Drop` 时对内部字节 `zeroize()`。已知局限（如实文档化）：`BmcHandle` 持有凭据副本供 basic 认证与会话重建（nv-redfish 接口要求 String），该副本生命周期受 nv 内部管理，本专项仅覆盖自有 `SecretString` 主实例的 drop 清零。

### 4.3 环境变量覆盖

- 变量名：`REDFISH_EXPORTER_PASSWORD_<NAME>`（BMC name 大写、非 `[A-Z0-9]` 字符替换为 `_`）。
- 规则：env 存在且非空 → 覆盖文件值；存在但为空 → 配置错误（与 password 非空校验一致）；未设置 → 用文件值。
- 两个 BMC name 映射到同一 env 名（如 `a-b` 与 `a_b`）→ 配置错误（防歧义）。
- 语义：env 覆盖在 `load_config` 内生效，`/reload` 重读时同样应用（配置对象热替换立即反映；scraper 凭据生效需重启，与既有 reload 语义一致）。

### 4.4 重定向策略

reqwest 自定义 `redirect::Policy::custom`：允许 http→http、http→https、https→https；**拒绝 https→http**（阻止 TLS 降级）。对 http BMC（明文 basic 场景）无影响。

### 4.5 防御性上限

- 分页 raw fetch（`pagination.rs`）：响应体上限 64MB（真实最大载荷浪潮 BIOS 166KB 的约 400 倍，仅防灾难）；超过报错计 failed resource。
- 分页页数上限 1000（真机最大约 23 页的 40 余倍；visited-URL 去重仍是防循环主防线，页数上限为第二防线）；超过报错 + warn。

### 4.6 配置文件权限与 token 生命周期

- Unix：`PermissionsExt` 检查 `mode & 0o077 != 0` → warn（不 fail）；Windows 跳过。
- 会话 token 生命周期文档化：token 驻留 nv 内部凭据（不可控），shutdown 删除服务器端会话已有（Task 13）；不新增机制。

## 5. 供应链与 CI

1. **cargo audit 白名单**：CI 中忽略 RUSTSEC-2026-0194/0195（`.cargo/audit.toml` ignore 列表），附理由注释：quick-xml 仅经 nv-redfish-csdl-compiler 进入构建期、不进运行时二进制；跟踪 nv-redfish 上游，升级后移除白名单。
2. **deny 策略核对**：确认 deny.toml 开启 advisories/bans/licenses/sources 四类且未弱化。
3. **依赖树审查**：`cargo tree --edges normal` 运行时依赖审查结果附录进 `docs/security.md`，确认零 OpenSSL/native-tls 基线保持；新增依赖仅 `zeroize` + `axum-server`（rustls）。
4. **发布资产**：musl 静态 + distroless nonroot 保持；SBOM（cargo cyclonedx）与二进制签名记 backlog。

## 6. 测试计划

| 测试组 | 覆盖点 |
|---|---|
| `config_test.rs` 扩展 | host 含 userinfo 拒绝；name 控制字符拒绝；env 覆盖优先级（覆盖文件值）；空 env 值报错；env 名映射冲突报错；默认绑定 `127.0.0.1` 断言；token <16 字符报错；auth_token 与 auth_token_file 同时设置报错 |
| `http_test.rs` 扩展 | 无 token → 401 + `WWW-Authenticate: Bearer`；错 token → 401；对 token → /metrics /discover /info /healthz /reload 全端点 200；常量时间比较函数对长度不同输入不早退（XOR 折叠结构验证） |
| TLS | 无效 PEM 启动失败（fail-fast）；自签证书端到端启动成功（集成测试） |
| zeroize | `SecretString` drop 后底层字节全零（直接构造 + drop 后检查切片） |
| 重定向 | mock 302 https→http 断言不跟随；http→https 断言正常跟随 |
| 分页 | 响应体超限报错；页数超限报错 |

## 7. 文档交付

- `docs/security.md`（新）：§2 威胁模型与防线映射 + 三档基线矩阵：

| 档位 | 入站 | 出站 | 适用 |
|---|---|---|---|
| 可信内网 | `127.0.0.1` 或显式 `0.0.0.0` 依赖网络边界 | TLS 校验默认开启 + `ca_cert_file` | 同网段 Prometheus 直连 |
| 半信任 | + `auth_token_file` | 同上 + NO_PROXY 覆盖 BMC 网段 | 跨网段/多租户 |
| 高暴露 | + `tls_cert_file`/`tls_key_file` | 严格校验（禁 `insecure_skip_verify`） | 公网可达 |

- README 安全节更新（默认绑定变更、认证/TLS 用法、env 覆盖命名约定）；`config.example.yaml` 更新；`docs/design.md` 安全设计节更新。
- 交付物验证：两端点冒烟（认证开启/关闭、TLS 开启）纳入验收。

## 8. 明确不做（YAGNI / backlog）

| 项 | 理由 |
|---|---|
| mTLS / Basic 认证 | D3 已选 Bearer；多一条认证路径多一份攻击面与测试面 |
| Vault/外部密钥系统 | D2 已选 env 覆盖；大工程超出 0.1.0 节奏 |
| 代理显式配置项 | D8 透传 + 文档；确有劫持威胁的部署再评估 |
| 连接数限制 / 请求体限制 | 端点均无 body 提取；反向代理层职责，backlog |
| 自定义 panic hook | 错误路径不含凭据（SecretString 脱敏），保持默认便于排障 |
| 配置文件权限 fail-fast | 仅 warn，兼容存量部署 |
| SBOM / 二进制签名 | 记录 backlog，随发布流程后续评估 |
| fuzzing（配置解析/HTTP） | 完整 SDL 套餐未入选（D5），backlog |

## 9. 回归门禁与安全语义冻结（衔接后续四维）

1. **安全测试套件**：后续每个维度（稳定性/性能/资源/功能）完成后全量重跑（纳入 CI）。
2. **CI 门禁**：cargo audit（白名单内零新增）/ deny / clippy -D warnings / fmt 全绿。
3. **安全语义冻结**：`web` 节配置语义、认证/TLS 行为、SecretString/zeroize、host userinfo 拒绝、重定向降级阻断、防御性上限、name 校验在 0.1.0 内冻结——后续维度只增不改；任何改动须在对应维度设计中显式声明并重跑安全测试。
4. **验收记录**：每维度验收记录延续 `docs/audit/` 惯例追加。

## 10. 风险与依赖

| 风险 | 处置 |
|---|---|
| 重定向降级阻断可能与真实 BMC 行为冲突（个别 BMC 登录后 302 至 http） | 真机回归验证（Dell/浪潮快组 up=1）必做；若冲突，回设计门评估 per-BMC 豁免，不静默放行 |
| axum-server rustls 依赖链 | 实施时验证版本兼容与零 OpenSSL；失败则退回 tokio-rustls 手动 acceptor 方案 |
| 默认绑定收紧为破坏性变更 | 0.1.0 未发布无存量用户；config.example / deploy 示例 / README 同步更新并显著提示 |
| zeroize 无法覆盖 nv 内部凭据副本 | §4.2 已如实文档化局限，不夸大声明 |

## 11. 验收标准（定义「完成」）

1. §6 全部测试通过；clippy/fmt/deny/audit（白名单内）全绿。
2. §7 文档交付齐全且与实现一致（无过时注释、无占位符）。
3. 真机回归：两台真机（Dell R750 / 浪潮）快组 up=1、指标产出与加固前一致（重定向策略与 TLS 配置改动未破坏采集）；认证/TLS 端点冒烟通过。
4. 安全语义冻结清单（§9.3）生效，后续维度以此为基线。

## 12. 与项目既有规范的一致性

- 不破坏 `docs/design.md` 既有架构（快照缓存、认证流、优雅停机）与指标命名约定；安全项均为增量层（配置校验、middleware、redirect policy、防御上限）。
- 保持 `#![forbid(unsafe_code)]`、零 OpenSSL、依赖最小化三条既有安全基线。
- 配置项风格与既有 `humantime`/serde 约定一致；错误路径沿用 `ConfigError::Invalid` / fail-fast 语义。
