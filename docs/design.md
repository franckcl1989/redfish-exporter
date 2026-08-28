# Design

Design overview of redfish-exporter 0.1.0 (Rust, nv-redfish 0.15.1).

## Architecture

```
                    +----------------------------------------------------------+
                    |                        main.rs                            |
                    |  clap args -> config -> Snapshot -> Scraper -> serve     |
                    |  shutdown signal (SIGINT/SIGTERM/Ctrl+C) via watch       |
                    +-----------+----------------------------------+-----------+
                                |                                  |
                   watch: stop |                                  | watch: stop
                                v                                  v
        +---------------------------+                 +---------------------------+
        |          scraper.rs       |                 |           http.rs         |
        |  interval tick loop       |                 |  axum router             |
        |  JoinSet: one task per    |                 |  GET /metrics            |
        |  BMC, per-group deadlines |                 |  GET /healthz, /readyz   |
        |  = scrape_timeout         |                 |  GET /info               |
        |  fast group every round   |                 |                          |
        |  slow group per           |                 +-------------+-------------+
        |  slow_interval (cached)   |                               |
        |  build_registry per BMC   |                               | snapshot.registries()
        +-------------+-------------+                               |
                      | snapshot.update(name, Arc<Registry>)        |
                      v                                             |
        +---------------------------+                               |
        |        registry.rs        |  RwLock<HashMap<String,      |
        |  Snapshot (per-BMC swap)  |   Arc<Registry>>>             |
        +---------------------------+<------------------------------+
```

Per-BMC scrape pipeline (each collector in `src/collector/`):

```
ServiceRoot
  ├─ sensors.rs   chassis sensor links + thresholds
  ├─ power.rs     legacy Thermal/Power, power supplies (+ stats), EnvironmentMetrics, Controls
  ├─ processors.rs ProcessorMetrics (temp/power/bandwidth)
  ├─ memory.rs    Memory capacity + MemoryMetrics (bandwidth, ECC alarm trips)
  ├─ storage.rs   Drive metrics + Volume capacity
  ├─ network.rs   EthernetInterface + PCIe devices + NetworkAdapter ports
  ├─ logs.rs      LogService entries (paginated via nextLink)
  ├─ bios.rs      BIOS attributes + pending-settings flag
  └─ systems.rs   System/Chassis/Manager health+info, Assembly, Firmware inventory
        └─ all push health (redfish_health_status) and info (redfish_info)
```

Fast group (every round): sensors, power, processors, memory, system/chassis/manager health. Slow group (at most once per `slow_interval`, default every round): storage, network, firmware, assembly, event logs, BIOS. Slow-group results are cached across rounds and last-good output survives a failed slow collection.

## Component responsibilities

| Module | Responsibility |
|--------|----------------|
| `config.rs` | YAML parsing, duration deserialization (`humantime`), validation, `SecretString` redaction |
| `bmc.rs` | Per-BMC reqwest client construction (timeout, TLS, CA bundle, `insecure_skip_verify`), `HttpBmc` handle creation, generic session establishment, 401/404 status matching |
| `collector/*.rs` | Fetch Redfish resources through nv-redfish and convert them into `Metric` values; one module per resource family (including `logs.rs` event-log entries and `bios.rs` BIOS attributes) |
| `collector/mod.rs` | `collect_fast` / `collect_slow`: orchestrate the fast (every round) and slow (per `slow_interval`) collectors against one BMC, aggregate metrics and `failed_resources`; `finalize_report` appends `redfish_up` / `redfish_scrape_duration_seconds`; `merge_reports` merges group results; shared `push_health` / `push_info` helpers |
| `pagination.rs` | Raw-fetch nextLink walker (`fetch_all_pages`), supporting both `@odata.nextLink` and `Members@odata.nextLink`; used by the event-log collector |
| `scraper.rs` | Periodic loop; spawns one task per BMC (`JoinSet`) with a **per fast/slow-group deadline** (`with_deadline` on each group); fast group every round, slow group per `slow_interval` with last-good caching; 401 → re-establish session and retry the round once; deletes established sessions on shutdown; builds per-BMC registries and publishes them to the snapshot |
| `registry.rs` | `Snapshot`: per-BMC registries plus one atomically replaced, unified Prometheus exposition; adds `redfish_scrape_error`, failed-round `redfish_up=0`, one global `redfish_build_info{version}` and the cumulative `redfish_scrape_errors_total{bmc}` counter |
| `http.rs` | axum router serving cached `/metrics`, liveness `/healthz`, first-snapshot readiness `/readyz`, and `/info`; bounded graceful shutdown |
| `main.rs` | CLI parsing, wiring, signal handling, exit codes |

