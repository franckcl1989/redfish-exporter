# 工程质量审计与改进 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在三台（两台真机 + mock）环境下完成对标审计并实现确认的工程改进：分频采集、事件日志、BIOS、内存纠错、PSU 明细、自观测、会话清理、404 分类，以及按审计结论门控的分页/运维端点/OEM 指标。

**Architecture:** 审计阶段用 Python 标准库探测脚本对两台真机 BMC 建 API 基线（快照 + 报告），再实跑 exporter 交叉对比产出缺陷清单；实现阶段全部走 TDD，改动限于 exporter 编排层与 collector 层，nv-redfish 仅做特性开关的增量启用（`log-services`、`bios`），不 fork 不改源码不换依赖。

**Tech Stack:** Rust 2024 / tokio / axum / prometheus / nv-redfish 0.15.1（+log-services、bios 特性）/ Python 3.12（审计脚本）/ PowerShell（Windows 执行环境）。

**Spec:** `docs/superpowers/specs/2026-08-21-engineering-quality-audit-design.md`（v2 + D5 修正）

## Global Constraints

- **nv 门禁**：nv-redfish 已提供的能力直接使用；只允许在 Cargo.toml 中**增量启用**特性（`log-services`、`bios`），禁止 fork/修改 nv 源码、禁止替换核心抓取管线中的 nv 类型。
- **无新依赖**：本计划不引入任何新的第三方 crate。`time` crate 不得作为直接依赖（用 `TryFrom<EdmDateTimeOffset> for std::time::SystemTime`）。
- **凭据纪律**：真机凭据只存在于 shell 环境变量与 `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\`（gitignore 之外，不在仓库内）。任何文件不得含真实密码；日志输出不得含密码（沿用 `SecretString`）。
- **只读**：对真机只做 GET；不 POST/PATCH/DELETE 任何真实资源（会话建立探测除外，仅限 A 组矩阵，且在探测脚本内使用临时会话）。
- **工程纪律**：每任务结束 `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test` 全绿；`#![forbid(unsafe_code)]` 保持。
- **指标命名**：遵循 `docs/metrics.md` 现有约定（单位进名字、静态信息用 `redfish_info`、健康用 `redfish_health_status`）。
- **审计产物路径**：`docs/audit/2026-08-21-bmc-audit.md`、`docs/audit/2026-08-21-gap-analysis.md`。
- 真机地址：`https://10.10.90.70/`（Dell R750，用户 root）；`https://10.10.90.80/`（浪潮，用户 admin）。

---

# 阶段一：审计准备与探测

### Task 1: 审计环境与探测脚本骨架

**Files:**
- Create: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py`（一次性工具，不入仓库）
- Create: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\creds.json`（凭据文件，不入仓库）
- Create: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\out\`（快照输出目录）

**Interfaces:**
- Produces: `probe.py` 提供 `fetch(url) -> (status, ms, headers, json|None)` 与 `snapshot(group, bmc, path, data)` 两个核心函数，供 Task 2/3 使用；`creds.json` 形如 `{"dells": {"host": "...", "user": "...", "pass": "..."}, "inspur": {...}}`。

- [ ] **Step 1: 确认连通性**

运行（PowerShell，凭据用环境变量，不落任何仓库文件）：
```powershell
$env:AUDIT_DELL_PASS = '<REDACTED-PASSWORD>'; $env:AUDIT_INSPUR_PASS = '<REDACTED-PASSWORD>'
$r = Invoke-WebRequest -Uri 'https://10.10.90.70/redfish/v1' -Headers @{Authorization='Basic ' + [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes("root:$env:AUDIT_DELL_PASS"))} -SkipCertificateCheck -TimeoutSec 15
$r.StatusCode; $r.Content.Substring(0, [Math]::Min(300, $r.Content.Length))
```
预期：Dell 返回 200 与 ServiceRoot JSON。浪潮同法（用户 admin）。若不通，停止并报告网络/凭据问题（先解决再继续）。

- [ ] **Step 2: 写凭据文件**

创建 `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\creds.json`：
```json
{"dells": {"host": "https://10.10.90.70", "user": "root", "pass": "<REDACTED-PASSWORD>"},
 "inspur": {"host": "https://10.10.90.80", "user": "admin", "pass": "<REDACTED-PASSWORD>"}}
```
确认该路径不在 git 仓库内（`git status` 无新文件）。

- [ ] **Step 3: 写探测脚本骨架**

`probe.py`（Python 3.12 标准库，完整内容）：
```python
import json, ssl, sys, time, urllib.request, os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")

def client():
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx

def fetch(bmc, url):
    ctx = client()
    req = urllib.request.Request(bmc["host"] + url, headers={
        "Authorization": "Basic " + __import__("base64").b64encode(
            f'{bmc["user"]}:{bmc["pass"]}'.encode()).decode(),
        "Accept": "application/json",
    })
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=15, context=ctx) as resp:
            body = resp.read()
            headers = {k.lower(): v for k, v in resp.headers.items()}
            data = None
            if body:
                try: data = json.loads(body)
                except Exception: data = None
            return {"url": url, "status": resp.status, "ms": round((time.time()-t0)*1000),
                    "headers": headers, "json": data}
    except urllib.error.HTTPError as e:
        return {"url": url, "status": e.code, "ms": round((time.time()-t0)*1000),
                "headers": {k.lower(): v for k, v in e.headers.items()}, "json": None}
    except Exception as e:
        return {"url": url, "status": -1, "ms": round((time.time()-t0)*1000),
                "headers": {}, "json": str(e)}

def save(bmc_name, group, result):
    os.makedirs(os.path.join(OUT, bmc_name), exist_ok=True)
    safe = group.replace("/", "_")
    with open(os.path.join(OUT, bmc_name, f"{safe}.json"), "w", encoding="utf-8") as f:
        json.dump(result, f, indent=1, ensure_ascii=False)

def run(bmc_name, bmc, groups):
    for g in groups:
        r = g(bmc)
        save(bmc_name, r["group"], r)

if __name__ == "__main__":
    creds = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "creds.json")))
    # 子命令：python probe.py matrix-a|matrix-b|... ；Task 2 传入 A-F，Task 3 传入 G-L
    which = sys.argv[1]
    groups = {
        "A": [g_auth_session], "B": [g_systems], "C": [g_chassis],
        "D": [g_managers], "E": [g_update], "F": [g_power],
        "G": [g_sensors], "H": [g_storage], "I": [g_network],
        "J": [g_pagination], "K": [g_latency], "L": [g_oem],
    }[which]
    for name, bmc in creds.items():
        run(name, bmc, groups)
    print("done", which)
```
占位组函数（A-L）由 Task 2/3 填充；先写 12 个空函数体（`def g_x(bmc): return {"group": "x", "results": []}`）保证脚本可运行。

- [ ] **Step 4: 冒烟运行**

```powershell
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" A
```
预期：输出 `done A`，`out\dells\A.json` 与 `out\inspur\A.json` 生成（当前为空 results 数组）。

- [ ] **Step 5: 提交（仅仓库内产物）**

本任务无仓库内文件，跳过 commit；在 Task 6 一并提交审计报告。

---

### Task 2: 探测矩阵 A–F 实现与执行

**Files:**
- Modify: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py`（填充 g_auth_session, g_systems, g_chassis, g_managers, g_update, g_power）
- Create: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\out\dells\A-F.json`、`out\inspur\A-F.json`

**Interfaces:**
- Consumes: `fetch(bmc, url) -> dict`, `save(bmc_name, group, result)`（Task 1）。
- Produces: 每 BMC 6 份快照 JSON（A–F 组），供 Task 3 汇总与 Task 5 报告引用。

- [ ] **Step 1: 实现 A 组（认证与会话）**

```python
def g_auth_session(bmc):
    out = {"group": "A-auth-session", "results": []}
    out["results"].append(fetch(bmc, "/redfish/v1"))                 # basic 根
    out["results"].append(fetch(bmc, "/redfish/v1/SessionService"))  # basic SessionService
    out["results"].append(fetch(bmc, "/redfish/v1/SessionService/Sessions"))  # 集合
    out["results"].append(fetch(bmc, "/redfish/v1/SessionService/Sessions/1"))  # 不存在项: 404 行为
    return out
```
（会话 POST 属写操作，仅作为只读探测的例外，放在 L 组由人工确认后再执行 —— 默认本组不做 POST。）

- [ ] **Step 2: 实现 B 组（Systems）**

```python
def g_systems(bmc):
    out = {"group": "B-systems", "results": []}
    root = fetch(bmc, "/redfish/v1")
    out["results"].append(root)
    sys_url = None
    links = root.get("json") or {}
    if links.get("Systems"):
        sys_url = links["Systems"].get("@odata.id")
    if not sys_url: return out
    coll = fetch(bmc, sys_url)
    out["results"].append(coll)
    members = ((coll.get("json") or {}).get("Members") or [])
    for m in members[:4]:
        su = m.get("@odata.id")
        out["results"].append(fetch(bmc, su))                       # System 本体
        s = (fetch(bmc, su).get("json") or {})
        if s.get("Bios"):   out["results"].append(fetch(bmc, s["Bios"]["@odata.id"]))
        if s.get("Memory"):
            out["results"].append(fetch(bmc, s["Memory"]["@odata.id"]))
            mc = (fetch(bmc, s["Memory"]["@odata.id"]).get("json") or {})
            for mm in (mc.get("Members") or [])[:4]:
                murl = mm["@odata.id"]
                out["results"].append(fetch(bmc, murl))
                mj = (fetch(bmc, murl).get("json") or {})
                if mj.get("Metrics"):
                    out["results"].append(fetch(bmc, mj["Metrics"]["@odata.id"]))
        if s.get("Processors"):
            out["results"].append(fetch(bmc, s["Processors"]["@odata.id"]))
        if s.get("EthernetInterfaces"):
            out["results"].append(fetch(bmc, s["EthernetInterfaces"]["@odata.id"]))
    return out
```

- [ ] **Step 3: 实现 C 组（Chassis 双路径存在性）**

```python
def g_chassis(bmc):
    out = {"group": "C-chassis", "results": []}
    root = fetch(bmc, "/redfish/v1")
    ch_url = None
    if root.get("json") and root["json"].get("Chassis"):
        ch_url = root["json"]["Chassis"]["@odata.id"]
    if not ch_url: return out
    coll = fetch(bmc, ch_url); out["results"].append(coll)
    for m in ((coll.get("json") or {}).get("Members") or []):
        cu = m["@odata.id"]
        c = fetch(bmc, cu); out["results"].append(c)
        j = c.get("json") or {}
        for key in ("Thermal", "Power", "PowerSubsystem", "ThermalSubsystem",
                    "Sensors", "EnvironmentMetrics", "Controls",
                    "PowerSupplies", "NetworkAdapters", "PCIeDevices",
                    "Drives", "Assembly", "LogServices"):
            if j.get(key):
                r = fetch(bmc, j[key]["@odata.id"]); out["results"].append(r)
                if key == "Sensors":
                    sj = r.get("json") or {}
                    out["results"].append({"group": "C-sensors-summary", "count": len(sj.get("Members") or []),
                                           "member0": (sj.get("Members") or [{}])[0]})
                if key == "PowerSupplies":
                    pj = r.get("json") or {}
                    for p in (pj.get("Members") or [])[:4]:
                        pu = p["@odata.id"]
                        out["results"].append(fetch(bmc, pu))
                        pm = (fetch(bmc, pu).get("json") or {})
                        if pm.get("Metrics"):
                            out["results"].append(fetch(bmc, pm["Metrics"]["@odata.id"]))
    return out
