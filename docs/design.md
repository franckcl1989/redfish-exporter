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
        |  BMC (collect_all)        |                 |  GET /healthz            |
        |  round deadline =         |                 +-------------+-------------+
        |  scrape_timeout           |                               |
        |  build_registry per BMC   |                               | snapshot.registry()
        +-------------+-------------+                               |
                      | snapshot.update(Arc<Registry>)             |
                      v                                             |
        +---------------------------+                               |
        |        registry.rs        |  RwLock<Option<Arc<Registry>>>|
        |  Snapshot (atomic swap)   |<------------------------------+
        +---------------------------+
```

Per-BMC scrape pipeline (each collector in `src/collector/`):

```
ServiceRoot
  ├─ sensors.rs   chassis sensor links + thresholds
  ├─ power.rs     legacy Thermal/Power, power supplies, EnvironmentMetrics, Controls
  ├─ processors.rs ProcessorMetrics (temp/power/bandwidth)
  ├─ memory.rs    Memory capacity + MemoryMetrics
  ├─ storage.rs   Drive metrics + Volume capacity
  ├─ network.rs   EthernetInterface + PCIe devices + NetworkAdapter ports
  └─ systems.rs   System/Chassis/Manager health+info, Assembly, Firmware inventory
        └─ all push health (redfish_health_status) and info (redfish_info)