## Data flow and error model

- **BMC-level isolation**: each round, every BMC is scraped in its own `JoinSet` task producing its own `Result`. A failing BMC yields `redfish_up{bmc}=0` and does not affect other BMCs. A BMC that never produced a snapshot is absent from `/metrics` (with `x-redfish-exporter: no-data-yet` when none has completed). `/readyz` remains 503 until all configured BMCs have published one success or failure snapshot.
- **Resource-level isolation**: a failing collector does not abort the whole BMC. It records the failing resource in `failed_resources`, which produces `redfish_scrape_error{bmc,resource}=1`, forces `redfish_up{bmc}=0` (applied in `registry.rs`), while the metrics of all other resources are still published. Sub-resource fetches inside a collector (sensor links, drives, volumes, ports) follow the same rule: a failed fetch skips that sub-resource only, and the collector returns `Err` only when *all* of a sub-resource group failed (e.g. all sensor links of a chassis).
- **Narrow firmware compatibility**: IEIT/Inspur vendor response 17034 (synthetic RAID member with no installed controller) is treated as an absent optional storage resource only when the HTTP status, vendor code, and message all match. Annotated `NetworkAdapters` members that contain `@odata.id` but omit inline `Id` are re-read as references and resolved individually; unrelated 500 or parse failures remain observable.
- **Per fast/slow-group deadline**: the fast group and the slow group of each BMC each run under their own `with_deadline(scrape_timeout)` (Task 6A). On expiry the group yields a `fast:timeout`/`slow:timeout` failure that publishes `redfish_up=0` plus `redfish_scrape_error{resource="fast:timeout"}=1` (or `resource="slow:timeout"`) and increments `redfish_scrape_errors_total`, so a slow group neither delays the other group nor silently drops the BMC's series (fixes AUDIT-1/3). A timed-out slow group keeps its last-good cache; a timed-out fast group publishes its failure snapshot until a later round succeeds.
- **Slow group**: collectors that dominate round time on real hardware (storage, network, firmware, assembly, event logs, BIOS — see audit §5) run at most once per `slow_interval` (default: every round). Results are cached in the scraper (`slow_state`) and last-good output survives a failed slow collection.
- **ServiceRoot failure semantics**: every ServiceRoot failure, including 404, is a failed BMC round. It publishes `redfish_up=0`, increments `redfish_scrape_errors_total`, and advances adaptive cooldown; resource-level 404s remain isolated to their collector.
- **Snapshot consistency**: the registry per BMC is built fresh each round. `Snapshot::update` merges all metric families into one valid exposition (one HELP/TYPE block per family and one global build-info sample) and atomically replaces the cached bytes.
- **Pre-encoded snapshot cache**: the unified exposition is encoded once at publish time into immutable `Bytes`; `/metrics` only clones that reference. Encoding equivalence and multi-BMC descriptor uniqueness are pinned by `tests/http_test.rs`.

## Resource model (resources)

- **Per-BMC memory**: each published BMC snapshot retains its Prometheus registry; one unified encoded copy is retained for all BMCs. The release resource test verifies stable encoded-size increments and non-decreasing RSS as BMC count grows.
- **Bounded growth**: snapshot memory grows linearly with the BMC count; the worst-case per-BMC snapshot size is bounded by the pagination defense limits (64MiB per page, 1000 pages, 200,000 members — security hardening).
- **CPU**: request-period CPU/allocations were eliminated by the pre-encoded cache (performance hardening); collect-period parse cost is serde-bound. Measured three-state profile: idle 0.2% / scrape round 0.1% / /metrics serving 0.25% (all windows ≤1.1%; process CPU is dominated by BMC-request waiting).
- **Connections**: keepalive/pooling confirmed — Dell holds a single long-lived connection fully reused (cross-round survival observed); Inspur connections are reused but the BMC periodically closes them (BMC-side keepalive timeouts; single-connection lifetime observed ≥44.2s, window 44.2–55.0s under 5s sampling), so the nv-redfish internal client periodically re-establishes connections — expected client behavior, no exporter-side action needed.
- **Request volume**: ~96/~101 requests per full round (Dell/Inspur; fast group ~42/~40, fast+slow collection surface); ETag caching, fast/slow split and snapshot caching keep BMC-side load below re-walking benchmarks (zero-cache full re-crawl).