```

- [ ] **Step 4: 实现 D 组（Managers + SEL）**

```python
def g_managers(bmc):
    out = {"group": "D-managers", "results": []}
    root = fetch(bmc, "/redfish/v1")
    m_url = root.get("json", {}).get("Managers", {}).get("@odata.id")
    if not m_url: return out
    coll = fetch(bmc, m_url); out["results"].append(coll)
    for m in ((coll.get("json") or {}).get("Members") or []):
        mu = m["@odata.id"]
        out["results"].append(fetch(bmc, mu))
        mj = (fetch(bmc, mu).get("json") or {})
        if mj.get("LogServices"):
            lurl = mj["LogServices"]["@odata.id"]
            lc = fetch(bmc, lurl); out["results"].append(lc)
            for ls in ((lc.get("json") or {}).get("Members") or []):
                lu = ls["@odata.id"]
                lsvc = fetch(bmc, lu); out["results"].append(lsvc)
                entries_url = (lsvc.get("json") or {}).get("Entries", {}).get("@odata.id")
                if entries_url:
                    e = fetch(bmc, entries_url); out["results"].append(e)
                    ej = e.get("json") or {}
                    mems = ej.get("Members") or []
                    sev = {}
                    for x in mems[:50]:
                        sev[x.get("Severity", "?")] = sev.get(x.get("Severity", "?"), 0) + 1
                    out["results"].append({"group": "D-sel-summary", "total": ej.get("Members@odata.count"),
                                           "severity_dist": sev, "nextLink": "@odata.nextLink" in ej})
    return out
```

- [ ] **Step 5: 实现 E 组（固件清单）**

```python
def g_update(bmc):
    out = {"group": "E-update", "results": []}
    root = fetch(bmc, "/redfish/v1")
    u_url = root.get("json", {}).get("UpdateService", {}).get("@odata.id")
    if not u_url: return out
    u = fetch(bmc, u_url); out["results"].append(u)
    fi_url = (u.get("json") or {}).get("FirmwareInventory", {}).get("@odata.id")
    if fi_url:
        fi = fetch(bmc, fi_url); out["results"].append(fi)
        items = []
        for x in ((fi.get("json") or {}).get("Members") or [])[:50]:
            r = fetch(bmc, x["@odata.id"])
            out["results"].append(r)
            j = r.get("json") or {}
            items.append({"url": x["@odata.id"], "name": j.get("Name"),
                          "version": j.get("Version"), "updateable": j.get("Updateable")})
        out["results"].append({"group": "E-firmware-summary", "count": len(items),
                               "items": items, "has_nextLink": "@odata.nextLink" in (fi.get("json") or {})})
    return out
```

- [ ] **Step 6: 实现 F 组（电源明细双路径）**

```python
def g_power(bmc):
    out = {"group": "F-power", "results": []}
    root = fetch(bmc, "/redfish/v1")
    ch_url = root.get("json", {}).get("Chassis", {}).get("@odata.id")
    if not ch_url: return out
    coll = fetch(bmc, ch_url)
    for m in ((coll.get("json") or {}).get("Members") or []):
        cu = m["@odata.id"]
        c = fetch(bmc, cu); j = c.get("json") or {}
        for key in ("Power", "PowerSubsystem", "EnvironmentMetrics"):
            if j.get(key):
                r = fetch(bmc, j[key]["@odata.id"]); out["results"].append(r)
                pj = r.get("json") or {}
                if key == "Power":
                    out["results"].append({"group": "F-powercontrol-summary",
                        "pc": [(x.get("Name"), x.get("PowerConsumedWatts"),
                                x.get("PowerMetrics")) for x in (pj.get("PowerControl") or [])],
                        "psu": [(x.get("@odata.id"), x.get("Name")) for x in (pj.get("PowerSupplies") or [])]})
                    for ps in (pj.get("PowerSupplies") or [])[:4]:
                        out["results"].append(fetch(bmc, ps["@odata.id"]))
                if key == "EnvironmentMetrics":
                    out["results"].append({"group": "F-env-fields",
                        "fields": {k: v for k, v in pj.items()
                                   if k not in ("@odata.id", "@odata.type", "Name", "Id", "Status")}})
    return out
```

- [ ] **Step 7: 执行 A–F**

```powershell
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" A
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" B
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" C
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" D
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" E
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" F
```
预期：每命令输出 `done X`；`out\dells\*.json`、`out\inspur\*.json` 各有内容。检查 `C.json` 中 PowerSubsystem/ThermalSubsystem 是否存在、`D.json` 中 SEL 条目数与 severity 分布、`F.json` 中 PowerMetrics 是否存在 —— 这些直接决定 Task 8–11 的实现确认。把发现记录到 Task 6 的工作笔记（审计报告草稿）。

---

### Task 3: 探测矩阵 G–L 实现与执行

**Files:**
- Modify: `probe.py`（填充 g_sensors, g_storage, g_network, g_pagination, g_latency, g_oem）
- Create: `out\dells\G-L.json`、`out\inspur\G-L.json`

**Interfaces:**
- Consumes: `fetch`/`save`（Task 1）。
- Produces: 6 份快照；其中 K 组（延迟基准）的输出直接决定 Task 8 分频设计参数、J 组决定 Task 16 分页门控、L 组决定 Task 18 门控。

- [ ] **Step 1: 实现 G 组（传感器集合）**

```python
def g_sensors(bmc):
    out = {"group": "G-sensors", "results": []}
    root = fetch(bmc, "/redfish/v1")
    ch_url = root.get("json", {}).get("Chassis", {}).get("@odata.id")
    if not ch_url: return out
    coll = fetch(bmc, ch_url)
    for m in ((coll.get("json") or {}).get("Members") or []):
        c = fetch(bmc, m["@odata.id"]); j = c.get("json") or {}
        for key in ("Sensors", "Thermal", "Power"):
            if j.get(key):
                r = fetch(bmc, j[key]["@odata.id"]); out["results"].append(r)
                sj = r.get("json") or {}
                if key == "Sensors":
                    types, with_threshold, missing_reading = {}, 0, 0
                    for s in (sj.get("Members") or [])[:80]:
                        sr = fetch(bmc, s["@odata.id"])
                        out["results"].append(sr)
                        sv = sr.get("json") or {}
                        rt = sv.get("ReadingType", "?")
                        types[rt] = types.get(rt, 0) + 1
                        if sv.get("Thresholds"): with_threshold += 1
                        if sv.get("Reading") is None: missing_reading += 1
                    out["results"].append({"group": "G-sensor-summary", "total": len(sj.get("Members") or []),
                        "types": types, "with_thresholds": with_threshold,
                        "missing_reading": missing_reading,
                        "has_nextLink": "@odata.nextLink" in sj})
    return out
```

- [ ] **Step 2: 实现 H 组（存储）**

```python
def g_storage(bmc):
    out = {"group": "H-storage", "results": []}
    root = fetch(bmc, "/redfish/v1")
    sys_url = root.get("json", {}).get("Systems", {}).get("@odata.id")
    if not sys_url: return out
    sc = fetch(bmc, sys_url)
    for m in ((sc.get("json") or {}).get("Members") or []):
        s = fetch(bmc, m["@odata.id"]); sj = s.get("json") or {}
        if not sj.get("Storage"): continue
        st = fetch(bmc, sj["Storage"]["@odata.id"]); out["results"].append(st)
        stj = st.get("json") or {}
        for s1 in (stj.get("Members") or []):
            s1u = s1["@odata.id"]
            s1j = (fetch(bmc, s1u).get("json") or {})
            out["results"].append({"group": "H-storage-ctrl-summary",
                "controllers": [(c.get("Name"), c.get("Model"), c.get("FirmwareVersion"),
                                 c.get("SerialNumber"), bool(c.get("Oem"))) for c in (s1j.get("StorageControllers") or [])],
                "drives": [d["@odata.id"] for d in (s1j.get("Drives") or [])],
                "volumes": [v["@odata.id"] for v in (s1j.get("Volumes") or [])]})
            for d in (s1j.get("Drives") or [])[:12]:
                dr = fetch(bmc, d["@odata.id"]); out["results"].append(dr)
                dj = dr.get("json") or {}
                out["results"].append({"group": "H-drive-oem",
                    "drive": d["@odata.id"], "fields": sorted(dj.keys()),
                    "oem_keys": list((dj.get("Oem") or {}).keys())})
    return out
```

- [ ] **Step 3: 实现 I 组（网络）**

```python
def g_network(bmc):
    out = {"group": "I-network", "results": []}
    root = fetch(bmc, "/redfish/v1")
    sys_url = root.get("json", {}).get("Systems", {}).get("@odata.id")
    if sys_url:
        sc = fetch(bmc, sys_url)
        for m in ((sc.get("json") or {}).get("Members") or []):
            s = fetch(bmc, m["@odata.id"]); sj = s.get("json") or {}
            if sj.get("EthernetInterfaces"):
                e = fetch(bmc, sj["EthernetInterfaces"]["@odata.id"]); out["results"].append(e)
                ej = e.get("json") or {}
                out["results"].append({"group": "I-eth-summary",
                    "ifaces": [{"id": x.get("@odata.id"), "link": None} for x in (ej.get("Members") or [])]})
                for x in (ej.get("Members") or [])[:8]:
                    r = fetch(bmc, x["@odata.id"]); out["results"].append(r)
                    rj = r.get("json") or {}
                    out["results"].append({"group": "I-eth-fields",
                        "fields": sorted(k for k in rj.keys() if k not in ("@odata.id", "@odata.type"))})
    ch_url = root.get("json", {}).get("Chassis", {}).get("@odata.id")
    if ch_url:
        cc = fetch(bmc, ch_url)
        for m in ((cc.get("json") or {}).get("Members") or []):
            c = fetch(bmc, m["@odata.id"]); cj = c.get("json") or {}
            for key in ("NetworkAdapters", "PCIeDevices"):
                if cj.get(key):
                    r = fetch(bmc, cj[key]["@odata.id"]); out["results"].append(r)
                    rj = r.get("json") or {}
                    out["results"].append({"group": f"I-{key}-summary",
                        "members": len(rj.get("Members") or []),
                        "has_nextLink": "@odata.nextLink" in rj})
    return out
```

- [ ] **Step 4: 实现 J 组（分页探测）**

```python
def g_pagination(bmc):
    out = {"group": "J-pagination", "results": []}
    for path in ("/redfish/v1/Systems", "/redfish/v1/Chassis", "/redfish/v1/Managers",
                 "/redfish/v1/UpdateService/FirmwareInventory",
                 "/redfish/v1/SessionService/Sessions"):
        r = fetch(bmc, path)
        out["results"].append({"url": path, "status": r["status"],
            "has_nextLink": "@odata.nextLink" in (r.get("json") or {}),
            "members": len((r.get("json") or {}).get("Members") or []),
            "count": (r.get("json") or {}).get("Members@odata.count")})
    out["results"].append({"group": "J-note",
        "note": "SEL/传感器/磁盘/固件集合的 nextLink 已在 D/G/H/E 组检查；本组补 System/Chassis/Manager 顶层集合"})
    return out
