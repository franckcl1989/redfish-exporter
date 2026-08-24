# 安全设计（0.1.0 安全专项）

日期：2026-08-21
依据：spec `docs/superpowers/specs/2026-08-21-security-hardening-design.md`

## 威胁模型

部署形态为混合环境（可信内网 / 半信任 / 高暴露并存），采用分层可配置安全模型：
默认安全基线（零成本）+ 可选 Bearer 认证 + 可选自身 TLS。

### 入站面（exporter HTTP）

| 攻击面 | 防线 |
|---|---|
| 默认全网卡监听 | 默认 `127.0.0.1:9417`；远程部署需显式 `0.0.0.0` 并配置认证/TLS |
| 端点无认证（数据/拓扑泄露） | `web.auth_token[_file]` 开启后全端点（含 /healthz、/reload）统一 401 + `WWW-Authenticate: Bearer`；常量时间比较防时序侧信道 |
| 明文传输 | `web.tls_cert_file`/`tls_key_file`（rustls，零 OpenSSL） |
| 慢连接占用 | 仅 HTTP/1.1（见局限）；header read timeout 10s，纯 HTTP 与 TLS 两种模式一致生效，覆盖"握手/连接后零字节"的空闲连接（slowloris） |
| 日志注入 | BMC name 拒绝控制字符（配置校验） |

### 出站面（到 BMC）

| 攻击面 | 防线 |
|---|---|
| URL 内嵌凭据 | host 含 userinfo 拒绝配置 |
| 密码残留内存 | SecretString Drop 时 zeroize；已知局限：nv-redfish 内部凭据副本不受控（如实说明） |
| 密码落盘 | 环境变量覆盖 `REDFISH_EXPORTER_PASSWORD_<NAME>`（name 大写、非字母数字替换 `_`；空值/映射冲突报错） |
| TLS 降级 | 重定向 https→http 阻断 + 10 跳上限 |
| 响应耗尽内存 | 分页单页 64MiB / 页数 1000 / 累计成员 200,000；已知局限：上限为反序列化后校验（nv 类型化 fetch 无流式读取），流式上限记 backlog |
| TLS 校验绕过 | per-BMC `insecure_skip_verify` opt-in，优先 `ca_cert_file` |
| 代理截获 | 环境变量透传（reqwest 默认）；BMC 网段应入 NO_PROXY 或依赖 TLS |

### 运行时 / 供应链

- `#![forbid(unsafe_code)]`；TLS 仅 rustls（零 OpenSSL/native-tls）；cargo deny 四类策略
- cargo audit：2 个 high（quick-xml，构建期依赖）白名单化，跟踪 nv-redfish 上游（`.cargo/audit.toml`）
- Unix 配置文件权限过宽启动 warn（建议 chmod 600）；Windows 无此检查
- 会话 token 驻留 nv 内部凭据；shutdown 删除服务器端会话（既有 Task 13）

## 基线矩阵

| 档位 | 入站 | 出站 | 适用 |
|---|---|---|---|
| 可信内网 | 默认 `127.0.0.1` 或显式 `0.0.0.0` 依赖网络边界 | TLS 校验默认开启 + `ca_cert_file` | 同网段 Prometheus 直连 |
| 半信任 | + `auth_token_file` | 同上 + NO_PROXY 覆盖 BMC 网段 | 跨网段/多租户 |
| 高暴露 | + `tls_cert_file`/`tls_key_file` | 严格校验（禁 `insecure_skip_verify`） | 公网可达 |

## 运维说明

- 认证开启时 Prometheus 用 `bearer_token_file`；k8s 探针需在 httpGet 中带 token。
- 认证/TLS 配置变更需重启生效（/reload 仅热替换配置对象，与 BMC 凭据变更同语义）。
- `auth_token`（≥16 字符）与 `auth_token_file` 二选一；推荐文件方式（防进程列表/env 泄露），文件末尾换行会被裁剪。

## 局限

- 服务端仅支持 HTTP/1.1：axum-server 的版本探测路径（读前 24 字节区分 h2 前言）没有超时，为确保 10s header 读超时对"零字节慢连接"必然生效，纯 HTTP 与 TLS 两种模式均固定 `http1_only()`，TLS 分支 ALPN 仅声明 `http/1.1`。h2 / h2c / h2-over-TLS 不被服务；Prometheus 抓取走 HTTP/1.1，无运维影响。
- 分页单页上限为反序列化后校验（近似真实响应大小），超限响应体仍会被完整读入；流式上限记 backlog。
- nv-redfish 内部凭据副本的 zeroize 不受控（见出站面）。