## Adaptive scheduling (stability)

Per-BMC state machine (`src/stability.rs`, wired in `src/scraper.rs`):

- **Healthy** — normal collection; consecutive failed rounds (round error OR any `failed_resources`, i.e. `up=0` rounds) are counted.
- **Cooling** — after `stability.cooldown_failures` consecutive failures the BMC is fully skipped (no requests); an `up=0` snapshot with `redfish_scrape_error{resource="cooldown"}` is published each round without incrementing `redfish_scrape_errors_total`. Retries follow exponential backoff `min(cooldown_max, cooldown_base × 2^(failures-1))` — first backoff base (60s), doubling per consecutive failure, capped at max: 60s/120s/240s/300s; success resets to Healthy.
- **SessionDegraded** (session-auth BMCs only) — entered when a 401 re-login or session establishment fails: Basic credentials take over collection each round (up=1 achievable, marked with `redfish_scrape_error{resource="session-degraded"}=1`), while session re-establishment retries on exponential backoff; each failed session retry advances that backoff even when Basic collection succeeds. Session success returns to Healthy; consecutive Basic failures lead to Cooling.

Config (frozen semantics for 0.1.0):

```yaml
stability:
  cooldown_failures: 3     # consecutive failure threshold
  cooldown_base: "60s"     # first backoff
  cooldown_max: "300s"     # backoff cap
```

Mutex poisoning is recovered via `crate::recover_lock` (log + continue) instead of cascading panics.

## Known design trade-off (Ruling 7c)

Each collector independently enumerates the `Chassis`/`Systems` collections, so the number of BMC requests per round is roughly **N × (collection walk)** where N = number of collectors (each collector re-fetches chassis/systems members and common collections). At the default 30 s interval this load is acceptable for typical single-BMC-in-band and out-of-band setups, and it keeps collectors strictly isolated (a new collector cannot break the resource walks of existing ones). Consolidating collection walks into a shared pre-fetch phase is a possible future optimization and should be revisited if request volume becomes a problem on large fleets.

## Lifecycle and shutdown

```
main
  └─ start: config -> Scraper::new (clients only; no startup network I/O)
  └─ watch channel (stop: bool)
  └─ serve + scraper.run concurrently
  └─ signal (SIGINT/SIGTERM/Ctrl+C) -> stop_tx.send(true)
       ├─ scraper loop: stop interrupts the current round -> delete established sessions concurrently
       └─ http serve: bounded 25s graceful drain
  └─ join both -> "shutdown complete" -> exit code 0
```

If either subsystem exits unexpectedly, main signals the other, waits up to 30 seconds for cleanup, and exits non-zero. A normal stop-signal shutdown is the only path that exits 0.

## Authentication flow

- `basic`: `BmcCredentials::username_password` is baked into the `HttpBmc`; every request carries HTTP Basic credentials.
- `session`: on the first concurrent scrape, `bmc.rs::establish_session` walks `ServiceRoot -> SessionService -> Sessions`, creates a session, extracts `X-Auth-Token`, and switches credentials to the token. Establishment failure falls back to Basic auth and enters `SessionDegraded`, so the session endpoint is retried with backoff instead of every scrape. A token 401 triggers one re-login and round retry. Established sessions are deleted concurrently on shutdown. Credentials are never logged.

## Inbound security