```

- [ ] **Step 5: 实现 K 组（延迟基准）**

```python
def g_latency(bmc):
    out = {"group": "K-latency", "results": []}
    seq = ["/redfish/v1",
           "/redfish/v1/Systems", "/redfish/v1/Chassis", "/redfish/v1/Managers",
           "/redfish/v1/SessionService", "/redfish/v1/UpdateService"]
    import statistics
    all_ms = []
    for p in seq:
        r = fetch(bmc, p)
        out["results"].append({"url": p, "status": r["status"], "ms": r["ms"]})
        all_ms.append(r["ms"])
    if all_ms:
        out["results"].append({"group": "K-summary",
            "mean_ms": round(statistics.mean(all_ms), 1),
            "max_ms": max(all_ms), "n": len(all_ms),
            "note": "乘以单轮请求数（每 collector 重复遍历 collection 的 N×walk 放大倍数）
                     得估算轮耗时；分频设计据此定 slow_interval 建议值"})
    return out
```

- [ ] **Step 6: 实现 L 组（OEM 差异）**

```python
def g_oem(bmc):
    out = {"group": "L-oem", "results": []}
    root = fetch(bmc, "/redfish/v1")
    out["results"].append({"group": "L-root-oem", "root_oem_keys": list((root.get("json") or {}).get("Oem", {}).keys())})
    sys_url = root.get("json", {}).get("Systems", {}).get("@odata.id")
    if sys_url:
        sc = fetch(bmc, sys_url)
        for m in ((sc.get("json") or {}).get("Members") or []):
            s = fetch(bmc, m["@odata.id"]); sj = s.get("json") or {}
            out["results"].append({"group": "L-system-oem",
                "oem_keys": list((sj.get("Oem") or {}).keys()),
                "oem": sj.get("Oem")})
    ch_url = root.get("json", {}).get("Chassis", {}).get("@odata.id")
    if ch_url:
        cc = fetch(bmc, ch_url)
        for m in ((cc.get("json") or {}).get("Members") or []):
            c = fetch(bmc, m["@odata.id"]); cj = c.get("json") or {}
            out["results"].append({"group": "L-chassis-oem",
                "oem_keys": list((cj.get("Oem") or {}).keys()),
                "oem": cj.get("Oem")})
    return out
```

- [ ] **Step 7: 执行 G–L 并汇总初判**

```powershell
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" G
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" H
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" I
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" J
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" K
python "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\probe.py" L
```
预期：全部 `done X`。执行后人工检查要点（写入审计报告草稿）：
1. **浪潮 OEM**：`L-chassis-oem`/`L-system-oem` 的 `oem_keys` 是否含 `Public`（idrac_exporter 源码记录浪潮 power+drive life 在 `Oem.Public`）→ 决定 Task 18。
2. **Dell OEM**：`L-*` 的 Dell 键清单 → 决定 Task 18 的字段路径。
3. **分页**：`J.json` 与 D/G/H/E 的 `has_nextLink` → 全部为 false 则 Task 16 降级为 backlog。
4. **K 组**：mean_ms × 估算请求数 → 写入 Task 8 的 slow_interval 默认值建议。

---

### Task 4: 实跑 exporter 交叉对比

**Files:**
- Create: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-config.yaml`（不入仓库）
- Create: `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-metrics.txt`、`live-errors.log`

**Interfaces:**
- Consumes: Task 2/3 的探测基线（API 存在性事实表）。
- Produces: 实跑对比结果（每 BMC：指标族清单、failed_resources、轮耗时、缺失指标清单），供 Task 5 报告引用。

- [ ] **Step 1: 写实跑配置**

`live-config.yaml`（凭据仅此处，gitignore 外）：
```yaml
listen_addr: "127.0.0.1:9417"
scrape_interval: "30s"
scrape_timeout: "60s"
request_timeout: "15s"
bmcs:
  - name: dells
    host: https://10.10.90.70
    username: root
    password: <REDACTED-PASSWORD>
    auth: session
    insecure_skip_verify: true
  - name: inspur
    host: https://10.10.90.80
    username: admin
    password: <REDACTED-PASSWORD>
    auth: session
    insecure_skip_verify: true
```
（浪潮 session 若失败，按 spec 第 1 节 B 部分回退 basic，并把行为记为缺陷。）

- [ ] **Step 2: 启动并采集 3 轮**

```powershell
$env:RUST_LOG = "info"
Start-Process -FilePath "cargo" -ArgumentList "run","--release","--","-c","C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-config.yaml" -RedirectStandardOutput "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-errors.log" -RedirectStandardError "C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-errors.log"
Start-Sleep -Seconds 110   # 等待 ≥3 轮（30s 间隔 ×3 + 启动缓冲）
(Invoke-WebRequest -Uri 'http://127.0.0.1:9417/metrics' -TimeoutSec 20).Content | Set-Content 'C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-metrics.txt'
Get-Content 'C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\live-metrics.txt' | Select-String -Pattern '^redfish_up|^redfish_scrape' 
```
记录输出。随后 `Stop-Process` 结束 exporter。

- [ ] **Step 3: 交叉对比**

对 `live-metrics.txt`：
1. 每 BMC 列出出现的指标族（`^redfish_[a-z_]+{` 去重）→ 与 `docs/metrics.md` 清单比对。
2. 与探测基线比对：探测发现但 metrics 无的（如 PowerMetrics、SEL、Bios、AlarmTrips）→ 记"覆盖缺陷"。
3. 从日志提取每轮 `failed = N`、`scrape round complete` 的 duration_ms、401/重登/超时告警 → 记"可靠性缺陷"。
4. 浪潮 session 失败则记"兼容性缺陷（session 认证）"。
5. 把 `live-metrics.txt` 归档到 `out\liverun\`（不入仓库）。

---

### Task 5: 审计报告

**Files:**
- Create: `docs/audit/2026-08-21-bmc-audit.md`（入库）

**Interfaces:**
- Consumes: Task 2–4 全部快照与实跑结果。
- Produces: 审计报告（缺陷清单 P0–P3，每条附证据）；Task 6 差距表引用它。

- [ ] **Step 1: 写报告骨架**

报告结构（中文，证据驱动）：
```markdown
# 真机审计报告（Dell R750 / 浪潮）

日期：2026-08-21
范围：10.10.90.70（Dell R750, iDRAC）/ 10.10.90.80（浪潮）
方法：Python 探测矩阵 A–L + exporter 实跑交叉对比（见 spec 第 4 节）

## 1. API 基线摘要（每 BMC）
（表格：资源类型 → 存在性/关键字段/请求耗时；数据来自 out/<bmc>/*.json）

## 2. 实跑结果（每 BMC）
（指标族清单、redfish_up、failed_resources、轮耗时；数据来自 live-metrics.txt / live-errors.log）

## 3. 缺陷清单
| ID | 严重度 | 缺陷 | 证据 | 关联对标 |
|----|--------|------|------|----------|
| AUDIT-1 | P0-P3 | ... | 快照/日志引用 + 脱敏片段 | idrac/fishymetrics/sapcc/无 |

## 4. 厂商差异结论
（Dell vs 浪潮：双路径存在性、OEM 键、会话行为、分页）

## 5. 分频设计输入
（K 组延迟 → 建议 slow_interval；请求数估算）
```
严重度定义：P0 数据损坏/崩溃；P1 核心指标缺失/数值错误；P2 边缘缺失/兼容性；P3 体验。

- [ ] **Step 2: 逐节填写（基于真实数据）**

每个缺陷行必须含：可复现条件 + 证据（`out/<bmc>/X.json` 中的字段/状态码，或日志行）。没有数据的结论一律不写。凭据不出现在报告中。

- [ ] **Step 3: 提交**

```bash
git add docs/audit/2026-08-21-bmc-audit.md
git commit -m "docs: real-device audit report (Dell R750 + Inspur)"
```
（此提交即 Task 1 的仓库内产物提交。）

---

### Task 6: 对标差距表

**Files:**
- Create: `docs/audit/2026-08-21-gap-analysis.md`（入库）

**Interfaces:**
- Consumes: 审计报告（Task 5）+ 三个对标仓库深度分析结论（spec 第 1 节）。
- Produces: 差距表；其每行"三关门禁结论"直接驱动 Task 7–18 的取舍，Task 16/18 的门控条件引用本表对应行。

- [ ] **Step 1: 建表**

结构（中文）：
```markdown
# 对标差距表（idrac_exporter / fishymetrics / sapcc vs 我们）

维度：指标族 / 调度 / 可靠性 / 运维 / 安全
每行：| 功能点 | idrac | fishymetrics | sapcc | 我们现状(含审计证据) | nv 能力 | 三关门禁 | 结论 |

## 结论汇总
实现：…（引用 Task 8-15）
评估后定：…（引用 Task 16-18 门控）
backlog：…（Vault/凭据脚本、共享预取、PDU、SSE）
不采纳：…（健康值数值映射 D11 —— 破坏现有 PromQL，label 方案保留；fishymetrics 100ms 强制睡眠；idrac 无条件 insecure TLS；sapcc 每请求重建会话）
```
- [ ] **Step 2: 填全每一行（约 25–30 行）**

行来源：spec 第 1.1–1.3 节"值得吸纳"与"硬伤"清单逐项成行；每行三关门禁给出 `实现 / backlog / 不采纳 + 理由`。与审计报告交叉引用（如 `AUDIT-x`）。

- [ ] **Step 3: 提交**

```bash
git add docs/audit/2026-08-21-gap-analysis.md
git commit -m "docs: gap analysis vs three reference exporters"
```

---

# 阶段二：必做实现（TDD）

### Task 6A: per-BMC 轮超时隔离（修复 AUDIT-1）

**背景（审计证据）**：AUDIT-1 —— 浪潮一轮需 67–89s，`scrape_timeout=60s` 整轮截止（scraper.rs:143 `tokio::time::timeout(self.timeout, collect)`）把整轮 abort；被 abort 的 BMC 任务不产生任何 registry 更新 → `/metrics` 上该 BMC **彻底消失**（无 `redfish_up=0`，Prometheus 静默丢数据）。同时 Dell 的已完成结果也被整轮截止拖住。修复：**截止时间改为每 BMC 独立**；超时的 BMC 发布 `redfish_up=0` 快照（不静默消失）。

**Files:**
- Modify: `src/scraper.rs`（per-BMC deadline + 超时发布 up=0）
- Test: `tests/scraper_test.rs`

**Interfaces:**
- Consumes: 现有 `collect_all`、`build_registry`、`Snapshot::update`。
- Produces:
  - `src/scraper.rs` 新 helper：`async fn with_deadline<T>(timeout: Duration, fut: impl Future<Output = T>) -> Option<T>`（None = 超时）。
  - 行为变化：每 BMC 任务 = `with_deadline(self.timeout, collect_all(...))`；`Some(report)` → 正常发布；`None`（超时）→ 发布 `ScrapeReport { metrics: [up=0], failed_resources: ["timeout"] }`（复用现有失败发布路径，见 scraper.rs:120-136 的构造模式）；移除整轮 `timeout(self.timeout, collect)`（collect 循环保留，等全部 BMC 任务结束）。

- [ ] **Step 1: 写失败测试**

`tests/scraper_test.rs` 追加：
```rust
#[tokio::test(start_paused = true)]
async fn with_deadline_times_out() {
    use redfish_exporter::scraper::with_deadline;
    use std::time::Duration;
    let never = async { tokio::time::sleep(Duration::from_secs(3600)).await; 42 };
    let result = tokio::time::timeout(Duration::from_secs(1), with_deadline(Duration::from_secs(1), never)).await;
    assert!(result.is_ok()); // 内部 deadline 先触发
    assert_eq!(result.unwrap(), None);
}

#[tokio::test]
async fn with_deadline_completes_in_time() {
    use redfish_exporter::scraper::with_deadline;
    use std::time::Duration;
    let ok = async { 7 };
    assert_eq!(with_deadline(Duration::from_secs(1), ok).await, Some(7));
}
```
先运行确认失败（`with_deadline` 未定义）。

- [ ] **Step 2: 实现 with_deadline**

`src/scraper.rs`：
```rust
pub async fn with_deadline<T>(timeout: Duration, fut: impl Future<Output = T>) -> Option<T> {
    match tokio::time::timeout(timeout, fut).await {
        Ok(v) => Some(v),
        Err(_) => None,
    }
}
```

- [ ] **Step 3: 改造 scrape_once 为 per-BMC deadline**

`scrape_once` 中每个 BMC 任务改为：
```rust
set.spawn(async move {
    let result = match with_deadline(round_timeout, collect_all(Arc::clone(&bmc), &name)).await {
        Some(Ok(report)) => Ok(report),
        Some(Err(err)) if auth == AuthMethod::Session && is_unauthorized(&err) => { /* 现有重登逻辑不变，重登重试也套 with_deadline */ }
        Some(Err(err)) => Err(err),
        None => Err(anyhow::anyhow!("round timed out for bmc")), // 或专用标记
    };
    (name, result)
});
```
超时路径的标记：在 `Ok((name, Err(e)))` 分支已存在（scraper.rs:120-136）——它会发布 up=0 + failed ["bmc"]。为区分超时与普通失败，用错误字符串标记 `"bmc: timeout"` 并让 failed_resources 记 `"timeout"`；发布逻辑不变（超时也发布 up=0 快照）。注意：**超时不再调用 `set.shutdown()`**（不再中止其他 BMC），整轮 collect 循环自然收敛。

- [ ] **Step 4: 测试通过**

Run: `cargo test --test scraper_test`
预期：PASS（两个新测试 + 现有全部）。

- [ ] **Step 5: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/scraper.rs tests/scraper_test.rs
git commit -m "fix: per-BMC round deadline and publish redfish_up=0 on timeout"
```

---

### Task 6B: 启动会话失败不退出（修复 AUDIT-2）

