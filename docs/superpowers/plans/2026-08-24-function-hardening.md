# 功能专项 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 0.1.0 功能专项（五维第 5 项，最后一项）——真机探测门控的指标覆盖补齐：CPU 频率/电压、存储控制器明细、驱动器 OEM 字段、系统/机箱 LED，共 4 个实现任务 + 1 个收尾任务。

**Architecture:** 纯增量指标：collector 按既有模块扩展（processors.rs / storage.rs / systems.rs），新指标族只追加不修改既有族；OEM 字段经 nv-redfish 0.15 `oem` feature 的 `oem_value` 通用 JSON 导航读取（不硬编码厂商分支）；mock e2e 测试经 tests/common 期望集注入 Oem JSON 载荷。

**Tech Stack:** Rust 1.90 / edition 2024、nv-redfish 0.15（启用既有 `oem` feature——同 crate feature 开关，零新依赖）、既有 mock 测试基建。

**Spec:** `docs/superpowers/specs/2026-08-24-function-hardening-design.md`（计划依 spec 论证，执行者须先读 spec）

**探测门控结论（真机 2026-08-24 实测，计划前置输入）：** `%TEMP%\opencode\redfish-func-probe\probe-summary.md` 为字段路径权威——执行者必须先读该文件与对应原始 JSON（`%TEMP%\opencode\redfish-func-probe\*.json`）确定精确 JSON 路径。

| 候选 | 门控 | 实现任务 |
|---|---|---|
| CPU 频率（current/max） | MET（Dell MaxSpeedMHz=4000/CurrentClockSpeedMhz=2100；浪潮 MaxSpeedMHz=3400/Oem.Public.FrequencyMHz=2100） | Task 1 |
| CPU 电压 | MET（仅 Dell：Oem.Dell.DellProcessor.Volts="1.6" 字符串） | Task 1 |
| 存储控制器明细 | MET（仅 Dell：Model/FirmwareVersion/Status） | Task 2 |
| 驱动器 OEM（WWN/RaidStatus/PowerStatus） | MET（仅 Dell；浪潮盘字段全 null、存储子树 500/17034） | Task 3 |
| 系统/机箱 LED | MET（两家 Systems；Dell 主机箱；Dell 背板机箱无字段） | Task 4 |
| RAID 电池 / 驱动器寿命 / PPID / CPU 级功耗 | NOT MET（null/404） | backlog（验收记录登记） |

## Global Constraints

