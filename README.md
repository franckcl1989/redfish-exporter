# redfish-exporter

A Prometheus exporter for Redfish BMCs, built on [nv-redfish](https://github.com/nickel-org/nv-redfish) 0.15.1.

- **Multi-BMC**: scrape any number of BMCs from a single process.
- **Periodic scrape + snapshot cache**: every BMC is scraped on a fixed interval; the latest completed snapshot (including explicit failure state) is cached and served atomically from `GET /metrics` (pre-encoded, zero live encoding on the hot path).
- **Resource-level isolation**: a failing resource (e.g. a missing chassis collection) marks that BMC's scrape as failed without dropping the rest of the collected data.
- **Fast/slow scheduling**: heavy collectors (storage, network, firmware, assembly, BIOS pending state, and opt-in event logs/full BIOS attributes) run at most once per `slow_interval` with last-good caching; light collectors run every round.
- **Session management**: `basic` or `session` auth per BMC, automatic 401 re-login, basic-auth fallback, exponential-backoff cooldown for failing BMCs, server-side session cleanup on shutdown.
- **Defensive scraping**: event-log pagination with loop/size defenses, per-request timeouts, and per fast/slow-group scrape deadlines (`fast:timeout` / `slow:timeout` on deadline expiry).

## Quick start

```bash
cargo run --release -- -c config.example.yaml
```

Container deployment is covered by the examples in [`deploy/kubernetes/`](deploy/kubernetes/). Tagged releases publish `ghcr.io/franckcl1989/redfish-exporter:<version>` and a static Linux amd64 archive with checksums, a locked-dependency SPDX SBOM, and GitHub provenance/SBOM attestations. Follow the mandatory pre-tag and canary gates in [`docs/release.md`](docs/release.md).

## Installation

From source (requires Rust 1.90+):

```bash
cargo install --path .
redfish-exporter -c config.example.yaml
```

Docker (static musl build, distroless non-root image):

```bash
docker build -t redfish-exporter:0.1.0 .
docker run --rm -p 9417:9417 -v "$(pwd)/config.yaml:/config.yaml:ro" redfish-exporter:0.1.0 -c /config.yaml
```

Note: `listen_addr` defaults to `127.0.0.1:9417` inside the container too — bind `0.0.0.0` explicitly (see `config.example.yaml`) so the published port is reachable from outside.

Endpoints:

| Path        | Description                                              |
|-------------|----------------------------------------------------------|
| `/metrics`  | Prometheus text exposition of the current metric snapshot |
| `/healthz`  | Liveness probe, returns `ok`                              |
| `/readyz`   | Readiness probe; `200` after every configured BMC has published its first snapshot, otherwise `503` |
| `/info`     | Build information (version, Rust version, target OS/arch) as JSON |

Note: `listen_addr` defaults to `127.0.0.1:9417` — remote scraping requires an explicit `0.0.0.0` bind (see Security).

CLI options: `-c/--config <path>` (default `config.yaml`), `-p/--port <port>` (overrides `listen_addr` port), `--log-level <filter>` (default `info`; `RUST_LOG` takes precedence), `-V/--version`.

## Configuration

See [`config.example.yaml`](config.example.yaml). All durations use `humantime` syntax (`30s`, `15m`, `1h`).

| Key                      | Default       | Description                                                      |
|--------------------------|---------------|------------------------------------------------------------------|
| `listen_addr`            | `127.0.0.1:9417`| HTTP listen address; defaults to 127.0.0.1 for safe-by-default, set 0.0.0.0 explicitly for remote Prometheus |
| `scrape_interval`        | `30s`         | Interval between scrape rounds                                   |
| `scrape_timeout`         | `15s`         | Independent deadline for each fast and slow collector group      |
| `slow_interval`          | `null`        | Interval for slow-group collectors (storage, network, firmware, assembly, event logs, BIOS); `null` = collect every round |
| `request_timeout`        | `10s`         | Per-request HTTP timeout, applied to every BMC request       |
| `stability`                | defaults 3 / 60s / 300s | Failure cooldown: consecutive failed rounds before full cooldown, first backoff, backoff cap. Session-auth BMCs fall back to basic collection while session re-login backs off |
| `collectors.event_logs`    | `false`       | Opt in to per-entry event-log metrics; disabled by default because log `id`/`message` labels are high-cardinality |
| `collectors.event_log_limit` | `500`       | Maximum event-log entries exported per BMC snapshot; valid range 1–5,000 |
| `collectors.bios_attributes` | `false`     | Opt in to full numeric/string BIOS attributes; `redfish_bios_pending_changes` remains enabled |
| `collectors.bios_attribute_limit` | `10000` | Maximum BIOS attributes exported per BMC snapshot; valid range 1–10,000 |
| `web`                    | `null`        | Inbound hardening: auth_token (>=16 chars), auth_token_file (mutually exclusive), tls_cert_file + tls_key_file (must be set together) |
| `bmcs`                   | required      | Non-empty list of BMC entries                                    |
| `bmcs[].name`            | required      | Unique name, used as the `bmc` metric label                      |
| `bmcs[].host`            | required      | BMC base URL, scheme `http` or `https`                           |
| `bmcs[].username`        | required      | Redfish account name                                             |
| `bmcs[].password`        | required from file or environment | Account password (never logged; `Debug` output is `[REDACTED]`); may be omitted from YAML when env var `REDFISH_EXPORTER_PASSWORD_<NAME>` is set (name uppercased, non-alphanumerics replaced by `_`) |
| `bmcs[].auth`            | `basic`       | `basic` or `session` (session authentication establishes an `X-Auth-Token` automatically) |
| `bmcs[].insecure_skip_verify` | `false`  | **Dangerous**: disables TLS certificate verification             |
| `bmcs[].ca_cert_file`    | `null`        | Path to a PEM CA bundle for self-signed BMC certificates         |

The duration defaults are starting points, not a latency guarantee. Measure a
full slow-group round on every production model and leave margin above its
observed tail latency. In the 0.1.0 release validation, a Dell PowerEdge R750
completed comfortably within the defaults, while an IEIT/Inspur NF5280M6 with
BMC firmware 7.18.00 required `scrape_timeout: "180s"` in the opt-in full
inventory profile containing more than 2,000 event-log entries; production was
tested with `scrape_interval: "120s"` and `slow_interval: "900s"` to keep BMC
request load bounded.

### Authentication

- **basic**: each request is authenticated with HTTP Basic credentials.
- **session**: during the first concurrent scrape, the exporter creates a Redfish session via the SessionService, receives an `X-Auth-Token`, and switches to token-based authentication. Session failures fall back to Basic auth and retry with backoff.

## Metrics

The full reference (every metric, its labels, help text and source Redfish resource) is in [`docs/metrics.md`](docs/metrics.md). Overview:

| Metric                                        | Meaning                              |
|-----------------------------------------------|--------------------------------------|
| `redfish_up`                                  | Last scrape of the BMC succeeded     |
| `redfish_scrape_duration_seconds`             | Duration of the last scrape          |
| `redfish_scrape_error`                        | A resource failed during the scrape  |
| `redfish_scrape_errors_total`, `redfish_build_info` | Exporter self-observation (cumulative scrape errors, build info) |
| `redfish_health_status`                       | Health/state of any Redfish resource |
| `redfish_info`                                | Resource-scoped static key-value inventory info |
| `redfish_sensor_reading` / `redfish_sensor_threshold_*` | Sensor readings and thresholds |
| `redfish_power_consumption_watts`, `redfish_power_consumption_min/max/avg_watts`, `redfish_power_consumption_interval_minutes` | Chassis power consumption and statistics |
| `redfish_power_supply_*`                      | PSU details (efficiency, input watts, capacity, input voltage) |
| `redfish_power_state`                         | System power state (1 = On, 0 = another known state) |
| `redfish_processor_temperature_celsius`, `redfish_processor_power_watts`, `redfish_processor_bandwidth_percent` | Processor metrics |
| `redfish_memory_capacity_bytes`, `redfish_memory_bandwidth_percent`, `redfish_memory_correctable_errors`, `redfish_memory_uncorrectable_errors` | Memory metrics (capacity, bandwidth, ECC alarm trips) |
| `redfish_drive_*`, `redfish_volume_capacity_bytes` | Storage metrics              |
| `redfish_ethernet_interface_*`, `redfish_pcie_device_*` | Network metrics             |
| `redfish_event_log_entry`                     | Opt-in event log entries (timestamp = creation time; capped) |
| `redfish_bios_attribute`, `redfish_bios_attribute_info` | Opt-in, capped BIOS attributes |
| `redfish_bios_pending_changes`                | BIOS settings pending reboot (enabled by default) |

Hardening additions (probe-gated on vendor OEM fields): `redfish_processor_frequency_mhz`, `redfish_processor_max_frequency_mhz`, `redfish_processor_voltage_volts`, `redfish_storage_controller_info`, `redfish_storage_controller_status`, `redfish_drive_info`, `redfish_drive_oem_status`, `redfish_indicator_led`.

The table above is an overview only — [`docs/metrics.md`](docs/metrics.md) is the authoritative full catalog (53 metric names in 0.1.0, every metric with labels, help text and source Redfish resource). Event-log entries and full BIOS attributes are deliberately disabled by default: on the validated Inspur BMC they accounted for roughly 98% of all series. Enable them only for a bounded inventory/compliance use case and size Prometheus retention accordingly.

## Alerting

Example Prometheus alert rules are provided in [`deploy/prometheus/redfish-alerts.yml`](deploy/prometheus/redfish-alerts.yml) (availability, current and transient scrape failures, component health, sensor thresholds, drive failure/life, memory ECC, BIOS pending changes, and link state).

A 14-panel Grafana operations dashboard with a multi-BMC filter is provided in [`deploy/grafana/redfish-dashboard.json`](deploy/grafana/redfish-dashboard.json) (import via Grafana UI or provisioning), and a Prometheus `ServiceMonitor` for the Prometheus operator ships in [`deploy/kubernetes/service-monitor.yaml`](deploy/kubernetes/service-monitor.yaml).

## Security

- **Inbound hardening (optional, per docs/security.md)**: bearer-token auth on `/metrics` and `/info` (constant-time compare, `web.auth_token` / `web.auth_token_file`, >= 16 chars); `/healthz` and `/readyz` intentionally expose status only so orchestrator probes do not need credentials. Server-side TLS uses rustls (`web.tls_cert_file` + `web.tls_key_file`). `listen_addr` defaults to `127.0.0.1:9417` — remote scraping requires an explicit `0.0.0.0` bind plus auth/TLS per the baseline matrix.
- **Secret redaction**: passwords are stored in a `SecretString` type whose `Debug` representation is `[REDACTED]` and whose buffer is zeroized on drop; they are never logged. Passwords can be injected via environment variables instead of the config file.
- **TLS**: rustls — no OpenSSL dependency. Custom CA bundles via `ca_cert_file`. Redirects may not change the BMC origin (scheme, host, or port); `insecure_skip_verify` is per-BMC opt-in for self-signed BMC certificates only — prefer `ca_cert_file`.
- **Supply chain**: `#![forbid(unsafe_code)]`, cargo-deny policy (`deny.toml`), cargo-audit allowlist with rationale (`.cargo/audit.toml`), SHA-pinned Actions and container bases, a locked-dependency SPDX SBOM, provenance/SBOM attestations, and a non-root distroless container with read-only root filesystem.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

`#![forbid(unsafe_code)]` is enabled in the crate root; the dependency tree uses only safe Rust TLS (rustls).

## License

Licensed under the Apache License, Version 2.0 ([LICENSE](LICENSE)).

## Acknowledgements

Built on [nv-redfish](https://github.com/nickel-org/nv-redfish) 0.15.1 — a Redfish client library for Rust. This project would not exist without it.