**背景（审计证据）**：AUDIT-2 —— 浪潮 session 建立失败（create-session 响应缺 `Name` 字段）导致 `Scraper::new` 返回 Err → `main.rs:69/77 exit(1)`，**整个 exporter 启动即死**，所有 BMC（含健康的 Dell）全部不可用。修复：单 BMC 会话建立失败 → 降级 basic 认证继续运行 + 记 warn；每轮尝试补建会话。

**Files:**
- Modify: `src/bmc.rs`（`try_establish_session` 拆出可测函数）
- Modify: `src/scraper.rs`（`new` 不再 fail-fast；每轮补建会话）
- Test: `tests/bmc_test.rs`

**Interfaces:**
- Consumes: `establish_session`（bmc.rs，已泛型可 mock 测试）。
- Produces:
  - `bmc.rs`: `pub async fn try_establish_session<B: Bmc>(bmc: &Arc<B>, username: &str, password: &str) -> Result<String, BmcError>` —— 即现 `establish_session` 的语义（返回 token），保留 `establish_session` 名称亦可；关键是 scraper 不再传播其 Err。
  - `scraper.rs`: `Scraper::new` 中 session 建立失败 → `warn!` + 该 BMC 保持 basic 凭据继续；`Scrape` 任务开头：`auth == Session && !session_established` 时尝试建立（成功则 `set_credentials(token)` 并标记，失败则继续 basic 采集并 `warn!`）。

- [ ] **Step 1: 写失败测试**

`tests/bmc_test.rs` 追加（现有 mock session 测试模式）：
```rust
#[tokio::test]
async fn try_establish_session_failure_returns_err_not_panic() {
    // mock: ServiceRoot 正常；SessionService 链接存在；Sessions 集合存在；
    // create_session 期望返回 Err(注入错误)（或响应缺少 auth token）
    // 断言 establish_session 返回 Err（不 panic），错误为 BmcError::Session(_)
}
```
先运行确认现有行为符合（该测试主要防回归：确保错误路径返回 Err 而非 panic）。

- [ ] **Step 2: 实现 scraper 降级与补建**

`src/scraper.rs`：
- `BmcHandle` 增加 `session_established: bool`（bmc.rs 中定义；`make_bmc` 初始 `session_established = auth == AuthMethod::Session` 时 false）。
- `Scraper::new`：session 分支改为：
```rust
if bmc_cfg.auth == AuthMethod::Session {
    match establish_session(&handle.bmc, &bmc_cfg.username, bmc_cfg.password.expose()).await {
        Ok(token) => { handle.bmc.set_credentials(BmcCredentials::token(token)); handle.session_established = true; }
        Err(e) => { warn!(bmc = %bmc_cfg.name, error = %e, "session establishment failed, falling back to basic auth"); }
    }
}
```
（不再 `?` / 不再返回 Err → `Scraper::new` 对本场景不失败。）
- `collect_round`（Task 7 定义）或现有任务闭包开头（Task 7 之前是 `collect_all` 闭包）：
```rust
if auth == AuthMethod::Session && !handle.session_established {
    if let Ok(token) = establish_session(&bmc, &username, password.expose()).await {
        bmc.set_credentials(BmcCredentials::token(token));
        handle.session_established = true;
        info!(bmc = %name, "session established on first round");
    }
}
```
（`session_established` 用 `Arc<AtomicBool>` 跨轮共享；Task 7 重写 scraper 时保留此字段。）

- [ ] **Step 3: 测试通过**

Run: `cargo test --test bmc_test`
预期：PASS。

- [ ] **Step 4: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/bmc.rs src/scraper.rs tests/bmc_test.rs
git commit -m "fix: degrade to basic auth on session establishment failure instead of aborting"
```

---

### Task 7: 分频采集（快/慢组）

**Files:**
- Modify: `src/config.rs`（加 `slow_interval` 字段与解析/校验）
- Modify: `src/collector/mod.rs`（拆分 `collect_fast`/`collect_slow` + `merge_reports` + `finalize_report`）
- Modify: `src/scraper.rs`（每 BMC 慢组状态与合并发布）
- Test: `tests/config_test.rs`、`tests/scraper_test.rs`

**Interfaces:**
- Consumes: 现有 `collect_all` 的内部 collector 调用（保持不变，仅重新分组）。
- Produces:
  - `config.rs`: `pub slow_interval: Option<Duration>`（默认 None = 不分频，行为与 0.1.0 完全一致）
  - `collector/mod.rs`:
    ```rust
    pub async fn collect_fast<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>, bmc_name: &str) -> Result<ScrapeReport, String>
    pub async fn collect_slow<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>, bmc_name: &str) -> Result<ScrapeReport, String>
    pub fn merge_reports(fast: ScrapeReport, slow: Option<&ScrapeReport>) -> ScrapeReport
    pub fn finalize_report(bmc_name: &str, metrics: Vec<Metric>, failed: Vec<String>, started: Instant) -> ScrapeReport
    ```
    `collect_fast` 内部依次 await：sensors、power、processors、memory、systems(collect_systems)、chassis_health、managers；`collect_slow` 依次 await：storage、network、firmware、assembly。两者都不再产出 up/duration（由 `finalize_report` 统一添加）。

- [ ] **Step 1: 写失败测试（config 解析）**

在 `tests/config_test.rs` 追加：
```rust
#[test]
fn parses_slow_interval() {
    let p = write_tmp(
        "slow",
        r#"
scrape_interval: "30s"
slow_interval: "300s"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
"#,
    );
    let cfg = load_config(&p).unwrap();
    assert_eq!(cfg.slow_interval, Some(std::time::Duration::from_secs(300)));
}

#[test]
fn slow_interval_defaults_to_none() {
    let p = write_tmp(
        "no_slow",
        r#"
scrape_interval: "30s"
bmcs:
  - { name: a, host: https://h1, username: u, password: "p" }
"#,
    );
    let cfg = load_config(&p).unwrap();
    assert_eq!(cfg.slow_interval, None);
}
```
预期：编译失败（无字段）。

- [ ] **Step 2: 实现 config 字段**

`src/config.rs`：`RawConfig` 加
```rust
#[serde(default, deserialize_with = "deserialize_optional_duration")]
slow_interval: Option<Duration>,
```
加函数：
```rust
fn default_none_duration() -> Option<Duration> { None }
fn deserialize_optional_duration<'de, D>(de: D) -> Result<Option<Duration>, D::Error>
where D: serde::Deserializer<'de> {
    let opt: Option<String> = Option::deserialize(de)?;
    opt.map(|s| humantime::parse_duration(&s).map_err(serde::de::Error::custom))
        .transpose()
}
```
`Config` 结构体加 `pub slow_interval: Option<Duration>`；`load_config` 校验：`if let Some(si) = raw.slow_interval { if si.is_zero() { return Err(ConfigError::Invalid("slow_interval must be > 0".into())); } }`。

- [ ] **Step 3: 跑 config 测试**

Run: `cargo test --test config_test`
预期：PASS。

- [ ] **Step 4: 拆分 collect_fast/collect_slow**

`src/collector/mod.rs`：把 `collect_all` 的 11 个 collector 调用按快/慢分组拆成两个函数；两者返回 `ScrapeReport`（不含 up/duration）。新增：
```rust
pub fn finalize_report(
    bmc_name: &str,
    metrics: Vec<Metric>,
    failed: Vec<String>,
    started: Instant,
) -> ScrapeReport {
    let mut metrics = metrics;
    let up = if failed.is_empty() { 1.0 } else { 0.0 };
    metrics.push(Metric::gauge(UP.0, UP.1).label("bmc", bmc_name.to_string()).build(up));
    metrics.push(
        Metric::gauge(SCRAPE_DURATION.0, SCRAPE_DURATION.1)
            .label("bmc", bmc_name.to_string())
            .build(started.elapsed().as_secs_f64()),
    );
    ScrapeReport { metrics, failed_resources: failed }
}

pub fn merge_reports(fast: ScrapeReport, slow: Option<&ScrapeReport>) -> ScrapeReport {
    let mut metrics = fast.metrics;
    let mut failed = fast.failed_resources;
    if let Some(s) = slow {
        metrics.extend(s.metrics.iter().cloned());
        for r in &s.failed_resources {
            if !failed.contains(r) { failed.push(r.clone()); }
        }
    }
    ScrapeReport { metrics, failed_resources: failed }
}
```
注意：`ScrapeReport` 的 `failed_resources` 语义不变；`merge_reports` 在 build_registry 前调用，`up` 由 `finalize_report` 生成后 merge 中不做重复 up 处理（两个 report 各含一个 up → 相同 label 集合，prometheus 合并为同系列，值相同，无冲突）。

- [ ] **Step 5: 写失败测试（分组与合并）**

`tests/scraper_test.rs` 追加：
```rust
#[tokio::test]
async fn merge_reports_combines_metrics_and_failed() {
    let fast = ScrapeReport {
        metrics: vec![Metric::gauge("a", "a").label("bmc", "b".into()).build(1.0)],
        failed_resources: vec!["sensors".into()],
    };
    let slow = ScrapeReport {
        metrics: vec![Metric::gauge("b", "b").label("bmc", "b".into()).build(2.0)],
        failed_resources: vec!["storage".into(), "sensors".into()],
    };
    let merged = merge_reports(fast, Some(&slow));
    assert_eq!(merged.metrics.len(), 2);
    assert_eq!(merged.failed_resources, vec!["sensors", "storage"]);
}
```
（`use redfish_exporter::collector::merge_reports;`）

- [ ] **Step 6: 改 scraper 实现慢组状态**

`src/scraper.rs`：
- `Scraper` 增加 `slow_interval: Option<Duration>` 与 `slow_state: Arc<Mutex<HashMap<String, (Instant, Option<ScrapeReport>)>>>`（`use std::sync::Mutex;`）。
- `Scraper::new` 读 `cfg.slow_interval`。
- per-BMC 任务内替换 `collect_all`：
```rust
let slow_interval = self.slow_interval;
let slow_state = Arc::clone(&self.slow_state);
set.spawn(async move {
    let result = match collect_round(
        Arc::clone(&bmc), &name, slow_interval, Arc::clone(&slow_state),
    ).await { ... };
    ...
});
```
- 新增：
```rust
type ConcreteBmc = HttpBmc<ReqwestClient>;

async fn collect_round(
    bmc: Arc<ConcreteBmc>,
    name: &str,
    slow_interval: Option<Duration>,
    slow_state: Arc<Mutex<HashMap<String, (Instant, Option<ScrapeReport>)>>>,
) -> Result<ScrapeReport, nv_redfish::Error<ConcreteBmc>> {
    let root = nv_redfish::ServiceRoot::new(Arc::clone(&bmc)).await?;
    let started = Instant::now();
    let fast = match collect_fast(Arc::clone(&bmc), &root, name).await {
        Ok(r) => r,
        Err(resource) => ScrapeReport { metrics: vec![], failed_resources: vec![resource] },
    };
    let mut slow_report = slow_state.lock().unwrap().get(name).and_then(|(_, r)| r.clone());
    let slow_due = match slow_interval {
        Some(interval) => {
            let last = slow_state.lock().unwrap().get(name).map(|(t, _)| *t).unwrap_or(Instant::now() - interval);
            last.elapsed() >= interval
        }
        None => false,
    };
    if slow_due {
        let new_slow = match collect_slow(Arc::clone(&bmc), &root, name).await {
            Ok(r) => r,
            Err(resource) => ScrapeReport { metrics: vec![], failed_resources: vec![resource] },
        };
        slow_report = Some(new_slow.clone());
        slow_state.lock().unwrap().insert(name.to_string(), (Instant::now(), Some(new_slow)));
    }
    let merged = merge_reports(fast, slow_report.as_ref());
    Ok(finalize_report(name, merged.metrics, merged.failed_resources, started))
}
```
（`collect_round` 返回 `nv_redfish::Error<ConcreteBmc>` 而非 String，使 401 重登与 404 判定（Task 14）可在 scraper 层直接匹配状态码；scraper 现有 `is_unauthorized` 已是此具体类型。任务错误路径沿用现有 401 重登逻辑不变。）
- 保留 `is_unauthorized`/重登逻辑：重登后重试调用 `collect_round`（与现状对 `collect_all` 的调用方式一致）。

- [ ] **Step 7: 修现有测试**

`tests/integration_test.rs` 若引用 `collect_all`，改为 `collect_fast` + `finalize_report`（按编译错误逐一修改）。跑全部测试。

Run: `cargo test`
预期：全绿。

- [ ] **Step 8: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/config.rs src/collector/mod.rs src/scraper.rs tests/config_test.rs tests/scraper_test.rs tests/integration_test.rs
git commit -m "feat: per-BMC fast/slow scrape groups (slow_interval config)"
```

