# Engineering Quality Audit — Design

Date: 2026-08-21
Status: approved (design sections 1–3 confirmed in chat)
Goal: benchmark redfish-exporter against three popular open-source Redfish
exporters, find and fix our defects on real hardware, and absorb their
strengths — without breaking the nv-redfish dependency chain and without
re-implementing anything nv-redfish already provides.

## Reference projects

| Project | Stack | Notable strengths |
|---|---|---|
| mrlhansen/idrac_exporter | Go | richest metric families (event log, controllers, RAID battery, PSU details, PDU, Dell OEM), `/reload`, `/discover` (Prometheus SD), build info + `scrape_errors_total`, Helm chart, Grafana dashboards |
| comcast/fishymetrics | Go | partial scraping, 404 retry policy (disable-404-retry), Vault/credentials script, HTTP proxy support, `/info`, module exclude flags |
| sapcc/redfish-exporter | Python | wide vendor test matrix (22+ models), per-family endpoints, memory correctable/uncorrectable error counts, BIOS attributes as metrics, `redfish_response_duration_seconds`, env-var credential override |

## Constraints

1. **nv gate**: if nv-redfish 0.15 already provides the capability (types,
   endpoints, parsing), use it as-is. Only implement ourselves what nv does
   not provide, at the exporter layer (orchestration, caching, HTTP policy).
2. **Dependency chain gate**: never fork or patch nv-redfish source; never
   replace the nv types in the core scrape pipeline; no new heavyweight
   dependencies for what nv already does.
3. **Benefit gate**: a candidate improvement enters the implementation plan
   only if real-device probing shows actual data exists to collect — no
   alignment for alignment's sake.
4. Read-only Redfish usage against the real BMCs: no POST/PATCH/CREATE on
   live resources. Credentials live only in git-ignored temp files, never in
   the repo, never in logs.

## Real-device targets

- `https://10.10.90.70/` — Dell PowerEdge R750 (iDRAC), user `root`
- `https://10.10.90.80/` — Inspur server, user `admin`
- Credentials provided by operator; stored only in
  `C:\Users\franck\AppData\Local\Temp\opencode\redfish-audit\` (gitignored).

## Section 1 — Empirical audit execution

**A. Real-device API probing (Python 3 stdlib script, throwaway, temp dir)**
- Basic-auth probing of both BMCs: walk `ServiceRoot -> collections`, fetch
  JSON snapshots of the key resources:
  Systems / Chassis / Managers / Thermal / Power / Sensors / Processors /
  Memory / Storage (Drives, Volumes) / NetworkAdapters (EthernetInterfaces,
  Ports) / PCIeDevices / UpdateService (FirmwareInventory) / LogService
  (EventLog) / Bios / EnvironmentMetrics / Controls.
- Output per-resource report: HTTP status, latency, field presence.
- Snapshots stored under `Temp\opencode\redfish-audit\<bmc-name>\`.

**B. Run our exporter and cross-compare**
- `cargo run` locally (Windows), config for both BMCs with `session` auth,
  `scrape_interval=30s`.
- Collect >=3 rounds of `/metrics` output and `info/debug` logs; record per
  BMC: metric family count, `redfish_up`, `failed_resources` errors, scrape
  duration.
- Session auth is the default test mode; if a vendor rejects session
  establishment (Inspur is a known risk), fall back to basic auth for that
  BMC and record the behavior as a finding (compatibility defect), not as a
  config error.
- Cross-compare A (API surface) vs B (exported metrics): API exists but no
  metric -> coverage defect; abnormal values -> parsing defect; request
  failures/timeouts -> reliability defect.

**C. Defect classification** — severity grading:
- P0: data corruption / crash
- P1: missing core metric family / wrong values
- P2: edge missing / vendor compatibility
- P3: experience / polish
- Every defect carries sanitized evidence (request/response/log excerpt).

## Section 2 — Gap analysis table

Artifact: `docs/audit/2026-08-21-gap-analysis.md`. Rows = candidate features
from the three reference projects; columns = reference projects' status, our
current status, nv capability mapping, gap level, recommendation.

Dimensions and candidate features:

| Dimension | Candidate features (sourced from reference projects) |
|---|---|
| Metric coverage | event log entry timestamps (idrac), memory correctable/uncorrectable counts (sapcc), CPU voltage/current speed (idrac), PSU efficiency/input voltage, power min/max/avg + interval (idrac), drive life-left/indicator (idrac), controller/RAID battery health (idrac), BIOS attributes + pending flag (sapcc), PDU (idrac, deferrable), network port status/speed (idrac), system LED/indicator (idrac) |
| Reliability | 404 fast-skip and configurable retry (fishymetrics), pagination handling, per-request failure stats, `redfish_response_duration_seconds` (sapcc) |
| Ops | config hot reload `/reload` (idrac), `/discover` Prometheus SD (idrac), build_info + `scrape_errors_total` (idrac), `/info` (fishymetrics), Grafana dashboard, Helm chart |
| Performance | partial scraping (fishymetrics), shared collection pre-fetch (remove per-collector re-walks), no-404-retry |
| Security | env-var credential override (sapcc), Vault/credentials script (fishymetrics — evaluate cost, likely backlog), HTTP proxy support (fishymetrics), listen path hardening |

Three-gate decision per candidate (see Constraints 1–3).

## Section 3 — Deliverables & acceptance

1. `docs/audit/2026-08-21-bmc-audit.md` — real-device audit report: API baseline
   for both BMCs, exporter run results, defect list P0–P3 with evidence,
   cross-comparison conclusions.
2. `docs/audit/<date>-gap-analysis.md` — gap table with three-gate outcomes.
3. This design doc committed to `docs/superpowers/specs/`.
4. Implementation plan produced via the writing-plans skill.

Acceptance criteria:
- Every P0/P1 defect is reproducible on real device or the nv mock.
- Every gap-table row has an explicit three-gate verdict and recommendation
  (implement / backlog / reject with reason).
- No claim without raw data backing; no fabricated findings.
