# redfish-exporter 0.1.0 设计文档

日期：2026-08-20
状态：待评审

## 1. 背景与目标

基于 NVIDIA 的 [nv-redfish](https://github.com/NVIDIA/nv-redfish)（Rust Redfish 客户端栈）开发 Prometheus exporter，把 BMC 的标准 Redfish 资源暴露为 Prometheus 指标。

0.1.0 目标：

- 架构完整落地（多 BMC、周期采集 + 缓存、双认证、优雅关闭）
- 实现标准 Redfish 全部适合作为监控指标的采集面
- 不适合做监控指标的 nv-redfish 功能（事件/日志/配置管理类）记录到文档，不实现
- 生产就绪：可放入生产环境长期运行

## 2. 硬性约束

| 约束 | 要求 |
|---|---|
| 100% 安全代码 | `#![forbid(unsafe_code)]`，全 crate 无 unsafe |
| 分发形式 | 纯静态链接单二进制（Linux musl），Docker 镜像运行 |
| 工程规范 | 按《Rust 工程开发规范》1.0 执行（见 §3） |
| TLS | rustls（纯 Rust，无 OpenSSL 系统依赖；nv-redfish 已用 rustls-tls） |
| 版本 | 0.1.0 |

## 3. 工程规范落地要点

- **Edition / Toolchain**：项目 Rust 2024 edition；`rust-toolchain.toml` 固定 `1.90.0`（与 nv-redfish 一致），开发机 / CI / Release 同一 toolchain
- **Cargo.lock**：提交（二进制项目）
- **单一 crate**：不拆 micro-crate；模块组织按职责（config / bmc / scraper / registry / collector / http）
- **依赖策略**：最小化；仅引入有明确收益的成熟库；默认关闭不必要 features
- **Owned-first / Concrete-first**：模块边界传递 owned 数据；采集结果用具体 struct；不使用无谓 trait 层（Collector 内部直接用具体函数或最小 trait，只有 dispatch 有真实需求——见 §6）
- **Safe-first**：`#![forbid(unsafe_code)]`；无 nightly
- **错误处理**：`thiserror` 类型化错误；保留 cause 与上下文；不把 secret 放入错误；应用顶层聚合错误
- **Panic 策略**：外部输入（网络/配置/文件）全部走 `Result`；`expect` 仅限不可变 invariant 并写明理由；启动期配置校验失败直接报错退出
- **Async**：单一 tokio 主 runtime；网络 I/O 全 async；无 CPU 热点，不需要 blocking pool
- **Task 生命周期**：每个任务有明确 owner、取消路径、错误处理；scraper 由 main 持有 JoinHandle，shutdown 信号驱动
- **Queue/并发**：无 unbounded queue；每轮采集用有界并发（固定 worker 数）；不跨轮堆积
- **Lock**：指标缓存用 `RwLock` 仅保护注册表替换点（极小临界区）；不用 `Arc<Mutex<AppState>>` 包全状态
- **Timeout**：每 BMC 的 connect/request timeout（配置项）；每轮采集有总 deadline；时长测量用 `Instant`（monotonic）
- **Retry**：单轮内不重试（采集失败下轮自然补偿，语义简单可靠）；session 认证失败会触发重登
- **Graceful shutdown**：收到 SIGTERM/SIGINT → 停止新采集 → 等待进行中一轮（带 deadline）→ 服务器退出
- **Secret**：密码字段禁止 `Debug` 输出、禁止进日志与错误上下文；类型上做 redact
- **Cardinality**：指标 label 仅 `bmc` 与硬件枚举 id（有限集合），禁止 request-id / 任意文本
- **编译时间**：`nv-redfish` features 按需裁剪（只开采集需要的服务 feature），不开 `std-redfish` 全量
- **文档**：注释解释 Why 不解释 What；`docs/` 交付物见 §12

## 4. 范围

### 4.1 A 类：0.1.0 实现为指标（标准 Redfish features）

| 类别 | nv-redfish features | 采集内容 |
|---|---|---|
| 传感器 | `sensors` | Sensor/Fan 读数、单位、健康、上下警告/临界阈值 |
| 热/电源 | `thermal`, `power`, `power-supplies`, `power-equipment`, `controls`, `environment-metrics` | 温度、功耗、电压、泄漏检测、PSU 状态与冗余、Control 读数 |
| 处理器 | `processors` | ProcessorMetrics：利用率、温度、功耗、健康 |
| 内存 | `memory` | MemoryMetrics：容量、带宽/利用率、健康 |
| 存储 | `storages` | Volume/Drive 容量、DriveMetrics/StorageControllerMetrics、预测故障、健康 |
| 网络 | `ethernet-interfaces`, `network-adapters`, `network-device-functions`, `ports` | 链路状态、速率、健康 |
| PCIe | `pcie-devices` | 健康、链路速率 |
| 系统/机箱/管理器 | `computer-systems`, `chassis`, `managers`, `assembly` | 电源状态、健康、型号/序列号 info |
| 固件库存 | `update-service`（只读 SoftwareInventory） | 固件版本、更新状态（info 指标；不做更新动作） |

### 4.2 B 类：记录到 docs/metrics.md，0.1.0 不实现

- `log-services`（事件日志）、`event-service`（事件订阅）、`task-service`（任务轮询）——事件类，无稳定监控语义
- `accounts`、`session-service`、`bios`、`boot-options`、`secure-boot`、`host-interfaces`、`manager-network-protocol`——配置管理类，非监控指标
- 全部 OEM features（`oem-*`）——架构预留，0.2+ 后置

### 4.3 明确不做（0.1.0）

OEM 指标、事件/日志采集、BMC 写操作（PATCH/动作）、Prometheus 动态发现、配置热加载、Windows/macOS 分发（开发可跑，发布仅 Linux 静态二进制）。

## 5. 架构

```
                    config.yaml
                        │
                    Config (校验)
                        │
   ┌────────────────────┼────────────────────────┐
   │  Scraper (tokio 后台周期任务)                  │
   │  └─ 每轮：JoinSet 有界并发采集所有 BMC          │
   │     └─ 每 BMC：HttpBmc → Collector 提取指标     │
   └────────────────────┬────────────────────────┘
                        ↓
              MetricsSnapshot（owned，原子替换）
                        │
        ┌───────────────┴───────────────┐
        │ axum HTTP server              │
        │  GET /metrics  → 当前快照      │
        │  GET /healthz → 存活/就绪      │
        └───────────────────────────────┘
```

### 组件（src/）

| 模块 | 职责 |
|---|---|
| `main.rs` | CLI（clap：`-c/--config`、`-p/--port`、`--log-level`）、启动装配、shutdown 编排 |
| `config.rs` | YAML 解析 + 全量校验（类型化配置；非法即退出，无静默 fallback） |
| `bmc.rs` | 每 BMC 的 `HttpBmc` 创建、basic/session 认证、TLS 选项、session 过期重登 |
| `scraper.rs` | 周期采集任务：每轮 JoinSet 有界并发，单 BMC 失败隔离，产出快照 |
| `registry.rs` | 快照存储：`RwLock<Option<MetricsSnapshot>>`，scrape 原子读 |
| `collector/` | 指标提取，按资源分模块：`sensors.rs` `thermal.rs` `power.rs` `processors.rs` `memory.rs` `storage.rs` `network.rs` `pcie.rs` `systems.rs` `health.rs`（通用健康/状态/info） |
| `http.rs` | axum 路由 `/metrics` `/healthz`；无鉴权（exporter 惯例，部署层控制） |

采集数据流（owned）：`collector` 各函数接收 nv-redfish 资源对象，返回 `Vec<Metric>`；快照组装为一个不可变 owned struct，用原子指针语义替换（`RwLock` 写时替换）。不引入 trait 抽象层除非有真实多态需求（无）。

## 6. 指标设计

### 通用指标

| 指标 | 说明 |
|---|---|
| `redfish_up{bmc}` | 1=采集成功，0=失败（含原因分类） |
| `redfish_scrape_duration_seconds{bmc}` | 该 BMC 单轮采集耗时 |
| `redfish_scrape_timestamp_seconds{bmc}` | 该 BMC 最近成功采集时间 |
| `redfish_scrape_error{bmc, resource}` | 1=该资源采集失败（逐资源隔离） |
| `redfish_health_status{bmc, resource_type, id, health, state}` | 0/1，统一编码所有资源健康/状态 |
| `redfish_info{bmc, key, value}` | 型号/序列号/固件版本等 key-value 信息 |

### 数值指标（label 示例）

- 传感器：`redfish_sensor_reading{bmc, resource, name, units, sensor_type, health}` + `redfish_sensor_threshold_upper_critical{...}` / `_upper_warning` / `_lower_warning` / `_lower_critical`
- 电源：`redfish_power_consumption_watts{bmc, chassis}`、`redfish_power_input_watts{bmc, chassis}`、`redfish_power_supply_output_watts{bmc, id}`
- 处理器：`redfish_processor_utilization_percent{bmc, system, id}`、`redfish_processor_temperature_celsius{bmc, id}`、`redfish_processor_power_watts{bmc, id}`
- 内存：`redfish_memory_capacity_bytes{bmc, system, id}`、`redfish_memory_bandwidth_percent{bmc, id}`
- 存储：`redfish_volume_capacity_bytes{bmc, storage, id}`、`redfish_drive_capacity_bytes{bmc, id}`、`redfish_drive_utilization_percent{bmc, id}`、`redfish_drive_predictive_failure{bmc, id}`
- 网络：`redfish_ethernet_interface_link_status{bmc, system, id}`（1=up）、`redfish_ethernet_interface_speed_mbps{bmc, id}`、`redfish_port_link_speed_gbps{bmc, id}`
- PCIe：`redfish_pcie_device_link_speed_gt_per_sec{bmc, id}`
- 系统：`redfish_power_state{bmc, system}`（On=1）

原则：数值不进 label 字符串；健康状态统一走 `redfish_health_status`；状态字符串用 enum 编码（非 Stringly typed）；单位进 label（`units`）或指标名（`_watts` 后缀），不两处都放。

## 7. 配置

```yaml
listen_addr: "0.0.0.0:9417"
scrape_interval: 30s            # 周期采集间隔
scrape_timeout: 15s             # 单轮总 deadline
request_timeout: 10s            # 单请求 timeout
max_concurrency: 4              # 每 BMC 并发请求上限（透传 http-extras）

bmcs:
  - name: bmc1
    host: https://10.0.0.1
    username: admin
    password: "redacted-in-logs"
    auth: basic                 # basic | session
    insecure_skip_verify: false # BMC 自签名证书时开启
    ca_cert_file: /path/ca.pem  # 可选：自定义 CA
```

校验：必填字段、URL 格式、auth 枚举、数值范围（interval>0 等）、bmc name 唯一；失败给出明确错误并退出。

## 8. 安全

- 100% safe code（`#![forbid(unsafe_code)]`）
- TLS：rustls + webpki-roots 默认；支持自定义 CA 文件与 `insecure_skip_verify`（显式危险选项，配置注释警示）
- Secret：密码类型化（`SecretString` 语义），禁止 Debug 派生、日志、错误上下文；tracing 字段手动过滤
- 输入校验：配置全部校验；BMC 响应解析失败只影响该资源（`redfish_scrape_error`），不 panic
- 错误不携带密码/token；不记录完整请求头

## 9. 并发与生命周期

- 主 runtime：tokio multi-thread
- Scraper 任务：main 持有 `JoinHandle`；每轮用 `JoinSet` 有界并发（BMC 数即上界），单轮结束等所有子任务完成或 deadline 超时
- 采集结果快照：每轮成功/部分失败后整体替换 `RwLock` 快照；锁内不做网络 I/O
- Shutdown：`tokio::signal`（SIGTERM/SIGINT）→ cancel 当前轮等待（deadline）→ drop 快照 → server graceful 关闭 → 退出码 0
- 认证 session：每 BMC 独立 session token；401 时重新登录并重试一次该轮；重登失败记错误指标

## 10. 测试策略

| 层级 | 内容 |
|---|---|
| 单元 | config 解析/校验（合法+非法样例）、指标转换（阈值编码、健康状态编码、数值解析）、label 组装 |
| 集成 | `nv-redfish-bmc-mock`（官方测试 BMC）模拟各资源，断言 `/metrics` 输出文本；认证失败、资源缺失、超时错误路径 |
| 行为 | 周期性采集（可控 clock/短间隔+事件等待，禁止 sleep 同步）、shutdown 触发、单 BMC 失败不影响其他 |
| 质量门禁 | `cargo fmt --check`、`cargo check`、`cargo clippy -D warnings`、`cargo test`、`cargo doc`；依赖安全（cargo-deny：漏洞/许可证/未维护）+ `cargo audit`（CI 内） |

## 11. CI 与发布

GitHub Actions：

1. `lint-test`（stable 1.90.0）：fmt + clippy + test + doc
2. `audit`：cargo-audit + cargo-deny（license、advisory、source）
3. `release-build`：tag/v0.1.0 触发 → `cross` 或 musl 容器构建 `x86_64-unknown-linux-musl` 静态二进制 → Docker 镜像（多阶段：builder 构建 → `gcr.io/distroless/static` 运行，非 root UID）
4. 产物：静态二进制 tarball + 镜像（含 SBOM 或至少版本标注）

Dockerfile：多阶段、`USER 65532`、无 shell（distroless）、镜像仅含二进制。

## 12. 文档交付物

- `README.md`：项目简介、快速开始、配置说明、指标表概览、部署（Docker/K8s/Prometheus 接入）
- `docs/metrics.md`：全量指标参考（指标、label、类型、说明）；B 类功能记录（为何不实现，0.2+ 方向）
- `docs/design.md`：架构说明（§5 展开）、生命周期、错误模型
- `config.example.yaml`：带注释的完整配置样例
- `deploy/`：prometheus 告警规则样例、grafana dashboard JSON、kubernetes（Deployment + ServiceMonitor）样例

## 13. 验收标准（0.1.0）

- [ ] `cargo clippy -D warnings`、`cargo test`、`cargo fmt --check` 全绿；无 unsafe（`cargo geiger` 或等效检查）
- [ ] 集成测试覆盖主要资源类别与错误路径
- [ ] 静态二进制：`file` 输出 "statically linked"，`ldd` 无输出（Linux）
- [ ] Docker 镜像非 root 运行；`/metrics` 与 `/healthz` 正常
- [ ] 文档齐全（README / metrics / design / 部署样例）
- [ ] 无 panic 路径（error 路径全 Result）；graceful shutdown 退出码 0