---

### Task 8: 事件日志 collector

**Files:**
- Modify: `Cargo.toml`（nv-redfish features 加 `"log-services"`）
- Create: `src/collector/logs.rs`
- Modify: `src/collector/mod.rs`（注册 `collect_event_logs` 到慢组）
- Test: `tests/collector_logs_test.rs`

**Interfaces:**
- Consumes: nv 0.15.1 `Manager::log_services()`、`LogService::entries()`、`schema::log_entry::LogEntry`（启用特性后生成）；`push_health`/`status_labels` 现有 helper。
- Produces: `pub async fn collect_event_logs<B: Bmc>(bmc: Arc<B>, root: &ServiceRoot<B>, bmc_name: &str) -> Result<Vec<Metric>, String>`；指标 `redfish_event_log_entry`（labels: bmc, manager, service, severity, message, id；value = 条目创建时间的 Unix 秒）。

- [ ] **Step 1: 启用特性并核对类型**

`Cargo.toml` 的 `nv-redfish` features 数组追加 `"log-services"`。跑 `cargo check`。随后核对生成的 `target/debug/build/nv-redfish-*/out/redfish.rs` 中 LogEntry 字段名（`rg -n "pub struct LogEntry"` 附近）：预期有 `created: Option<Option<EdmDateTimeOffset>>`、`message: Option<Option<String>>`、`severity: Option<Option<LogSeverity>>`（enum，`#[serde(other)]` 兜底）。把实际字段名记录在此任务备注中（若字段名不同，后续代码按实际名调整）。

- [ ] **Step 2: 写失败测试**

`tests/collector_logs_test.rs`：
```rust
use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::logs::collect_event_logs;
use serde_json::json;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

#[tokio::test]
async fn collects_event_log_entries() {
    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get("/redfish/v1", json!({
        "@odata.id": "/redfish/v1", "Id": "Root", "Name": "Root",
        "RedfishVersion": "1.0.0",
        "Managers": { "@odata.id": "/redfish/v1/Managers" },
        "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
    })));
    bmc.expect(Expect::get("/redfish/v1/Managers", json!({
        "@odata.id": "/redfish/v1/Managers",
        "@odata.type": "#ManagerCollection.ManagerCollection",
        "Name": "Managers", "Members": [{ "@odata.id": "/redfish/v1/Managers/BMC" }],
    })));
    bmc.expect(Expect::get("/redfish/v1/Managers/BMC", json!({
        "@odata.id": "/redfish/v1/Managers/BMC",
        "Id": "BMC", "Name": "BMC", "ManagerType": "BMC",
        "LogServices": { "@odata.id": "/redfish/v1/Managers/BMC/LogServices" },
    })));
    bmc.expect(Expect::get("/redfish/v1/Managers/BMC/LogServices", json!({
        "@odata.id": "/redfish/v1/Managers/BMC/LogServices",
        "@odata.type": "#LogServiceCollection.LogServiceCollection",
        "Name": "LogServices", "Members": [{ "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel" }],
    })));
    bmc.expect(Expect::get("/redfish/v1/Managers/BMC/LogServices/Sel", json!({
        "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel",
        "Id": "Sel", "Name": "SEL", "LogEntryType": "SEL",
        "Entries": { "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries" },
    })));
    bmc.expect(Expect::get("/redfish/v1/Managers/BMC/LogServices/Sel/Entries", json!({
        "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries",
        "@odata.type": "#LogEntryCollection.LogEntryCollection",
        "Name": "Entries",
        "Members": [{
            "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries/1",
            "Id": "1", "Name": "Entry 1",
            "Created": "2026-08-01T12:00:00Z",
            "Message": "Fan redundancy lost",
            "Severity": "Critical",
        }],
    })));
    bmc.expect(Expect::get("/redfish/v1/Managers/BMC/LogServices/Sel/Entries/1", json!({
        "@odata.id": "/redfish/v1/Managers/BMC/LogServices/Sel/Entries/1",
        "Id": "1", "Name": "Entry 1",
        "Created": "2026-08-01T12:00:00Z",
        "Message": "Fan redundancy lost",
        "Severity": "Critical",
    })));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_event_logs(bmc, &root, "bmc1").await.unwrap();
    let entry: Vec<_> = metrics.iter().filter(|m| m.name == "redfish_event_log_entry").collect();
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].value, 1782993600.0); // 2026-08-01T12:00:00Z
}
```
若 Severity 枚举名不同（如 `LogSeverity` 的变体名），mock JSON 用可解析的值（如 `"Critical"`）并核对测试期望。先运行确认失败（编译错误 = 无 logs 模块）。

- [ ] **Step 3: 实现 logs.rs**

```rust
use crate::collector::status_labels;
use crate::metrics::Metric;
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const EVENT_LOG_ENTRY: (&str, &str) = (
    "redfish_event_log_entry",
    "Event log entry, value is the entry creation time as a Unix timestamp",
);

pub async fn collect_event_logs<B: Bmc>(
    bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(managers) = root.managers().await.map_err(|e| format!("managers: {e}"))? else {
        return Ok(out);
    };
    let managers = managers.members().await.map_err(|e| format!("manager members: {e}"))?;
    for manager in managers {
        let manager_id = manager.id().to_string();
        let Ok(Some(services)) = manager.log_services().await else { continue };
        for service in services {
            let service_id = service.id().to_string();
            let Ok(Some(entries)) = service.entries().await else { continue };
            for entry in entries {
                push_entry(&mut out, bmc_name, &manager_id, &service_id, &entry);
            }
        }
    }
    Ok(out)
}

fn push_entry<B: Bmc>(out: &mut Vec<Metric>, bmc_name: &str, manager: &str, service: &str, entry: &nv_redfish::schema::log_entry::LogEntry) {
    let id = entry.base.id.to_string();
    let message = entry.message.clone().flatten().unwrap_or_default();
    let severity = entry.severity.as_ref().and_then(|s| s.as_ref())
        .map(|s| format!("{s:?}")).unwrap_or_default();
    let ts = entry.created.as_ref().and_then(|c| c.as_ref())
        .and_then(|c| SystemTime::try_from(*c).ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as f64);
    let Some(ts) = ts else { return };
    out.push(Metric::gauge(EVENT_LOG_ENTRY.0, EVENT_LOG_ENTRY.1)
        .label("bmc", bmc_name.to_string())
        .label("manager", manager.to_string())
        .label("service", service.to_string())
        .label("severity", severity)
        .label("message", message)
        .label("id", id)
        .build(ts));
}
```
（`entry.base.id` 与 `message`/`severity`/`created` 的实际字段名以 Step 1 核对结果为准；`LogEntry` 的 `id()` 方法可能来自 `Resource` trait —— 若 `base.id` 与 `id()` 均不可用，用 `entry.base.id.to_string()` 的替代字段。）

- [ ] **Step 4: 注册到慢组**

`src/collector/mod.rs`：`pub mod logs;`；`collect_slow` 中追加：
```rust
match logs::collect_event_logs(Arc::clone(&bmc), &root, bmc_name).await {
    Ok(m) => metrics.extend(m),
    Err(resource) => failed_resources.push(resource),
}
```
（在 storage/network/firmware 之后调用；`collect_slow` 的变量命名为 `metrics`/`failed_resources`，按 Task 7 的实现为准。）

- [ ] **Step 5: 测试通过**

Run: `cargo test --test collector_logs_test`
预期：PASS。

- [ ] **Step 6: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add Cargo.toml Cargo.lock src/collector/logs.rs src/collector/mod.rs tests/collector_logs_test.rs
git commit -m "feat: event log collector via nv log-services feature"
```

---

### Task 9: BIOS collector

**Files:**
- Modify: `Cargo.toml`（features 加 `"bios"`）
- Create: `src/collector/bios.rs`
- Modify: `src/collector/mod.rs`（注册到慢组）
- Test: `tests/collector_bios_test.rs`

**Interfaces:**
- Consumes: `ComputerSystem::bios()`、`Bios::raw()`、`schema::bios::Bios`（`attributes.dynamic_properties: HashMap<String, Value>`，属性值为 `EdmPrimitiveType`）；`RedfishSettings` trait 的 `settings_object()` 检测 pending。
- Produces: `collect_bios(...) -> Result<Vec<Metric>, String>`；指标：
  - `redfish_bios_pending_changes{bmc, system}`（0/1）
  - `redfish_bios_attribute{bmc, system, attribute}`（数值编码：bool→0/1、int/float→值、"Enabled"/"Disabled"→1/0、其他跳过）
  - `redfish_bios_attribute_info{bmc, system, attribute, value}`（非数值字符串，值恒 1）

- [ ] **Step 1: 核对 bios schema 类型**

启用特性后核对 `redfish.rs`：
- `rg -n "pub struct Bios"` → 确认 `attributes: Option<BiosAttributes>` 与 `BiosAttributes.dynamic_properties: HashMap<String, Value>`。
- `rg -n "EdmPrimitiveType" "$core\src\edm_primitive_type.rs"` → 确认变体（String/Bool/Int64/f64 等）与实际取值方法。
- 确认 `RedfishSettings` trait 的 `settings_object()` 位于 `nv_redfish` 根（core lib.rs:344-347 附近），`Bios` 是否实现。

- [ ] **Step 2: 写失败测试**

`tests/collector_bios_test.rs`：
```rust
use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::bios::collect_bios;
use serde_json::json;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

#[tokio::test]
async fn collects_bios_attributes() {
    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get("/redfish/v1", json!({
        "@odata.id": "/redfish/v1", "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
        "Systems": { "@odata.id": "/redfish/v1/Systems" },
    })));
    bmc.expect(Expect::get("/redfish/v1/Systems", json!({
        "@odata.id": "/redfish/v1/Systems",
        "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
        "Name": "Systems", "Members": [{ "@odata.id": "/redfish/v1/Systems/1" }],
    })));
    bmc.expect(Expect::get("/redfish/v1/Systems/1", json!({
        "@odata.id": "/redfish/v1/Systems/1",
        "Id": "1", "Name": "System 1",
        "Bios": { "@odata.id": "/redfish/v1/Systems/1/Bios" },
    })));
    bmc.expect(Expect::get("/redfish/v1/Systems/1/Bios", json!({
        "@odata.id": "/redfish/v1/Systems/1/Bios",
        "Id": "Bios", "Name": "BIOS",
        "Attributes": {
            "BootMode": "Uefi",
            "MemTest": "Disabled",
            "LegacyBoot": true,
            "MaxCores": 16,
            "SerialNumber": "SN123",
            "ListAttr": ["a", "b"]
        },
        "@Redfish.Settings": { "@odata.id": "/redfish/v1/Systems/1/Bios/Settings" },
    })));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_bios(bmc, &root, "bmc1").await.unwrap();
    let attrs: Vec<_> = metrics.iter().filter(|m| m.name == "redfish_bios_attribute").collect();
    assert_eq!(attrs.len(), 3); // BootMode? no (string, non-enabled) -> info; MemTest Disabled -> 0; LegacyBoot true -> 1; MaxCores 16
    // 依据 Step 1 核对的 EdmPrimitiveType 语义调整断言：字符串非 Enabled/Disabled 走 info 指标
    let info: Vec<_> = metrics.iter().filter(|m| m.name == "redfish_bios_attribute_info").collect();
    assert!(!info.is_empty());
    let pending = metrics.iter().find(|m| m.name == "redfish_bios_pending_changes").unwrap();
    assert_eq!(pending.value, 1.0);
}
```
若 `@Redfish.Settings` 在 mock 中经 `settings_object()` 不可见（需额外 GET），则 pending 判定改为检查 raw 中 `@Redfish.Settings` 键是否存在（通过 raw() 的 serde_json Value 访问），并在断言中说明。以实际编译/运行结果校准测试与实现。

- [ ] **Step 3: 实现 bios.rs**

```rust
use crate::metrics::Metric;
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::schema::edm_primitive_type::EdmPrimitiveType; // 路径以 Step 1 核对为准
use std::sync::Arc;