```

## Component responsibilities

| Module | Responsibility |
|--------|----------------|
| `config.rs` | YAML parsing, duration deserialization (`humantime`), validation, `SecretString` redaction |
| `bmc.rs` | Per-BMC reqwest client construction (timeout, TLS, CA bundle, `insecure_skip_verify`), `HttpBmc` handle creation, generic session establishment |
| `collector/*.rs` | Fetch Redfish resources through nv-redfish and convert them into `Metric` values; one module per resource family |
| `collector/mod.rs` | `collect_all`: orchestrates every collector against one BMC, aggregates metrics and `failed_resources`; shared `push_health` / `push_info` helpers |
| `scraper.rs` | Periodic loop; spawns one task per BMC (`JoinSet`); enforces the round deadline; builds per-BMC registries and publishes them to the snapshot |
| `registry.rs` | `Snapshot`: `RwLock<Option<Arc<Registry>>>` with atomic replacement; adds `redfish_scrape_error` series and `redfish_up=0` for failed resources |
| `http.rs` | axum router serving `/metrics` (encoded snapshot) and `/healthz`; graceful shutdown |
| `main.rs` | CLI parsing, wiring, signal handling, exit codes |

## Data flow and error model

- **BMC-level isolation**: each round, every BMC is scraped in its own `JoinSet` task producing its own `Result`. A failing BMC (auth error, unreachable host, …) yields `redfish_up{bmc}=0` and does not affect other BMCs. A BMC that never produced a snapshot is simply absent from `/metrics` (with `x-redfish-exporter: no-data-yet` header).
- **Resource-level isolation**: a failing collector does not abort the whole BMC. It records the failing resource in `failed_resources`, which produces `redfish_scrape_error{bmc,resource}=1`, forces `redfish_up{bmc}=0` (applied in `registry.rs`), while the metrics of all other resources are still published.
- **Round deadline**: the whole round is wrapped in `tokio::time::timeout(scrape_timeout)`; on expiry the remaining tasks are aborted (`JoinSet::shutdown`) and the round is logged as timed out.
- **Snapshot consistency**: the registry per BMC is built fresh each round and swapped in atomically, so `/metrics` always returns one consistent snapshot and never a partially-written one.

## Known design trade-off (Ruling 7c)

Each collector independently enumerates the `Chassis`/`Systems` collections, so the number of BMC requests per round is roughly **N × (collection walk)** where N = number of collectors (each collector re-fetches chassis/systems members and common collections). At the default 30 s interval this load is acceptable for typical single-BMC-in-band and out-of-band setups, and it keeps collectors strictly isolated (a new collector cannot break the resource walks of existing ones). Consolidating collection walks into a shared pre-fetch phase is a possible future optimization and should be revisited if request volume becomes a problem on large fleets.

## Lifecycle and shutdown

```
main
  └─ start: config -> Scraper::new (clients + optional session establishment)
  └─ watch channel (stop: bool)
  └─ serve + scraper.run concurrently
  └─ signal (SIGINT/SIGTERM/Ctrl+C) -> stop_tx.send(true)
       ├─ scraper loop: select! on stop.changed() -> break (before next tick)
       └─ http serve: with_graceful_shutdown(stop.changed()) -> drains in-flight requests
  └─ join both -> "shutdown complete" -> exit code 0
```

If the scraper task panics/aborts unexpectedly, main sends the stop signal and shuts the server down rather than serving stale data.

## Authentication flow

- `basic`: `BmcCredentials::username_password` is baked into the `HttpBmc`; every request carries HTTP Basic credentials.
- `session`: at startup, `bmc.rs::establish_session` (generic over `B: nv_redfish::Bmc`, so the flow is testable with the mock BMC) walks `ServiceRoot -> SessionService -> Sessions`, POSTs a `SessionCreate`, extracts `X-Auth-Token`, and the caller applies it via `HttpBmc::set_credentials(BmcCredentials::token(...))`. All subsequent requests use the token. Session expiry/refresh is out of scope for 0.1.0 (restart re-establishes).

## Config validation rules

From `config.rs` (`load_config`), applied in order:

1. `name` must be non-empty and unique across `bmcs`.
2. `host` must parse as a URL; scheme must be `http` or `https`.
3. `password` must not be empty.
4. At least one BMC is required.
5. `listen_addr` must parse as a `SocketAddr`.
6. `scrape_interval`, `scrape_timeout`, `request_timeout` must be non-zero (durations via `humantime`, e.g. `30s`).
7. Defaults: `listen_addr=0.0.0.0:9417`, `scrape_interval=30s`, `scrape_timeout=15s`, `request_timeout=10s`, `auth=basic`, `insecure_skip_verify=false`, `ca_cert_file=null`.

Client-level HTTP: 120 s request timeout, 5 s connect timeout, user agent `nv-redfish/v1`; `ca_cert_file` adds a root certificate, `insecure_skip_verify` disables certificate verification entirely.

## Metric naming conventions

- Unit goes into the metric name where the value is a physical quantity: `_watts`, `_percent`, `_bytes`, `_mbps`, `_celsius`, `_seconds`.
- Sensor readings keep the unit in the `units` label (`Cel`, `W`, `RPM`, …) because one metric must serve mixed units; thresholds mirror the reading's labels so they can be matched in PromQL (`on(bmc,chassis,name)`).
- Health/state of every resource is normalized to a single `redfish_health_status` gauge with `health`/`state` labels, so alerts can be written once for all resource types.
- Static inventory is flattened into `redfish_info{key,value}` (e.g. `manufacturer`, `model`, `serial_number`, `firmware_version`) instead of a metric per field.
- Status values: `health` is normalized to `OK`/`Warning`/`Critical`/`UnsupportedValue`/`unknown`, `state` to its debug name or `unknown`.

## Release and static linking

0.1.0 release target: a pure static single binary built with `target x86_64-unknown-linux-musl` and a distroless non-root container image (no glibc/OpenSSL — TLS via rustls). The Dockerfile is delivered in Task 14; detailed static-linking verification is recorded by Task 15.

## Dependency security

- TLS is provided by **rustls** (`reqwest` with `rustls-tls`); the lock file contains no OpenSSL/native-tls dependency.
- The first-party crate forbids unsafe code (`#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`); the dependency tree is selected with no `unsafe`-dependent TLS stack.
- Passwords are never logged: `SecretString` redacts its `Debug` output to `[REDACTED]`.
