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
| 端点无认证（数据/拓扑泄露） | `web.auth_token[_file]` 开启后 `/metrics`、`/info` 统一 401 + `WWW-Authenticate: Bearer`；常量时间比较防时序侧信道。`/healthz`、`/readyz` 仅暴露状态并保持公开，供编排器探针使用 |
| 明文传输 | `web.tls_cert_file`/`tls_key_file`（rustls，零 OpenSSL） |
| 慢连接占用 | 仅 HTTP/1.1（见局限）；header read timeout 10s，纯 HTTP 与 TLS 两种模式一致生效，覆盖"握手/连接后零字节"的空闲连接（slowloris） |
| 日志注入 | BMC name 拒绝控制字符（配置校验） |

### 出站面（到 BMC）

| 攻击面 | 防线 |
|---|---|
| URL/重定向逃逸 | host 必须为无 userinfo/query/fragment 的绝对 URL；重定向不得改变 scheme/host/port，最多 10 跳 |
| 密码残留内存 | SecretString Drop 时 zeroize；已知局限：nv-redfish 内部凭据副本不受控（如实说明） |
| 密码落盘 | 环境变量覆盖 `REDFISH_EXPORTER_PASSWORD_<NAME>`（name 大写、非字母数字替换 `_`；空值/映射冲突报错） |
| 响应耗尽内存 | 分页单页 64MiB / 页数 1000 / 累计成员 200,000；已知局限：上限为反序列化后校验（nv 类型化 fetch 无流式读取），流式上限记 backlog |
| Prometheus 高基数耗尽 | 逐条事件日志与 BIOS 全属性默认关闭；显式开启时每 BMC 分别默认限制 500/10,000 条，配置硬上限为 5,000/10,000 |
| 资产/日志内容泄露 | `redfish_info` 可含序列号/MAC，opt-in BIOS 与事件日志还可含配置值和消息；远程 `/metrics` 必须使用网络隔离及 `web.auth_token`/TLS，Prometheus 侧限制访问权限与保留周期 |
| TLS 校验绕过 | per-BMC `insecure_skip_verify` opt-in，与 `ca_cert_file` 互斥；CA 文件支持多证书 PEM bundle |
| 代理截获 | 环境变量透传（reqwest 默认）；BMC 网段应入 NO_PROXY 或依赖 TLS |

### 运行时 / 供应链

- `#![forbid(unsafe_code)]`；TLS 仅 rustls（零 OpenSSL/native-tls）；cargo deny 四类策略
- cargo audit：2 个 high（quick-xml，构建期依赖）白名单化，跟踪 nv-redfish 上游（`.cargo/audit.toml`）
- GitHub Actions 固定到完整 commit SHA；release 发布 SHA-256 与锁定依赖 SPDX SBOM；静态二进制分别生成 provenance/SBOM attestation，归档生成 provenance attestation；GHCR 镜像同时发布 BuildKit provenance/OCI SBOM 和 GitHub artifact attestation
- Unix 配置文件权限过宽启动 warn（建议 chmod 600）；Windows 无此检查
- 会话 token 驻留 nv 内部凭据；shutdown 删除服务器端会话（既有 Task 13）

## 基线矩阵

| 档位 | 入站 | 出站 | 适用 |
|---|---|---|---|
| 可信内网 | 默认 `127.0.0.1` 或显式 `0.0.0.0` 依赖网络边界 | TLS 校验默认开启 + `ca_cert_file` | 同网段 Prometheus 直连 |
| 半信任 | + `auth_token_file` | 同上 + NO_PROXY 覆盖 BMC 网段 | 跨网段/多租户 |
| 高暴露 | + `tls_cert_file`/`tls_key_file` | 严格校验（禁 `insecure_skip_verify`） | 公网可达 |

## 运维说明

- 认证开启时 Prometheus 用 `bearer_token_file`；k8s 的 `/healthz`、`/readyz` 探针无需 token。
- 认证、TLS、BMC 和采集调度配置变更均需重启生效。
- `auth_token`（≥16 字符）与 `auth_token_file` 二选一；推荐文件方式（防进程列表/env 泄露），文件末尾换行会被裁剪。

## 局限

- 服务端仅支持 HTTP/1.1：axum-server 的版本探测路径（读前 24 字节区分 h2 前言）没有超时，为确保 10s header 读超时对"零字节慢连接"必然生效，纯 HTTP 与 TLS 两种模式均固定 `http1_only()`，TLS 分支 ALPN 仅声明 `http/1.1`。h2 / h2c / h2-over-TLS 不被服务；Prometheus 抓取走 HTTP/1.1，无运维影响。
- 分页单页上限为反序列化后校验（近似真实响应大小），超限响应体仍会被完整读入；流式上限记 backlog。
- nv-redfish 内部凭据副本的 zeroize 不受控（见出站面）。