pub const BIOS_PENDING: (&str, &str) = ("redfish_bios_pending_changes", "BIOS settings pending reboot");
pub const BIOS_ATTR: (&str, &str) = ("redfish_bios_attribute", "BIOS attribute value");
pub const BIOS_ATTR_INFO: (&str, &str) = ("redfish_bios_attribute_info", "BIOS string attribute");

pub async fn collect_bios<B: Bmc>(
    bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(systems) = root.systems().await.map_err(|e| format!("systems: {e}"))? else {
        return Ok(out);
    };
    let systems = systems.members().await.map_err(|e| format!("system members: {e}"))?;
    for system in systems {
        let system_id = system.id().to_string();
        let Ok(Some(bios)) = system.bios().await else { continue };
        let raw = bios.raw();
        let pending = settings_pending(&raw);
        out.push(Metric::gauge(BIOS_PENDING.0, BIOS_PENDING.1)
            .label("bmc", bmc_name.to_string())
            .label("system", system_id.clone())
            .build(if pending { 1.0 } else { 0.0 }));
        let Some(attrs) = &raw.attributes else { continue };
        for (name, value) in &attrs.dynamic_properties {
            push_attribute(&mut out, bmc_name, &system_id, name, value);
        }
    }
    Ok(out)
}
```
`settings_pending`：若 `raw` 可直接读 `@Redfish.Settings`（`Bios` schema 有 `settings` 字段或经 `RedfishSettings` trait），按实际可用 API 实现；否则跳过 pending 指标并在测试中移除对应断言（在任务备注记录原因）。

`push_attribute` 按 sapcc 语义：
```rust
fn push_attribute(out: &mut Vec<Metric>, bmc: &str, system: &str, name: &str, value: &EdmPrimitiveType) {
    let Some(num) = numeric(value) else {
        let Some(s) = string_value(value) else { return };
        if s.eq_ignore_ascii_case("Enabled") { out.push(attr_metric(out, ...1.0)); return; }
        if s.eq_ignore_ascii_case("Disabled") { out.push(attr_metric(..., 0.0)); return; }
        out.push(Metric::gauge(BIOS_ATTR_INFO.0, BIOS_ATTR_INFO.1)
            .label("bmc", bmc.to_string()).label("system", system.to_string())
            .label("attribute", name.to_string()).label("value", s).build(1.0));
        return;
    };
    out.push(Metric::gauge(BIOS_ATTR.0, BIOS_ATTR.1)
        .label("bmc", bmc.to_string()).label("system", system.to_string())
        .label("attribute", name.to_string()).build(num));
}
```
`numeric`/`string_value` 依据 Step 1 核对的 `EdmPrimitiveType` 变体实现（bool→0/1、Int64/f64→值、String→字符串）。

- [ ] **Step 4: 注册到慢组**

`src/collector/mod.rs`：`pub mod bios;`；`collect_slow` 追加 `bios::collect_bios(...)` 调用（模式同 Task 8 Step 4）。

- [ ] **Step 5: 测试通过**

Run: `cargo test --test collector_bios_test`
预期：PASS（按实际 API 校准后）。

- [ ] **Step 6: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add Cargo.toml Cargo.lock src/collector/bios.rs src/collector/mod.rs tests/collector_bios_test.rs
git commit -m "feat: BIOS attribute collector via nv bios feature"
```

---

### Task 10: 内存纠错计数

**Files:**
- Modify: `src/collector/memory.rs`
- Modify: `src/metrics.rs`
- Test: `tests/collector_proc_mem_test.rs`

**Interfaces:**
- Consumes: `Memory::metrics()` → `MemoryMetrics.raw().health_data.alarm_trips`（已确认存在：`memory_metrics::HealthData { alarm_trips: Option<memory_metrics::AlarmTrips> }`，`AlarmTrips` 含 `correctable_ecc_error`、`uncorrectable_ecc_error` 均为 `Option<Option<bool>>`）。
- Produces: `redfish_memory_correctable_errors{bmc,system,id}`、`redfish_memory_uncorrectable_errors{bmc,system,id}`（0/1）。

- [ ] **Step 1: 写失败测试**

`tests/collector_proc_mem_test.rs` 追加（先看该文件现有 helper 复用）：
```rust
#[tokio::test]
async fn collects_memory_ecc_alarm_trips() {
    // 复用文件内 expect_service_root / expect_systems 风格 helper；
    // Memory 集合成员带 Metrics 链接，Metrics 响应含:
    // "HealthData": { "AlarmTrips": { "CorrectableECCError": true, "UncorrectableECCError": false } }
    // 断言 redfish_memory_correctable_errors == 1.0、redfish_memory_uncorrectable_errors == 0.0
}
```
按 `tests/collector_proc_mem_test.rs` 现有 mock 顺序写完整 expectations（该文件已有 memory 测试，参考其 helper）。

- [ ] **Step 2: 实现**

`src/collector/memory.rs` 的 `collect_module` 中，在 push_value(MEMORY_BANDWIDTH) 之后追加：
```rust
let raw_metrics = metrics.raw();
if let Some(health_data) = &raw_metrics.health_data {
    if let Some(trips) = &health_data.alarm_trips {
        if let Some(Ok(v)) = trips.correctable_ecc_error.as_ref().map(|b| b.as_ref()) {
            push_bool(out, bmc_name, system_id, &id, MEMORY_CORRECTABLE, v);
        }
        if let Some(Ok(v)) = trips.uncorrectable_ecc_error.as_ref().map(|b| b.as_ref()) {
            push_bool(out, bmc_name, system_id, &id, MEMORY_UNCORRECTABLE, v);
        }
    }
}
```
`src/metrics.rs` 加：
```rust
pub const MEMORY_CORRECTABLE: (&str, &str) = (
    "redfish_memory_correctable_errors",
    "Memory correctable ECC alarm trip, 1 = tripped",
);
pub const MEMORY_UNCORRECTABLE: (&str, &str) = (
    "redfish_memory_uncorrectable_errors",
    "Memory uncorrectable ECC alarm trip, 1 = tripped",
);
```
`push_bool` 用现有 `push_value` 模式（label bmc/system/id）。

- [ ] **Step 3: 测试通过**

Run: `cargo test --test collector_proc_mem_test`
预期：PASS。

- [ ] **Step 4: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/collector/memory.rs src/metrics.rs tests/collector_proc_mem_test.rs
git commit -m "feat: memory ECC alarm trip counters"
```

---

### Task 11: PSU 明细与功耗统计

**Files:**
- Modify: `src/collector/power.rs`
- Modify: `src/metrics.rs`
- Test: `tests/collector_power_test.rs`

**Interfaces:**
- Consumes: `PowerControl.power_metrics: Option<PowerMetrics>`（`interval_in_min`/`min_consumed_watts`/`max_consumed_watts`/`average_consumed_watts` 已确认）；`PowerSupply` 的 `efficiency_percent`、`input_voltage`、`capacity_watts`、`power_input_watts` 字段（以 Step 2 核对为准）。
- Produces:
  - `redfish_power_consumption_min_watts` / `_max_watts` / `_avg_watts`（bmc, chassis）
  - `redfish_power_consumption_interval_minutes`（bmc, chassis）
  - `redfish_power_supply_efficiency_percent` / `_input_watts` / `_capacity_watts` / `_input_voltage`（bmc, chassis, id）

- [ ] **Step 1: 核对 PowerSupply 字段名**

`rg -n "pub struct PowerSupply"` 附近核对：`efficiency_percent`、`input_voltage`、`capacity_watts`、`power_input_watts` 是否存在及其类型（`Option<Option<Decimal>>`）。记录实际名。

- [ ] **Step 2: 写失败测试**

`tests/collector_power_test.rs` 追加：
```rust
#[tokio::test]
async fn collects_power_metrics_and_psu_details() {
    // 复用现有 expect_service_root/expect_chassis_collection；
    // Chassis 带 Power 链接，Power 响应含:
    // "PowerControl": [{ "Name": "Total", "PowerConsumedWatts": 320.0,
    //     "PowerMetrics": { "IntervalInMin": 10, "MinConsumedWatts": 280.0,
    //                        "MaxConsumedWatts": 350.0, "AverageConsumedWatts": 310.0 } }],
    // "PowerSupplies": [{ "@odata.id": "/redfish/v1/Chassis/1/Power#/PowerSupplies/1", ... }]
    // PowerSupplies/1 响应含:
    // "Name": "PSU1", "MemberId": "1", "PowerOutputWatts": 150.0,
    // "EfficiencyPercent": 94.0, "InputVoltage": 230.0, "CapacityWatts": 750.0,
    // "PowerInputWatts": 160.0
    // 断言 4 个 power_consumption_* 与 4 个 power_supply_* 指标存在且值正确
}
```
（按该文件现有 mock 顺序写全 expectations。）

- [ ] **Step 3: 实现**

`src/collector/power.rs`：
- `collect_legacy_power` 中在 `power_consumed_watts` 成功分支追加：
```rust
if let Some(pm) = &power_control.power_metrics {
    push_power_stat(out, bmc_name, &chassis_id, POWER_CONSUMPTION_MIN, unbox_reading(pm.min_consumed_watts));
    push_power_stat(out, bmc_name, &chassis_id, POWER_CONSUMPTION_MAX, unbox_reading(pm.max_consumed_watts));
    push_power_stat(out, bmc_name, &chassis_id, POWER_CONSUMPTION_AVG, unbox_reading(pm.average_consumed_watts));
    push_power_stat(out, bmc_name, &chassis_id, POWER_CONSUMPTION_INTERVAL,
        pm.interval_in_min.flatten().map(|v| v as f64));
}
```
- `legacy_power_supplies` 中在 push_sensor_reading 之后追加（用 `supply.id().to_string()` 或 `base.member_id` 作 id）：
```rust
let id = supply.base.member_id.clone();
push_psu_metric(out, bmc_name, &chassis_id, &id, POWER_SUPPLY_EFFICIENCY, unbox_reading(supply.efficiency_percent));
push_psu_metric(out, bmc_name, &chassis_id, &id, POWER_SUPPLY_INPUT_WATTS, unbox_reading(supply.power_input_watts));
push_psu_metric(out, bmc_name, &chassis_id, &id, POWER_SUPPLY_CAPACITY, unbox_reading(supply.capacity_watts));
push_psu_metric(out, bmc_name, &chassis_id, &id, POWER_SUPPLY_INPUT_VOLTAGE, unbox_reading(supply.input_voltage));
```
（`push_power_stat`/`push_psu_metric` 为带 bmc/chassis[/id] 标签的小 helper；字段名按 Step 1 核对调整。）
`src/metrics.rs` 加 8 个常量（名称如上，help 中文或英文均可，遵循现有风格）。

- [ ] **Step 4: 测试通过**

Run: `cargo test --test collector_power_test`
预期：PASS。

- [ ] **Step 5: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/collector/power.rs src/metrics.rs tests/collector_power_test.rs
git commit -m "feat: PSU detail and power consumption stats metrics"
```

---

### Task 12: build_info 与 scrape_errors_total 自观测

**Files:**
- Modify: `src/metrics.rs`
- Modify: `src/registry.rs`
- Modify: `src/scraper.rs`
- Test: `tests/scraper_test.rs`

**Interfaces:**
- Consumes: `Snapshot`（现 RwLock 结构）。
- Produces:
  - `redfish_build_info{version}` 值恒 1（每次 build_registry 注入）
  - `redfish_scrape_errors_total{bmc}` = 该 BMC 累计失败资源数（跨轮累计；每轮失败资源数累加）
  - `Snapshot::record_scrape_errors(&self, bmc: &str, count: u64)` 与 `Snapshot::scrape_errors(&self, bmc: &str) -> u64`

- [ ] **Step 1: 写失败测试**