- Optional bearer-token auth: when `web.auth_token`/`auth_token_file` is set, `/metrics` and `/info` require `Authorization: Bearer <token>`; comparison is constant-time and the authentication scheme is case-insensitive. `/healthz` and `/readyz` intentionally remain public and reveal status only. Configuration changes require a restart.
- Optional server TLS: `serve()` loads a rustls config from `web.tls_cert_file`/`tls_key_file` (fail-fast on error) and serves via axum-server 0.8 (rustls); a 10s HTTP/1.1 header read timeout applies in both modes. The server is fixed to HTTP/1.1 so the timeout also covers zero-byte slow connections. Outbound redirects are limited to 10 hops and may not change scheme, host, or port.
- Implementation note (Windows): `serve_on` sets the listener nonblocking before serving — blocking std listeners hang all accepted-connection I/O under IOCP.
- Pagination defense caps: 64MiB per page (post-deserialization check; nv typed fetch has no streaming API), 1000 pages, 200k accumulated members — violations fail the affected log service (resource-level isolation).

## Config validation rules

From `config.rs` (`load_config`), applied in order:

1. `name` and `username` must be non-empty; names are unique and contain no control characters.
2. `host` must be an absolute `http`/`https` URL with a host and without userinfo, query, or fragment.
3. A non-empty password must come from YAML or the per-BMC environment override.
4. At least one BMC is required.
5. `listen_addr` must parse as a `SocketAddr`.
6. `scrape_interval`, `scrape_timeout`, `request_timeout` must be non-zero (durations via `humantime`, e.g. `30s`).
7. `slow_interval`, if set, must be non-zero.
8. Defaults: `listen_addr=127.0.0.1:9417`, `scrape_interval=30s`, `scrape_timeout=15s`, `request_timeout=10s`, `slow_interval=null` (no frequency splitting), `auth=basic`, `insecure_skip_verify=false`, `ca_cert_file=null`.
9. Client-level HTTP: request timeout from config, 5 s connect timeout, user agent `redfish-exporter/<version>`; `ca_cert_file` loads every certificate in a PEM bundle. HTTP BMCs may not specify TLS options, and `ca_cert_file` is mutually exclusive with `insecure_skip_verify`.
10. Unknown YAML fields at every configuration level are rejected.
11. `web` section: `auth_token` (>=16 chars) XOR `auth_token_file` (content trimmed, >=16 chars); `tls_cert_file` and `tls_key_file` must be set together; password overrides reject empty values and normalized-name collisions.

## Metric naming conventions

- Unit goes into the metric name where the value is a physical quantity: `_watts`, `_percent`, `_bytes`, `_mbps`, `_celsius`, `_seconds`, `_mhz`, `_volts`, `_minutes`.
- Sensor readings keep the unit in the `units` label (`Cel`, `W`, `RPM`, …) because one metric must serve mixed units; thresholds mirror the reading's labels so they can be matched in PromQL (`on(bmc,chassis,id)`). The `id` is the full `@odata.id`, which prevents duplicate display names from collapsing.
- Health/state of every resource is normalized to a single `redfish_health_status` gauge with `health`/`state` labels, so alerts can be written once for all resource types. Generic health/info identity is the full resource `@odata.id`; specialized metrics retain concise native ids plus their parent labels.
- Static inventory is flattened into `redfish_info{resource_type,id,key,value}` so each value remains attributable when a BMC has multiple devices.
- Status values: `health` is normalized to `OK`/`Warning`/`Critical`/`UnsupportedValue`/`unknown`, `state` to its debug name or `unknown`.
- Names ending in `_total` (`redfish_drive_io_*_errors_total`, `redfish_scrape_errors_total`) are registered as Prometheus Counters; `redfish_event_log_entry` carries a Unix timestamp as its value.

## Release and static linking

0.1.0 release target: a static `x86_64-unknown-linux-musl` archive and a distroless non-root GHCR image. The release workflow hard-fails if the binary has a program interpreter, publishes checksums and an SPDX SBOM, and creates GitHub attestations for the archive and container digest.

## Dependency security

- TLS is provided by **rustls** (`reqwest` with `rustls-tls`); the lock file contains no OpenSSL/native-tls dependency.
- The first-party crate forbids unsafe code (`#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`); the dependency tree is selected with no `unsafe`-dependent TLS stack.
- Passwords are never logged: `SecretString` redacts its `Debug` output to `[REDACTED]`.
