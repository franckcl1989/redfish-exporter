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
        |  BMC, per-BMC deadline    |                 |  GET /healthz            |
        |  = scrape_timeout         |                 |  GET /info, /discover    |
        |  fast group every round   |                 |  POST /reload            |
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
| `scraper.rs` | Periodic loop; spawns one task per BMC (`JoinSet`) with a **per-BMC deadline** (`with_deadline`); fast group every round, slow group per `slow_interval` with last-good caching; 401 → re-establish session and retry the round once; deletes established sessions on shutdown; builds per-BMC registries and publishes them to the snapshot |
| `registry.rs` | `Snapshot`: `RwLock<HashMap<String, Arc<Registry>>>` keyed by BMC name with per-BMC atomic replacement; adds `redfish_scrape_error` series, `redfish_up=0` for failed rounds, `redfish_build_info{version}` and the cumulative `redfish_scrape_errors_total{bmc}` counter |
| `http.rs` | axum router serving `/metrics` (per-BMC snapshots concatenated in BMC-name order), `/healthz`, `/info`, `/discover`, `/reload`; `AppState` holds the snapshot, the runtime config (`/reload` hot-replaces the config object) and the config path; graceful shutdown |
| `main.rs` | CLI parsing, wiring, signal handling, exit codes |

## Data flow and error model

- **BMC-level isolation**: each round, every BMC is scraped in its own `JoinSet` task producing its own `Result`. A failing BMC (auth error, unreachable host, …) yields `redfish_up{bmc}=0` and does not affect other BMCs. A BMC that never produced a snapshot is simply absent from `/metrics` (with `x-redfish-exporter: no-data-yet` header when no BMC has ever produced one). Multi-BMC: each BMC's registry is published independently under its own name and `/metrics` concatenates all of them sorted by BMC name, so every BMC keeps being served as long as it completed at least one round.
- **Resource-level isolation**: a failing collector does not abort the whole BMC. It records the failing resource in `failed_resources`, which produces `redfish_scrape_error{bmc,resource}=1`, forces `redfish_up{bmc}=0` (applied in `registry.rs`), while the metrics of all other resources are still published. Sub-resource fetches inside a collector (sensor links, drives, volumes, ports) follow the same rule: a failed fetch skips that sub-resource only, and the collector returns `Err` only when *all* of a sub-resource group failed (e.g. all sensor links of a chassis).
- **Per-BMC deadline**: each BMC task runs under `with_deadline(scrape_timeout)` (Task 6A). On expiry the task yields a `timeout` failure that publishes `redfish_up=0` plus `redfish_scrape_error{resource="timeout"}=1` (and increments `redfish_scrape_errors_total`), so a slow BMC neither delays other BMCs nor silently drops its series (fixes AUDIT-1/3). There is no rollback: BMCs that completed keep their snapshots, and an aborted BMC publishes its failure snapshot until a later round succeeds.
- **Slow group**: collectors that dominate round time on real hardware (storage, network, firmware, assembly, event logs, BIOS — see audit §5) run at most once per `slow_interval` (default: every round). Results are cached in the scraper (`slow_state`) and last-good output survives a failed slow collection.
- **404 fast-skip**: if the ServiceRoot fetch returns 404, the round is skipped and an `up=0` snapshot without failed-resource accounting is published (`is_not_found` in `collector/error.rs`); other 404s still count as failed resources.
- **Snapshot consistency**: the registry per BMC is built fresh each round and swapped in atomically, so `/metrics` always returns one consistent snapshot per BMC and never a partially-written one.

## Adaptive scheduling (stability)

Per-BMC state machine (`src/stability.rs`, wired in `src/scraper.rs`):

- **Healthy** — normal collection; consecutive failed rounds (round error OR any `failed_resources`, i.e. `up=0` rounds) are counted.
- **Cooling** — after `stability.cooldown_failures` consecutive failures the BMC is fully skipped (no requests); an `up=0` snapshot with `redfish_scrape_error{resource="cooldown"}` is published each round without incrementing `redfish_scrape_errors_total`. Retries follow exponential backoff `min(cooldown_max, cooldown_base × 2^n)`; success resets to Healthy.
- **SessionDegraded** (session-auth BMCs only) — entered when a 401 re-login fails: basic credentials take over collection each round (up=1 achievable, marked with `redfish_scrape_error{resource="session-degraded"}=1`), while session re-establishment retries on the same backoff schedule; success returns to Healthy; consecutive basic failures also lead to Cooling.

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
  └─ start: config -> Scraper::new (clients + optional session establishment; failure falls back to basic)
  └─ watch channel (stop: bool)
  └─ serve + scraper.run concurrently
  └─ signal (SIGINT/SIGTERM/Ctrl+C) -> stop_tx.send(true)
       ├─ scraper loop: select! on stop.changed() -> delete established sessions -> break (before next tick)
       └─ http serve: with_graceful_shutdown(stop.changed()) -> drains in-flight requests
  └─ join both -> "shutdown complete" -> exit code 0