`tests/scraper_test.rs` 追加：
```rust
#[tokio::test]
async fn registry_includes_build_info_and_error_total() {
    let reg = build_registry(
        "bmc1",
        &ScrapeReport { metrics: vec![], failed_resources: vec!["sensors".into()] },
        3,
    )
    .await
    .unwrap();
    let out = encode(&reg);
    assert!(out.contains("redfish_build_info"));
    assert!(out.contains("redfish_scrape_errors_total{bmc=\"bmc1\"} 3"));
}

#[tokio::test]
async fn snapshot_tracks_scrape_errors() {
    let snap = Snapshot::new();
    snap.record_scrape_errors("bmc1", 2);
    snap.record_scrape_errors("bmc1", 1);
    assert_eq!(snap.scrape_errors("bmc1"), 3);
}
```
（`build_registry` 增加第三参 `error_total: u64` —— 现有两处调用同步修改。）

- [ ] **Step 2: 实现**

`src/registry.rs`：
```rust
pub struct Snapshot {
    inner: RwLock<HashMap<String, Arc<prometheus::Registry>>>,
    errors: Mutex<HashMap<String, u64>>,
}
impl Snapshot {
    pub fn record_scrape_errors(&self, bmc: &str, count: u64) {
        *self.errors.lock().unwrap().entry(bmc.to_string()).or_insert(0) += count;
    }
    pub fn scrape_errors(&self, bmc: &str) -> u64 {
        self.errors.lock().unwrap().get(bmc).copied().unwrap_or(0)
    }
}
pub async fn build_registry(bmc_name: &str, report: &ScrapeReport, error_total: u64) -> Result<Arc<prometheus::Registry>, MetricsError> {
    let registry = prometheus::Registry::new();
    register_into(&report.metrics, &registry)?;
    // ... 现有 up/scrape_error 逻辑 ...
    let build = Metric::gauge(BUILD_INFO.0, BUILD_INFO.1)
        .label("version", env!("CARGO_PKG_VERSION").to_string())
        .build(1.0);
    register_into(&[build], &registry)?;
    let errors = Metric::gauge(SCRAPE_ERRORS_TOTAL.0, SCRAPE_ERRORS_TOTAL.1)
        .label("bmc", bmc_name.to_string())
        .build(error_total as f64);
    register_into(&[errors], &registry)?;
    Ok(Arc::new(registry))
}
```
`src/metrics.rs` 加：
```rust
pub const BUILD_INFO: (&str, &str) = ("redfish_build_info", "Build information");
pub const SCRAPE_ERRORS_TOTAL: (&str, &str) = (
    "redfish_scrape_errors_total",
    "Total number of failed resources across all scrape rounds",
);
```
`src/scraper.rs`：快路径与失败路径（`Err` 分支）调用 `build_registry` 时：
- 成功报告：`let n = report.failed_resources.len() as u64; self.snapshot.record_scrape_errors(&name, n); build_registry(&name, &report, self.snapshot.scrape_errors(&name)).await`
- 任务失败（整个 BMC 失败）：`self.snapshot.record_scrape_errors(&name, 1);`（代表 1 次整轮失败）后 build。

- [ ] **Step 3: 修现有测试**

`tests/scraper_test.rs` 与 `tests/integration_test.rs` 中 `build_registry` 调用点补第三参（失败前先看现有调用，统一传 `0` 或对应值）。

- [ ] **Step 4: 测试通过**

Run: `cargo test`
预期：全绿。

- [ ] **Step 5: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/metrics.rs src/registry.rs src/scraper.rs tests/scraper_test.rs tests/integration_test.rs
git commit -m "feat: build_info and persistent scrape_errors_total metrics"
```

---

### Task 13: 会话 shutdown 清理

**Files:**
- Modify: `src/bmc.rs`
- Modify: `src/scraper.rs`
- Test: `tests/bmc_test.rs`

**Interfaces:**
- Consumes: nv `Session::delete(&self)`（已确认存在）、`establish_session` 现有流程。
- Produces:
  - `bmc.rs`: `pub struct EstablishedSession<B: Bmc> { pub token: String, pub session: Option<Arc<nv_redfish::session_service::Session<B>>> }`；`establish_session` 返回值从 `Result<String, BmcError>` 改为 `Result<EstablishedSession<B>, BmcError>`。
  - `scraper.rs`: `Scraper::run` 的 stop 分支删除所有已建会话（仅 session 认证 BMC）。

- [ ] **Step 1: 写失败测试**

`tests/bmc_test.rs` 追加：
```rust
#[tokio::test]
async fn establish_session_returns_token_and_session() {
    // 现有测试的 mock expectations 复用；create_session 的 Expect 返回 token+location
    // 断言返回的 EstablishedSession.token 非空、session.is_some()
}

#[tokio::test]
async fn session_delete_issues_delete_request() {
    // 用 Expect::create_session 建会话，随后 Expect::delete 会话 URL
    // 调用 session.delete() 成功
}
```
（按 `tests/bmc_test.rs` 现有 mock 模式写全 expectations。）

- [ ] **Step 2: 实现 bmc.rs**

```rust
pub struct EstablishedSession<B: nv_redfish::Bmc> {
    pub token: String,
    pub session: Option<Arc<nv_redfish::session_service::Session<B>>>,
}

pub async fn establish_session<B: nv_redfish::Bmc>(
    bmc: &Arc<B>,
    username: &str,
    password: &str,
) -> Result<EstablishedSession<B>, BmcError> {
    // 现有流程不变，末尾改为：
    let token = session.auth_token().ok_or_else(|| BmcError::Session("session response without auth token".into()))?;
    Ok(EstablishedSession { token: token.to_string(), session: Some(Arc::new(session)) })
}
```
（`Session` 是否 Send+Sync 需编译验证；若 `Arc<Session<B>>` 不可用，改为保存 `Option<ODataId>`（location），shutdown 时经 `manager.log_services()` 路径不可行，则用 `NavProperty::new_reference(location).get()` 重建 —— 以编译结果为准，测试相应调整。）

- [ ] **Step 3: 实现 scraper 会话持有与清理**

`src/scraper.rs`：
- `Scraper` 加 `sessions: Mutex<HashMap<String, Option<Arc<nv_redfish::session_service::Session<HttpBmc<ReqwestClient>>>>>>`（仅 session 认证 BMC 记录）。
- `Scraper::new`：session 认证分支改用新返回值，把 session 存入。
- 401 重登分支：重登成功后更新 token（`set_credentials` 不变）并更新存储的 session；旧会话（已 401 过期）不删（记录说明）。
- `run` 的 stop 分支：
```rust
_ = stop.changed() => {
    for (name, session) in self.sessions.lock().unwrap().iter() {
        if let Some(s) = session {
            if let Err(e) = s.delete().await {
                warn!(bmc = %name, error = %e, "session cleanup failed");
            }
        }
    }
    break;
}
```
（`run` 是 async 闭包内，需 `self` 所有权调整 —— 将 `self.sessions` 提前 clone 进闭包或重构；以编译为准。）

- [ ] **Step 4: 修现有测试**

`tests/bmc_test.rs`、`tests/integration_test.rs`、`tests/scraper_test.rs` 中 `establish_session` 调用点改用 `.token` 字段。

- [ ] **Step 5: 测试通过**

Run: `cargo test`
预期：全绿。

- [ ] **Step 6: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/bmc.rs src/scraper.rs tests/bmc_test.rs tests/integration_test.rs tests/scraper_test.rs
git commit -m "feat: delete Redfish sessions on graceful shutdown"
```

---

### Task 14: 404 快速跳过与计数

**Files:**
- Create: `src/collector/error.rs`
- Modify: `src/collector/mod.rs`（collect_fast/collect_slow 的 collector 错误处理）
- Modify: `src/collector/*.rs`（7 个 collector 的 `map_err` 改走 helper；机械替换）
- Test: `tests/collector_errors_test.rs`

**Interfaces:**
- Consumes: `nv_redfish::Error<ConcreteBmc>` 的 `BmcError::InvalidResponse { status, .. }` 模式（404 可精确匹配，`ConcreteBmc = HttpBmc<ReqwestClient>`）。
- Produces:
  - `error.rs`: `pub type ConcreteBmc = HttpBmc<ReqwestClient>;` 与 `pub fn is_not_found(err: &nv_redfish::Error<ConcreteBmc>) -> bool`。
  - 行为变化：`collect_round` 中 ServiceRoot 404 → 跳过整轮（up=0 快照，不计 failed）；collector 内部错误维持现状（String 化后无法判 404，接受局限并记入差距表）。

- [ ] **Step 1: 写失败测试**

`tests/collector_errors_test.rs`：
```rust
use nv_redfish::bmc_http::reqwest::BmcError;
use redfish_exporter::collector::error::{CollectError, CollectErrorKind, is_not_found};

#[test]
fn classifies_404_as_not_found() {
    let err = nv_redfish::Error::Bmc(BmcError::InvalidResponse {
        url: "https://h/redfish/v1/Chassis".parse().unwrap(),
        status: reqwest::StatusCode::NOT_FOUND,
        text: "".into(),
    });
    assert!(is_not_found(&err));
}

#[test]
fn classifies_500_as_other() {
    let err = nv_redfish::Error::Bmc(BmcError::InvalidResponse {
        url: "https://h/redfish/v1/Chassis".parse().unwrap(),
        status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        text: "".into(),
    });
    assert!(!is_not_found(&err));
}
```
（`is_not_found` 为具体类型 `nv_redfish::Error<HttpBmc<ReqwestClient>>` 的判别函数；`reqwest` crate 已作为传递依赖可用，但若要直接引用需在 dev-dependencies 或 dependencies 加 `reqwest` —— 它已在 `[dependencies]` 中（Cargo.toml:16），OK。）

- [ ] **Step 2: 实现 error.rs**

```rust
use crate::bmc::{HttpBmc, ReqwestClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectErrorKind { NotFound, Other }

#[derive(Debug)]
pub struct CollectError {
    pub resource: String,
    pub kind: CollectErrorKind,
}

impl std::fmt::Display for CollectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.resource, self.kind)
    }
}

pub type CollectErrorResult<T> = Result<T, CollectError>;

pub fn is_not_found(err: &nv_redfish::Error<HttpBmc<ReqwestClient>>) -> bool {
    matches!(
        err,
        nv_redfish::Error::Bmc(nv_redfish::bmc_http::reqwest::BmcError::InvalidResponse {
            status,
            ..
        }) if *status == reqwest::StatusCode::NOT_FOUND
    )
}

pub fn collect_err(resource: impl Into<String>, e: &nv_redfish::Error<HttpBmc<ReqwestClient>>) -> CollectError {
    CollectError {
        resource: resource.into(),
        kind: if is_not_found(e) { CollectErrorKind::NotFound } else { CollectErrorKind::Other },
    }
}
```
- [ ] **Step 1: 写测试（is_not_found 分类）**

按上文 Step 1 的 4 个断言测试写入 `tests/collector_errors_test.rs`；先运行确认失败（模块不存在）。测试中构造 `BmcError::InvalidResponse` 需 `url: Url` 类型 —— `use url::Url;` 后 `Url::parse("https://h/redfish/v1/Chassis").unwrap()`。

- [ ] **Step 2: 实现 error.rs 与 scraper 接入**

**设计决策（已定，不再推演）**：collector 层保持泛型与 `Result<Vec<Metric>, String>` 现状；404 判定只在 scraper 层可用的地方生效 —— `collect_round`（Task 7）返回具体类型 `nv_redfish::Error<ConcreteBmc>`，`is_not_found` 对其匹配。collector 内部已 String 化的错误无法判别 404，**接受此局限**：除 root 外的 404 仍计为失败资源，结论记入差距表（404 全量跳过列为 backlog，理由：错误经 String 化丢失状态码，判别需要 collector 具体类型化改造，成本大于当前收益；真机 404 发生率由审计 J 组数据佐证）。