- rust-version 1.90、edition 2024；`#![forbid(unsafe_code)]` 在 src/ 必须保持
- **零新依赖**：仅启用 nv-redfish 既有 `oem` feature（Cargo.toml features 追加 `"oem"`；同一 crate，非新依赖）
- **语义冻结（spec §8）**：既有 18 指标族只增不改——不修改任何既有族的名称/help/labels/值语义；新族只追加
- 每任务结束 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets` 全绿后才提交
- 代码注释用中文（仓库惯例）；提交信息用英文、仓库风格（`feat:`/`test:`/`docs:`）
- **诚实记录**：验收记录每个数字须有制品引用；真机不可达/字段 null 如实记录
- 全维回归（spec §6）：安全四套件（config/http/bmc/pagination）+ 稳定套件（stability/scraper/integration/soak 基建）+ 性能基准 perf_test（round/encode/hot_path ±5% 容差；**bytes 因新族增长，Task 5 实测新值记为新基线并声明**）+ 资源 resource_test（系数在既有范围）
- 探测制品路径：`%TEMP%\opencode\redfish-func-probe\`（probe-summary.md + 69 个原始 JSON）

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `Cargo.toml` | 改 | nv-redfish features 追加 `"oem"`（Task 1） |
| `src/metrics.rs` | 改 | 新指标族常量（name/help 对） |
| `src/collector/processors.rs` | 改 | CPU 频率/电压指标 |
| `src/collector/storage.rs` | 改 | 存储控制器 + 驱动器 OEM 指标 |
| `src/collector/systems.rs` | 改 | 系统/机箱 LED 指标 |
| `tests/common/mod.rs` | 改 | 期望集扩展（Oem JSON / MaxSpeedMHz / IndicatorLED / StorageControllers） |
| `tests/metrics_test.rs` 或 `tests/integration_test.rs` | 改 | 新指标断言 |
| `docs/metrics.md` | 改 | 新指标族条目（Task 5） |
| `docs/audit/2026-08-24-function.md` | 新建 | 验收记录（Task 5） |

---

### Task 1: 处理器频率/电压指标（oem feature + processors.rs）

**Files:**
- Modify: `Cargo.toml`（nv-redfish features 追加 `"oem"`）、`src/metrics.rs`、`src/collector/processors.rs`
- Test: `tests/common/mod.rs`（expect_processor_payloads 扩展）、`tests/metrics_test.rs`（或 integration_test，按既有测试落点）

**Interfaces:**
- Consumes: nv-redfish `oem` feature 的 `oem_value(oem, key)`（`nv_redfish::oem::oem_value`，oem 参数为 facade raw 类型的 `oem` 字段引用）；探测制品（字段路径权威）
- Produces: 新常量与指标（collector 层，无跨任务签名变化）：
  - `PROCESSOR_FREQUENCY = ("redfish_processor_frequency_mhz", "Current operating frequency of the processor in MHz")`
  - `PROCESSOR_MAX_FREQUENCY = ("redfish_processor_max_frequency_mhz", "Maximum rated frequency of the processor in MHz")`
  - `PROCESSOR_VOLTAGE = ("redfish_processor_voltage_volts", "Processor input voltage in volts (vendor OEM field when present)")`
  - labels 沿用 processors.rs 既有 push_value 惯例：`bmc`/`system`/`id`

- [ ] **Step 1: 读探测制品定路径**

Read `%TEMP%\opencode\redfish-func-probe\probe-summary.md` 与 Dell/浪潮 Processors 相关原始 JSON，确定：
- Dell 当前频率字段精确路径（候选：Processor 顶层 `Oem.Dell.DellProcessor.CurrentClockSpeedMhz`；probe 记录 OperatingSpeedMHz=2100 与 CurrentClockSpeedMhz=2100 并存——以 JSON 实际层级为准）
- 浪潮当前频率：`Oem.Public.FrequencyMHz` 精确路径
- MaxSpeedMHz 是否为标准顶层字段（两家 probe 均有 → 直接 `raw.max_speed_mhz`）
- Dell 电压：`Oem.Dell.DellProcessor.Volts`（字符串 "1.6"）
把精确路径记入任务报告。

- [ ] **Step 2: 写失败测试（mock 期望集扩展 + 断言）**

`tests/common/mod.rs` 的 `expect_processor_payloads`：CPU1 载荷追加标准字段与 Dell OEM（按 Step 1 路径；Oem 块加在 `"Metrics"` 旁）：

```rust
bmc.expect(Expect::get(
    "/redfish/v1/Systems/1/Processors/CPU1",
    json!({
        "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1",
        "Id": "CPU1", "Name": "CPU 1", "ProcessorType": "CPU",
        "Status": { "Health": "OK", "State": "Enabled" },
        "Manufacturer": "Intel", "Model": "Xeon Gold 6338",
        "MaxSpeedMHz": 3400,
        "Oem": {
            "Dell": { "DellProcessor": { "Volts": "1.6", "CurrentClockSpeedMhz": 2100 } },
            "Public": { "FrequencyMHz": 2100 }
        },
        "Metrics": { "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1/Metrics" },
    }),
));
```

（既有 CPU1 Metrics 载荷不动；新增一个 CPU2 载荷仅含 MaxSpeedMHz 无 Oem——验证缺失字段不发指标的分支。若 tests/common 的 `Members` 只列 CPU1，追加 CPU2 条目与两个期望。）

断言（`tests/metrics_test.rs` 或既有 e2e 落点，执行者按文件现状选；断言用 `encode(&registry)` 输出 `contains`）：

```rust
#[test]
fn processor_frequency_voltage_metrics() {
    // 构造 registry 注册 CPU1+CPU2 指标后 encode：
    let out = /* 走 mock 采集或直接 register_into（按既有测试模式） */;
    assert!(out.contains("redfish_processor_frequency_mhz{bmc=\"bmc1\",system=\"1\",id=\"CPU1\"} 2100"), "{out}");
    assert!(out.contains("redfish_processor_max_frequency_mhz{bmc=\"bmc1\",system=\"1\",id=\"CPU2\"} 3400"), "{out}");
    assert!(out.contains("redfish_processor_voltage_volts{bmc=\"bmc1\",system=\"1\",id=\"CPU1\"} 1.6"), "{out}");
    assert!(!out.contains("redfish_processor_voltage_volts{bmc=\"bmc1\",system=\"1\",id=\"CPU2\"}"), "CPU2 无 OEM 电压字段，不得产出电压指标");
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: 该测试
Expected: FAIL（新常量不存在 / 采集不产出）

- [ ] **Step 4: 实现**

`Cargo.toml` nv-redfish features 追加 `"oem"`。

`src/metrics.rs` 追加三组常量（Task 1 Interfaces 中的值，verbatim）。

`src/collector/processors.rs`：`collect_processor` 中在 `push_health` 之后追加（`raw` 为 `processor.raw()`）：

```rust
    // 频率：MaxSpeedMHz 为标准字段；当前频率取厂商 OEM 字段（Dell CurrentClockSpeedMhz / 浪潮 Public.FrequencyMHz），
    // 缺失时该指标不产出（真机探测：Dell 2100 / 浪潮 2100）。
    push_value(
        out,
        bmc_name,
        system_id,
        &id,
        PROCESSOR_MAX_FREQUENCY,
        raw.max_speed_mhz.flatten().map(|v| v as f64),
    );
    if let Some(oem) = raw.oem.as_ref() {
        // 当前频率：Dell 优先，浪潮 Public 兜底（以探测制品路径为准，此处为示意层级）
        let freq = nv_redfish::oem::oem_value(oem, "Dell")
            .and_then(|d| d.get("DellProcessor"))
            .and_then(|p| p.get("CurrentClockSpeedMhz"))
            .and_then(|v| v.as_f64())
            .or_else(|| {
                nv_redfish::oem::oem_value(oem, "Public")
                    .and_then(|p| p.get("FrequencyMHz"))
                    .and_then(|v| v.as_f64())
            });
        push_value(out, bmc_name, system_id, &id, PROCESSOR_FREQUENCY, freq);
        // 电压：Dell DellProcessor.Volts 为字符串，解析失败则跳过（不报错、不产出）
        let volts = nv_redfish::oem::oem_value(oem, "Dell")
            .and_then(|d| d.get("DellProcessor"))
            .and_then(|p| p.get("Volts"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok());
        push_value(out, bmc_name, system_id, &id, PROCESSOR_VOLTAGE, volts);
    }
```

（若 facade ProcessorRaw 无 `oem` 字段或路径层级与示意不同，以 nv-redfish 0.15 实际类型与探测 JSON 为准调整；`oem_value` 签名 `oem_value(oem: &ResourceOemSchema, key: &str) -> Option<&Value>`。）

- [ ] **Step 5: 运行测试确认通过**

Run: 该测试 + `cargo test --test metrics_test`（如落点不同则对应文件）
Expected: PASS

- [ ] **Step 6: 全量门禁与提交**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`
Expected: 全绿

```powershell
git add Cargo.toml src/metrics.rs src/collector/processors.rs tests/common/mod.rs <test file>
git commit -m "feat: processor frequency and voltage metrics (Dell/Inspur OEM fields)"
```

---

### Task 2: 存储控制器指标（storage.rs，慢组）

**Files:**
- Modify: `src/metrics.rs`、`src/collector/storage.rs`
- Test: `tests/common/mod.rs`（expect_storage_payloads 扩展）、既有 e2e 断言文件

**Interfaces:**
- Consumes: Task 1 的 oem feature（如需读 DellControllers OEM 字段；标准 StorageControllers 字段不需要）；探测制品（Dell H-storage JSON 路径）
- Produces:
  - `STORAGE_CONTROLLER_INFO = ("redfish_storage_controller_info", "Storage controller model and firmware information, 1 = present")`
  - `STORAGE_CONTROLLER_STATUS = ("redfish_storage_controller_status", "Storage controller status, 1 = present with status label")`
  - labels 沿用 storage.rs 既有 push_value 惯例：`bmc`/`system`/`storage`/`id`；info 族追加 `model`/`firmware_version` label；status 族追加 `status` label

- [ ] **Step 1: 读探测制品定路径**

Read `%TEMP%\opencode\redfish-func-probe\probe-summary.md` 与 Dell Storage 原始 JSON，确定 StorageControllers 数组的标准字段（Model/FirmwareVersion/Status.State）与每成员 Id/MemberId 的实际字段名。记入报告。

- [ ] **Step 2: 写失败测试**

`tests/common/mod.rs` 的 `expect_storage_payloads`：Storage/SATA1 载荷追加：

```rust
"StorageControllers": [{
    "MemberId": "0",
    "Model": "PERC H755",
    "FirmwareVersion": "52.26.0-5019",
    "Status": { "Health": "OK", "State": "Enabled" }
}],
```

断言（e2e 既有落点）：

```rust
assert!(out.contains("redfish_storage_controller_info{bmc=\"bmc1\",system=\"1\",storage=\"SATA1\",id=\"0\",model=\"PERC H755\",firmware_version=\"52.26.0-5019\"} 1"), "{out}");
assert!(out.contains("redfish_storage_controller_status{bmc=\"bmc1\",system=\"1\",storage=\"SATA1\",id=\"0\",status=\"Enabled\"} 1"), "{out}");
```

- [ ] **Step 3: 运行测试确认失败**

Expected: FAIL

- [ ] **Step 4: 实现**

`src/metrics.rs` 追加两组常量（verbatim）。

`src/collector/storage.rs`：`collect_storage_controller` 中在 drives/volumes 之前追加（`raw` 为 `storage.raw()`；StorageControllers 为标准字段，若 facade 无类型化访问则经 raw JSON 导航——以 nv 0.15 实际类型为准）：

```rust
    // 存储控制器明细（标准 StorageControllers 数组；真机仅 Dell 有，浪潮存储子树损坏不产出）
    for (i, c) in raw.storage_controllers.iter().enumerate() {
        let id = /* c.MemberId 或 c.Id，缺省用 i.to_string() */;
        let model = c.model.clone().flatten();
        let fw = c.firmware_version.clone().flatten();
        if let Some(m) = model {
            out.push(
                Metric::gauge(STORAGE_CONTROLLER_INFO.0, STORAGE_CONTROLLER_INFO.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.to_string())
                    .label("storage", storage_id.clone())
                    .label("id", id.clone())
                    .label("model", m)
                    .label("firmware_version", fw.unwrap_or_default())
                    .build(1.0),
            );
        }
        if let Some(st) = c.status.as_ref().and_then(|s| s.state.as_ref()).map(|s| s.to_string()) {
            out.push(
                Metric::gauge(STORAGE_CONTROLLER_STATUS.0, STORAGE_CONTROLLER_STATUS.1)
                    .label("bmc", bmc_name.to_string())
                    .label("system", system_id.to_string())
                    .label("storage", storage_id.clone())
                    .label("id", id)
                    .label("status", st)
                    .build(1.0),
            );
        }
    }
```

（`raw.storage_controllers` 类型按 nv facade 实际字段名调整；无该字段则经 `raw.oem`/JSON 导航，路径以探测制品为准。）

- [ ] **Step 5: 运行测试确认通过** → **Step 6: 全量门禁与提交**

```powershell
git add src/metrics.rs src/collector/storage.rs tests/common/mod.rs <test file>
git commit -m "feat: storage controller info and status metrics"
```

---

### Task 3: 驱动器 OEM 指标（storage.rs，慢组）

**Files:**
- Modify: `src/metrics.rs`、`src/collector/storage.rs`
- Test: `tests/common/mod.rs`（Drive 载荷扩展）、既有 e2e 断言文件

**Interfaces:**
- Consumes: Task 1 的 oem feature + `oem_value`；探测制品（Dell Drives JSON 的 Oem.Dell.DellPhysicalDisk 路径）
- Produces:
  - `DRIVE_INFO = ("redfish_drive_info", "Drive vendor identifier information, 1 = present")`（labels：`bmc`/`system`/`storage`/`id`/`wwn`）
  - `DRIVE_OEM_STATUS = ("redfish_drive_oem_status", "Drive vendor OEM status, 1 = present with status labels")`（labels：`bmc`/`system`/`storage`/`id`/`raid_status`/`power_status`）

- [ ] **Step 1: 读探测制品定路径**

Read probe-summary.md 与 Dell Drives 原始 JSON：`Oem.Dell.DellPhysicalDisk.{WWN, RaidStatus, PowerStatus, PredictedMediaLifeLeftPercent}` 精确层级与字段类型。记入报告。

- [ ] **Step 2: 写失败测试**

`tests/common/mod.rs` 的 Drive/HDD1 载荷追加：

```rust
"Oem": { "Dell": { "DellPhysicalDisk": {
    "WWN": "3F4EE0803B522508",
    "RaidStatus": "Online",
    "PowerStatus": "On",
    "PredictedMediaLifeLeftPercent": null
} } },
```

断言：

```rust
assert!(out.contains("redfish_drive_info{bmc=\"bmc1\",system=\"1\",storage=\"SATA1\",id=\"HDD1\",wwn=\"3F4EE0803B522508\"} 1"), "{out}");
assert!(out.contains("redfish_drive_oem_status{bmc=\"bmc1\",system=\"1\",storage=\"SATA1\",id=\"HDD1\",raid_status=\"Online\",power_status=\"On\"} 1"), "{out}");
assert!(!out.contains("PredictedMediaLifeLeftPercent"), "寿命字段 null 不得产出（NOT MET backlog）");
```

- [ ] **Step 3: 运行测试确认失败** → **Step 4: 实现**

`src/metrics.rs` 追加两组常量（verbatim）。

`src/collector/storage.rs` `collect_drive` 中在 `push_health` 之后追加：

```rust
    // 驱动器 OEM 字段（Dell DellPhysicalDisk；真机寿命字段 null 不采集，浪潮盘字段全 null 不产出）
    if let Some(oem) = raw.oem.as_ref() {
        if let Some(dell) = nv_redfish::oem::oem_value(oem, "Dell")
            .and_then(|d| d.get("DellPhysicalDisk"))
        {
            if let Some(wwn) = dell.get("WWN").and_then(|v| v.as_str()).map(str::to_string) {
                out.push(
                    Metric::gauge(DRIVE_INFO.0, DRIVE_INFO.1)
                        .label("bmc", bmc_name.to_string())
                        .label("system", system_id.to_string())
                        .label("storage", storage_id.to_string())
                        .label("id", id.clone())
                        .label("wwn", wwn)
                        .build(1.0),
                );
            }
            let raid = dell.get("RaidStatus").and_then(|v| v.as_str()).unwrap_or_default();
            let power = dell.get("PowerStatus").and_then(|v| v.as_str()).unwrap_or_default();
            if !raid.is_empty() || !power.is_empty() {
                out.push(
                    Metric::gauge(DRIVE_OEM_STATUS.0, DRIVE_OEM_STATUS.1)
                        .label("bmc", bmc_name.to_string())
                        .label("system", system_id.to_string())
                        .label("storage", storage_id.to_string())
                        .label("id", id)
                        .label("raid_status", raid.to_string())
                        .label("power_status", power.to_string())
                        .build(1.0),
                );
            }
        }
    }
```

（`raw.oem` 字段名与层级以 nv 0.15 实际类型与探测 JSON 为准调整。）

- [ ] **Step 5: 运行测试确认通过** → **Step 6: 全量门禁与提交**

```powershell
git add src/metrics.rs src/collector/storage.rs tests/common/mod.rs <test file>
git commit -m "feat: drive OEM identifier and status metrics (Dell DellPhysicalDisk)"
```

---

### Task 4: 系统/机箱 LED 指标（systems.rs，快组）

**Files:**
- Modify: `src/metrics.rs`、`src/collector/systems.rs`
- Test: `tests/common/mod.rs`（expect_system 与 expect_chassis_round 载荷扩展）、既有 e2e 断言文件

**Interfaces:**
- Consumes: 探测制品（IndicatorLED 字段位置：Systems/{id} 与 Chassis/{id} 顶层）
- Produces:
  - `INDICATOR_LED = ("redfish_indicator_led", "Indicator LED state, 1 = present with state label")`（labels：`bmc`/`resource_type`（`system`|`chassis`）/`id`/`state`）

- [ ] **Step 1: 读探测制品定路径**

Read probe-summary.md 与两家 Systems/Chassis JSON：IndicatorLED 为顶层字段（Dell Systems "Lit"、浪潮 "Off"；Dell 主机箱 "Lit"、Dell 背板机箱无字段）。记入报告。

- [ ] **Step 2: 写失败测试**

`tests/common/mod.rs`：`expect_system` 的 System 载荷追加 `"IndicatorLED": "Lit"`；`expect_chassis_round` 的主机箱载荷追加 `"IndicatorLED": "Lit"`（背板机箱不加——验证缺失分支）。

断言：

```rust
assert!(out.contains("redfish_indicator_led{bmc=\"bmc1\",resource_type=\"system\",id=\"1\",state=\"Lit\"} 1"), "{out}");
assert!(out.contains("redfish_indicator_led{bmc=\"bmc1\",resource_type=\"chassis\",id=\"<主机箱id>\",state=\"Lit\"} 1"), "{out}");
assert!(!out.contains("redfish_indicator_led{bmc=\"bmc1\",resource_type=\"chassis\",id=\"<背板机箱id>\""), "无 IndicatorLED 的机箱不得产出");
```

- [ ] **Step 3: 运行测试确认失败** → **Step 4: 实现**

`src/metrics.rs` 追加一组常量（verbatim）。

`src/collector/systems.rs`：
- 系统 LED：在 systems 遍历（`push_info` 附近）追加——`raw.indicator_led`（Option 枚举/字符串，按 facade 实际类型）非空时：

```rust
    if let Some(led) = raw.indicator_led.clone().flatten() {
        out.push(
            Metric::gauge(INDICATOR_LED.0, INDICATOR_LED.1)
                .label("bmc", bmc_name.to_string())
                .label("resource_type", "system".to_string())
                .label("id", system_id.clone())
                .label("state", format!("{led:?}"))
                .build(1.0),
        );
    }
```

- 机箱 LED：`collect_chassis_health` 的 chassis 遍历（`push_health` 之后）同样处理，`resource_type` = `"chassis"`、id = chassis_id。

（IndicatorLED 的 facade 类型为枚举时用 `format!("{led:?}")`；为字符串时直用。以 nv 0.15 实际类型为准。）

- [ ] **Step 5: 运行测试确认通过** → **Step 6: 全量门禁与提交**

```powershell
git add src/metrics.rs src/collector/systems.rs tests/common/mod.rs <test file>
git commit -m "feat: system and chassis indicator LED metrics"
```

---

### Task 5: 文档 + 真机验证 + 验收记录 + 全量回归

**Files:**
- Modify: `docs/metrics.md`（新指标族条目）
- Create: `docs/audit/2026-08-24-function.md`
- 无代码改动（真机验证用已提交代码构建）

- [ ] **Step 1: docs/metrics.md 新增族条目**

按既有表格格式追加 8 个新族（Task 1-4 Interfaces 中的名称/help/labels/来源字段 verbatim）。

- [ ] **Step 2: 真机实跑验证**

- `cargo build --release`；用验收配置（`%TEMP%\opencode\redfish-audit\acceptance-config.yaml` 或 perf 配置，复制到 `%TEMP%\opencode\redfish-func-run\` 运行；显式 `listen_addr` 便于本机抓取）
- 等待 ≥2 个全量轮后抓取 /metrics：断言新族出现且数值与 probe-summary.md 一致（Dell：frequency 2100/max 4000、volts 1.6、控制器 PERC H755、盘 WWN 3F4EE0803B522508、LED Lit；浪潮：frequency 2100/max 3400、LED Off、无 volts/控制器/盘 OEM）
- 数值不符或族缺失：如实记录，不得篡改预期
- 制品落 `%TEMP%\opencode\redfish-func-run\`（metrics 抓取 + 日志）；真机不可达则如实记录原因

- [ ] **Step 3: 撰写验收记录 `docs/audit/2026-08-24-function.md`**

```markdown
# 0.1.0 功能专项验收记录

日期：2026-08-24
依据：spec `docs/superpowers/specs/2026-08-24-function-hardening-design.md` §10 验收标准
范围：五维专项第 5 项（功能）；不破坏第 1-4 项（安全/稳定/性能/资源）成果

## 结论
**PASS / CONDITIONAL PASS / FAIL**（执行者按证据填写）

## 决策记录
D1 指标覆盖补齐 / D2 全清单探测门控 / D3 仓库惯例 / D4 mock+真机验证

## 探测门控结论（S1）
| 候选字段 | 门控 | 证据 |
|---|---|---|
| （probe-summary.md 全表；NOT MET 项全部登记 backlog） | | |

## 验收标准对照（S1-S5）
| 标准 | 结果 | 证据 |
|---|---|---|

## 新指标族（8 个）
（Task 1-4 清单：名称/labels/来源字段/厂商覆盖）

## 真机验证（S3）
| 族 | Dell 实测 | 浪潮实测 | 与探测一致性 |
|---|---|---|---|

## 回归（S4）
安全四套件 + 稳定套件 + perf_test（round/encode/hot_path ±5%；bytes 新基线 {新值}，旧基线 8896 因新族增长，声明：既有序列字节不变、总量增长）+ resource_test

## 遗留跟踪
- NOT MET 项：RAID 电池（404）、驱动器寿命（null）、PPID（null）、CPU 级功耗（无 Metrics）、浪潮盘/控制器（存储子树 500/17034 固件缺陷）
- P1 可靠性 backlog（AUDIT-1/3、2/4/8）维持
```

- [ ] **Step 4: 全量回归**

Run: `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test --all-targets`; `cargo audit`; `cargo deny check`
点名：安全四套件、稳定套件（stability/scraper/integration + soak --no-run）、`cargo test --release --test perf_test -- --ignored --nocapture`（round/encode/hot_path 与基线 0.33/0.06/0.0005 ±5%；bytes 记新基线）、`cargo test --release --test resource_test -- --ignored --nocapture`
Expected: 全绿 + perf 容差内 + resource 系数在既有范围

- [ ] **Step 5: 提交**

```powershell
git add docs/metrics.md docs/audit/2026-08-24-function.md
git commit -m "docs: 0.1.0 function hardening acceptance record"
```

---

## Self-Review 记录

- **Spec 覆盖**：§3 探测（计划前置已执行，Task 5 S1 转述）、§4 指标约定（Task 1-4）、§5.1 mock（Task 1-4 期望集扩展）、§5.2 真机（Task 5）、§6 回归（Task 5）、§7 文档（Task 5）、§10 验收（Task 5）、§8 明确不做（NOT MET backlog 登记于 Task 5）。
- **占位符**：无 TBD；`<主机箱id>`/`<背板机箱id>` 为执行者按 tests/common 既有机箱 id 填写（执行者读文件即得）；字段路径类指令均指向探测制品为权威并给出示意代码。
- **类型一致性**：8 个新族常量在 Task 1-4 Interfaces 定义、Task 5 文档转述一致；labels 沿用各 collector 既有 push_value/push_health 惯例；`oem_value` 签名（`&ResourceOemSchema, &str → Option<&Value>`）与实际一致（Task 1 起启用 oem feature，Task 3 复用）。
- **依赖链核查**：仅启用 nv-redfish 既有 `oem` feature（零新依赖，spec §11）；语义冻结——新族只追加、既有族不改；perf bytes 基线变化已提前声明（Task 5 Step 4）。