```

If the scraper task panics/aborts unexpectedly, main logs the JoinError and exits with code 1 (fail-fast rather than serving stale data). A normal stop-signal shutdown is the only path that exits 0.

## Authentication flow

- `basic`: `BmcCredentials::username_password` is baked into the `HttpBmc`; every request carries HTTP Basic credentials.
- `session`: at startup, `bmc.rs::establish_session` (generic over `B: nv_redfish::Bmc`, so the flow is testable with the mock BMC) walks `ServiceRoot -> SessionService -> Sessions`, POSTs a `SessionCreate`, extracts `X-Auth-Token`, and the caller applies it via `HttpBmc::set_credentials(BmcCredentials::token(...))`. All subsequent requests use the token. If establishment fails at startup the BMC **falls back to basic auth** (warn, no abort — fixes AUDIT-2) and retries session establishment on the first round. On a `401` during a round (`bmc.rs::is_unauthorized` matches `BmcError::InvalidResponse { status: 401, .. }`), the scraper re-establishes the session once (same flow as startup) and retries the round once; if re-establishment or the retry fails, the round is treated as failed (`redfish_up=0`). Established sessions are **deleted on shutdown** (`Session::delete`, scraper stop branch, Task 13) so no server-side sessions leak; the old session of a 401 re-login is already expired and not deleted. Credentials are never logged — re-auth log lines carry only the BMC name and error. Basic-auth BMCs never re-login (Basic has no session to refresh).

## Inbound security

- Optional bearer-token auth: when `web.auth_token`/`auth_token_file` is set, an axum middleware (`from_fn_with_state`) requires `Authorization: Bearer <token>` on every endpoint (incl. `/healthz` and `/reload`); comparison is constant-time (XOR fold, `src/auth.rs`); 401 carries `WWW-Authenticate: Bearer`. Token changes require a restart (consistent with the reload semantics).
- Optional server TLS: `serve()` loads a rustls config from `web.tls_cert_file`/`tls_key_file` (fail-fast on error) and serves via axum-server 0.8 (rustls); a 10s HTTP/1.1 header read timeout applies in both modes. The server is fixed to HTTP/1.1 only (`http1_only()` in both branches, ALPN restricted to `http/1.1`): the axum-server version-sniffing path (first 24 bytes to detect an h2 preface) has no deadline, so h2 was removed to make the timeout cover zero-byte idle connections (slowloris). Prometheus speaks HTTP/1.1 — no operational impact. Outbound requests carry a custom redirect policy blocking https->http downgrades (10-hop limit).
- Implementation note (Windows): `serve_on` sets the listener nonblocking before serving — blocking std listeners hang all accepted-connection I/O under IOCP.
- Pagination defense caps: 64MiB per page (post-deserialization check; nv typed fetch has no streaming API), 1000 pages, 200k accumulated members — violations fail the affected log service (resource-level isolation).

## Config validation rules

From `config.rs` (`load_config`), applied in order:

1. `name` must be non-empty and unique across `bmcs`.
2. `host` must parse as a URL; scheme must be `http` or `https`.
3. `password` must not be empty.
4. At least one BMC is required.
5. `listen_addr` must parse as a `SocketAddr`.
6. `scrape_interval`, `scrape_timeout`, `request_timeout` must be non-zero (durations via `humantime`, e.g. `30s`).
7. `slow_interval`, if set, must be non-zero.
8. Defaults: `listen_addr=127.0.0.1:9417`, `scrape_interval=30s`, `scrape_timeout=15s`, `request_timeout=10s`, `slow_interval=null` (no frequency splitting), `auth=basic`, `insecure_skip_verify=false`, `ca_cert_file=null`.
9. Client-level HTTP: request timeout from `request_timeout` config (default 10 s), 5 s connect timeout, user agent `nv-redfish/v1`; `ca_cert_file` adds a root certificate, `insecure_skip_verify` disables certificate verification entirely.
10. bmc `name` must not contain control characters; bmc `host` must not carry URL credentials (userinfo) — use `username`/`password` fields.
11. `web` section: `auth_token` (>=16 chars) XOR `auth_token_file` (content trimmed, >=16 chars); `tls_cert_file` and `tls_key_file` must be set together; passwords may be overridden by `REDFISH_EXPORTER_PASSWORD_<NAME>` (empty env value rejected, name collisions rejected).

## Metric naming conventions

- Unit goes into the metric name where the value is a physical quantity: `_watts`, `_percent`, `_bytes`, `_mbps`, `_celsius`, `_seconds`.
- Sensor readings keep the unit in the `units` label (`Cel`, `W`, `RPM`, …) because one metric must serve mixed units; thresholds mirror the reading's labels so they can be matched in PromQL (`on(bmc,chassis,name)`).
- Health/state of every resource is normalized to a single `redfish_health_status` gauge with `health`/`state` labels, so alerts can be written once for all resource types.
- Static inventory is flattened into `redfish_info{key,value}` (e.g. `manufacturer`, `model`, `serial_number`, `firmware_version`) instead of a metric per field.
- Status values: `health` is normalized to `OK`/`Warning`/`Critical`/`UnsupportedValue`/`unknown`, `state` to its debug name or `unknown`.
- Counter-style names ending in `_total` (`redfish_drive_io_*_errors_total`, `redfish_scrape_errors_total`) follow Prometheus counter naming but are registered as Gauges carrying the last scraped (or cumulative) value, consistent with the snapshot-cache design; `redfish_event_log_entry` carries a Unix timestamp as its value.

## Release and static linking

0.1.0 release target: a pure static single binary built with `target x86_64-unknown-linux-musl` and a distroless non-root container image (no glibc/OpenSSL — TLS via rustls). The Dockerfile is delivered in Task 14; detailed static-linking verification is recorded by Task 15.

## Dependency security

- TLS is provided by **rustls** (`reqwest` with `rustls-tls`); the lock file contains no OpenSSL/native-tls dependency.
- The first-party crate forbids unsafe code (`#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`); the dependency tree is selected with no `unsafe`-dependent TLS stack.
- Passwords are never logged: `SecretString` redacts its `Debug` output to `[REDACTED]`.