`src/collector/error.rs`：
```rust
use crate::bmc::{HttpBmc, ReqwestClient};

pub type ConcreteBmc = HttpBmc<ReqwestClient>;

pub fn is_not_found(err: &nv_redfish::Error<ConcreteBmc>) -> bool {
    matches!(
        err,
        nv_redfish::Error::Bmc(nv_redfish::bmc_http::reqwest::BmcError::InvalidResponse {
            status,
            ..
        }) if *status == reqwest::StatusCode::NOT_FOUND
    )
}
```
`src/collector/mod.rs` 加 `pub mod error;`。`src/scraper.rs` `collect_round` 的 root 错误分支：
```rust
let root = match nv_redfish::ServiceRoot::new(Arc::clone(&bmc)).await {
    Ok(r) => r,
    Err(e) if is_not_found(&e) => {
        debug!(bmc = %name, error = %e, "service root 404, skipping round");
        // 手动构造 up=0 快照（failed 为空 → 不计 scrape_errors_total）。
        // 注意：不可用 finalize_report(name, vec![], vec![], started) —— 它产出 up=1.0，
        // 与 404 跳过语义相反（实施时已确认并修正，勿改回）。
        return Ok(ScrapeReport {
            metrics: vec![Metric::gauge(UP.0, UP.1)
                .label("bmc", name.to_string())
                .build(0.0)],
            failed_resources: vec![],
        });
    }
    Err(e) => return Err(e),
};
```
（404 跳过轮：发布 up=0、不计数错误；`started` 需在 root 获取之后创建。）

- [ ] **Step 3: 测试通过**

Run: `cargo test --test collector_errors_test`
预期：PASS（`is_not_found` 直接构造测试）。

- [ ] **Step 4: 更新差距表**

在 `docs/audit/2026-08-21-gap-analysis.md` 的"可靠性"维度 404 行：结论写"部分实现（root 404 快速跳过）+ 其余 backlog（错误 String 化丢失状态码，判别需具体类型化，成本大于收益）"。

- [ ] **Step 5: 收尾**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
git add src/collector/error.rs src/collector/mod.rs src/scraper.rs tests/collector_errors_test.rs docs/audit/2026-08-21-gap-analysis.md
git commit -m "feat: fast-skip round on service root 404; classify 404 errors"
```

---

# 阶段三：审计门控实现（以差距表结论为准）

### Task 15: 分页支持（门控：审计 J 组发现任何 nextLink）

**门控条件**：`docs/audit/2026-08-21-bmc-audit.md` 第 1 节 J 组结果中，任一台 BMC 任一大集合（SEL/传感器/磁盘/固件/顶层集合）出现 `@odata.nextLink`。若全为 false → **跳过本任务**，在差距表记 backlog（理由：真机无分页响应，收益未证实）。

**Files:**
- Create: `src/pagination.rs`
- Modify: `src/collector/logs.rs`（事件日志用分页）
- Test: `tests/pagination_test.rs`

**Interfaces:**
- Produces: `pub async fn fetch_all_pages<B: Bmc>(bmc: &Arc<B>, url: &nv_redfish::core::ODataId) -> Result<Vec<serde_json::Value>, nv_redfish::Error<B>>`：
  - 用 raw fetch 模式：`#[derive(Deserialize)] struct Page { #[serde(rename = "Members")] members: Option<Vec<serde_json::Value>>, #[serde(rename = "@odata.nextLink")] next_link: Option<String> }` + `impl EntityTypeRef for Page`（参考 nv 内部 patch_support/payload.rs 的 ~10 行模式）。
  - 循环：`bmc.get::<Page>(...)` → 收集 members → 有 next_link 则继续（next_link 相对路径拼 host）。
  - 无分页时与现有行为等价（单页）。

- [ ] **Step 1: 写失败测试**

`tests/pagination_test.rs`：
```rust
use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::pagination::fetch_all_pages;
use serde_json::json;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

#[tokio::test]
async fn walks_next_links() {
    let bmc = Arc::new(Mock::default());
    let url: nv_redfish::core::ODataId = "/redfish/v1/Chassis/1/Sensors".into();
    bmc.expect(Expect::get("/redfish/v1/Chassis/1/Sensors", json!({
        "@odata.id": "/redfish/v1/Chassis/1/Sensors", "Members": [{"a": 1}],
        "@odata.nextLink": "/redfish/v1/Chassis/1/Sensors?$skip=25",
    })));
    bmc.expect(Expect::get("/redfish/v1/Chassis/1/Sensors?$skip=25", json!({
        "@odata.id": "/redfish/v1/Chassis/1/Sensors?$skip=25", "Members": [{"b": 2}],
    })));
    let pages = fetch_all_pages(&bmc, &url).await.unwrap();
    assert_eq!(pages.len(), 2);
}
```
先运行确认失败（模块不存在）。

- [ ] **Step 2: 实现 pagination.rs**

（实现细节以 `EntityTypeRef`/`Bmc::get` 的实际签名校准；`next_link` 绝对/相对 URL 处理参照 `nv_redfish::core::ODataId` 的 `service_root()`/拼接语义。）

- [ ] **Step 3: 事件日志接入**

`logs.rs` 的 `entries()` 调用改为 `fetch_all_pages`（先 `raw().entries` 取集合 URL，再分页抓取，解析为 `LogEntry`）；保持其他行为不变。mock 测试相应补 nextLink expectations（或新增分页用例）。

- [ ] **Step 4: 测试通过 + 收尾**

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test
git add src/pagination.rs src/collector/logs.rs tests/pagination_test.rs
git commit -m "feat: @odata.nextLink pagination walker (raw fetch)"
```

---

### Task 16: /discover、/reload、/info 端点（门控：审计"运维体验"结论为需要）

**门控条件**：审计报告"运维"维度未发现反对理由（默认实施；若审计结论为低优先，跳过并记 backlog）。

**Files:**
- Modify: `src/http.rs`
- Modify: `src/main.rs`（config 持有方式改为 `Arc<Config>` + reload 通道）
- Test: `tests/http_test.rs`

**Interfaces:**
- Produces:
  - `GET /info` → JSON `{"version": env!("CARGO_PKG_VERSION"), "rust_version": env!("CARGO_PKG_RUST_VERSION"), "os": env!("CARGO_CFG_TARGET_OS"), "arch": env!("CARGO_CFG_TARGET_ARCH")}`
  - `GET /discover` → Prometheus HTTP SD 格式 `[{"targets": ["<host>", ...]}]`（来自 config bmcs host）
  - `POST /reload` → 重新读取配置文件并热替换；校验失败返回 400 + 错误信息，保留旧配置
  - router 签名改为 `router(snapshot: Arc<Snapshot>, cfg: Arc<RwLock<Config>>, config_path: PathBuf)`

- [ ] **Step 1: 写失败测试**

`tests/http_test.rs` 追加（现有 axum 测试模式）：
```rust
#[tokio::test]
async fn info_endpoint_reports_build_info() { ... }
#[tokio::test]
async fn discover_endpoint_lists_targets() { ... }
```
（reload 的端到端测试依赖文件系统与配置热换，做 handler 层测试：`/reload` 无 config_path 时 400；有路径时替换 state 中 Config。）

- [ ] **Step 2: 实现**

- `http.rs`：路由加 `/info`、`/discover`、`/reload`；state 改为 `(Arc<Snapshot>, Arc<RwLock<Config>>, Option<PathBuf>)` 或小结构体 `AppState`。
- `main.rs`：`let cfg = Arc::new(RwLock::new(cfg));` 传 router；`/reload` 内 `Config::load_config(path)` → 校验 → 替换；scraper 重建问题：**本任务仅替换配置对象供 /discover 使用**；scraper 重建（BMC 增删/凭据变更生效）列为 backlog（差距表记录"热加载部分实现：配置可替换，BMC 变更需重启生效"）。`/reload` 响应中说明。

- [ ] **Step 3: 测试通过 + 收尾**

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test
git add src/http.rs src/main.rs tests/http_test.rs
git commit -m "feat: /info, /discover and /reload endpoints"
```

---

### Task 17: 驱动器 OEM 指标（门控：审计 L 组发现 Dell/浪潮 OEM 寿命字段）

**门控条件**：`docs/audit/2026-08-21-bmc-audit.md` L 组结果中，任一台 BMC 的 Drive 资源 `Oem.*` 含可解析的寿命/指示灯字段（如浪潮 `Oem.Public.*Life*`、Dell `Oem.Dell.*`）。无 → 跳过并记 backlog。

**Files:**
- Modify: `Cargo.toml`（features 加 `"oem"`，如浪潮 `oem-inspur` 不存在则用 `oem` 通用）
- Modify: `src/collector/storage.rs`
- Modify: `src/metrics.rs`
- Test: `tests/collector_storage_test.rs`

**Interfaces:**
- Consumes: `nv_redfish::oem::oem_value`（或 `oem_object`）读取 `Drive.raw().oem` 中目标字段；字段路径以审计快照 `H-drive-oem` 记录为准。
- Produces: `redfish_drive_life_left_percent{bmc,system,id}`、`redfish_drive_indicator_active{bmc,system,id}`（若字段存在）。

- [ ] **Step 1: 记录字段路径**

从 `out/<bmc>/H.json` 的 `H-drive-oem` 结果确认字段路径（例：`Oem.Public.DriveLifeLeft`）。若两厂商均无 → **跳过本任务**，差距表记 backlog，commit 无变化。

- [ ] **Step 2: 写失败测试 + 实现**

按审计确认的 JSON 形状写 mock 测试（Drive 响应含对应 `Oem` 字段），storage.rs 用 `oem_value` 读取并输出指标；字段名/路径与审计快照一致。

- [ ] **Step 3: 测试通过 + 收尾**

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test
git add Cargo.toml Cargo.lock src/collector/storage.rs src/metrics.rs tests/collector_storage_test.rs
git commit -m "feat: vendor OEM drive metrics (life left / indicator)"
```

---

### Task 18: 文档收尾与全量验证

**Files:**
- Modify: `docs/metrics.md`（新增指标族）
- Modify: `README.md`（配置表加 slow_interval、端点表加新端点）
- Modify: `config.example.yaml`（加 slow_interval 示例注释）
- Modify: `docs/design.md`（分频架构、错误模型、指标清单）
- Modify: `src/metrics.rs`（清理过时 `#[allow(dead_code)]` 注释，D10）
- Test: 全量

- [ ] **Step 1: 更新文档**

`docs/metrics.md` 追加 Task 8–12/17 新增指标；`README.md` 配置表加 `slow_interval`（默认 null = 不分频）与端点表（/info、/discover、/reload，若 Task 16 实施）；`config.example.yaml` 加注释示例；`docs/design.md` 更新架构图说明（快/慢组、会话清理、自观测指标）。

- [ ] **Step 2: 清理过时注释**

`src/metrics.rs`：删除 `SCRAPE_ERROR`/`PROCESSOR_UTILIZATION`/`DRIVE_UTILIZATION` 上的 `#[allow(dead_code)]` 与"0.1.0 未实现"注释（核实实际使用情况后处理：未使用的常量删除或实现）。

- [ ] **Step 3: 全量验证**

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
预期：全绿 + release 构建成功。

- [ ] **Step 4: 真机复验（可选，需用户在场确认）**

按 Task 4 方式实跑一次，确认新增指标族在真机上出现（事件日志/BIOS/内存纠错/PSU 明细/功耗统计），记录到审计报告"复验"小节；凭据仍走临时目录。

- [ ] **Step 5: 提交**

```bash
git add docs/metrics.md README.md config.example.yaml docs/design.md src/metrics.rs
git commit -m "docs: refresh metrics reference, config docs and design for new features"
```

---

# 自检清单（写完后人工核对）

- **Spec 覆盖**：spec 第 4 节探测矩阵 A–L → Task 2/3；实跑 → Task 4；审计报告 → Task 5；差距表 → Task 6；分频 → Task 7；事件日志 → Task 8；BIOS → Task 9；内存纠错 → Task 10；PSU/功耗 → Task 11；自观测 → Task 12；会话清理 → Task 13；404 → Task 14；分页/端点/OEM → Task 15/16/17（门控）；文档 → Task 18。backlog 项（Vault、共享预取、PDU、SSE、健康值数值映射、凭据脚本）在差距表记结论，无实现任务 —— 与 spec 第 7 节一致。
- **类型一致性**：`collect_fast`/`collect_slow`/`merge_reports`/`finalize_report`（Task 7）被 scraper（Task 7 内）与 tests 引用；`is_not_found`（Task 14）被 scraper 引用；`build_registry` 三参签名（Task 12）影响 scraper 与所有测试调用点 —— 每任务收尾的 `cargo test` 会捕获不一致。
- **占位符**：Task 8/9 的 schema 字段名标注"以核对结果为准"是有意的验证步骤（先启用特性再确认生成类型），不是占位符。
