# redfish-exporter

A Prometheus exporter for Redfish BMCs, built on [nv-redfish](https://github.com/nickel-org/nv-redfish) 0.15.1.

- **Multi-BMC**: scrape any number of BMCs from a single process.
- **Periodic scrape + snapshot cache**: every BMC is scraped on a fixed interval; the last successful snapshot per BMC is cached and served atomically from `GET /metrics`.
- **Resource-level isolation**: a failing resource (e.g. a missing chassis collection) marks that BMC's scrape as failed without dropping the rest of the collected data.

## Quick start

```bash
cargo run --release -- -c config.example.yaml
```

Container deployment is covered by the examples in [`deploy/kubernetes/`](deploy/kubernetes/) (image placeholder `ghcr.io/your-org/redfish-exporter:0.1.0`).

Endpoints:

| Path        | Description                                              |
|-------------|----------------------------------------------------------|
| `/metrics`  | Prometheus text exposition of the current metric snapshot |
| `/healthz`  | Liveness/readiness probe, returns `ok`                    |
| `/info`     | Build information (version, Rust version, target OS/arch) as JSON |
| `/discover` | Prometheus HTTP SD target list of all BMC hosts as JSON   |
| `/reload`   | Reload and validate the config file from disk (`POST`; `400` on invalid config, old config kept; BMC add/remove or credential changes still need a restart) |

Note: `listen_addr` defaults to `127.0.0.1:9417` — remote scraping requires an explicit `0.0.0.0` bind (see Security).

CLI options: `-c/--config <path>` (default `config.yaml`), `-p/--port <port>` (overrides `listen_addr` port), `--log-level <level>` (default `info`; `RUST_LOG` takes precedence).

## Configuration

See [`config.example.yaml`](config.example.yaml). All durations use `humantime` syntax (`30s`, `15m`, `1h`).

| Key                      | Default       | Description                                                      |
|--------------------------|---------------|------------------------------------------------------------------|
| `listen_addr`            | `127.0.0.1:9417`| HTTP listen address; defaults to 127.0.0.1 for safe-by-default, set 0.0.0.0 explicitly for remote Prometheus |
| `scrape_interval`        | `30s`         | Interval between scrape rounds                                   |
| `scrape_timeout`         | `15s`         | Deadline for one scrape round; tasks are aborted when exceeded   |
| `slow_interval`          | `null`        | Interval for slow-group collectors (storage, network, firmware, assembly, event logs, BIOS); `null` = collect every round |
| `request_timeout`        | `10s`         | Per-request HTTP timeout, applied to every BMC request       |
| `web`                    | `null`        | Inbound hardening: auth_token (>=16 chars), auth_token_file (mutually exclusive), tls_cert_file + tls_key_file (must be set together) |
| `bmcs`                   | required      | Non-empty list of BMC entries                                    |
| `bmcs[].name`            | required      | Unique name, used as the `bmc` metric label                      |
| `bmcs[].host`            | required      | BMC base URL, scheme `http` or `https`                           |
| `bmcs[].username`        | required      | Redfish account name                                             |
| `bmcs[].password`        | required      | Account password (never logged; `Debug` output is `[REDACTED]`); can be overridden by env var REDFISH_EXPORTER_PASSWORD_<NAME> (name uppercased, non-alphanumerics replaced by _) |
| `bmcs[].auth`            | `basic`       | `basic` or `session` (session authentication establishes an `X-Auth-Token` automatically) |
| `bmcs[].insecure_skip_verify` | `false`  | **Dangerous**: disables TLS certificate verification             |
| `bmcs[].ca_cert_file`    | `null`        | Path to a PEM CA bundle for self-signed BMC certificates         |

### Authentication

- **basic**: each request is authenticated with HTTP Basic credentials.
- **session**: the exporter creates a Redfish session at startup (via the SessionService), receives an `X-Auth-Token`, and switches to token-based authentication for all subsequent requests.

## Metrics

The full reference (every metric, its labels, help text and source Redfish resource) is in [`docs/metrics.md`](docs/metrics.md). Overview:

| Metric                                        | Meaning                              |
|-----------------------------------------------|--------------------------------------|
| `redfish_up`                                  | Last scrape of the BMC succeeded     |
| `redfish_scrape_duration_seconds`             | Duration of the last scrape          |
| `redfish_scrape_error`                        | A resource failed during the scrape  |
| `redfish_scrape_errors_total`, `redfish_build_info` | Exporter self-observation (cumulative scrape errors, build info) |
| `redfish_health_status`                       | Health/state of any Redfish resource |
| `redfish_info`                                | Static key-value inventory info      |
| `redfish_sensor_reading` / `redfish_sensor_threshold_*` | Sensor readings and thresholds |
| `redfish_power_consumption_watts`, `redfish_power_consumption_min/max/avg_watts`, `redfish_power_consumption_interval_minutes` | Chassis power consumption and statistics |
| `redfish_power_supply_*`                      | PSU details (efficiency, input watts, capacity, input voltage) |
| `redfish_power_state`                         | System power state (1 = On)          |
| `redfish_processor_temperature_celsius`, `redfish_processor_power_watts`, `redfish_processor_bandwidth_percent` | Processor metrics |
| `redfish_memory_capacity_bytes`, `redfish_memory_bandwidth_percent`, `redfish_memory_correctable_errors`, `redfish_memory_uncorrectable_errors` | Memory metrics (capacity, bandwidth, ECC alarm trips) |
| `redfish_drive_*`, `redfish_volume_capacity_bytes` | Storage metrics              |
| `redfish_ethernet_interface_*`, `redfish_pcie_device_*` | Network metrics             |
| `redfish_event_log_entry`                     | Event log entries (timestamp = creation time) |
| `redfish_bios_attribute`, `redfish_bios_attribute_info`, `redfish_bios_pending_changes` | BIOS attributes and pending settings |

## Alerting

Example Prometheus alert rules are provided in [`deploy/prometheus/redfish-alerts.yml`](deploy/prometheus/redfish-alerts.yml) (BMC unreachable, scrape errors, sensor thresholds, drive predictive failure, link down).

## Security

- **Inbound hardening (optional, per docs/security.md)**: bearer-token auth on all endpoints (constant-time compare, `web.auth_token` / `web.auth_token_file`, >= 16 chars) and server-side TLS via rustls (`web.tls_cert_file` + `web.tls_key_file`). `listen_addr` defaults to `127.0.0.1:9417` — remote scraping requires an explicit `0.0.0.0` bind plus auth/TLS per the baseline matrix.
- **Secret redaction**: passwords are stored in a `SecretString` type whose `Debug` representation is `[REDACTED]` and whose buffer is zeroized on drop; they are never logged. Passwords can be injected via environment variables instead of the config file.
- **TLS**: rustls — no OpenSSL dependency. Custom CA bundles via `ca_cert_file`. https->http redirect downgrades are blocked; `insecure_skip_verify` is per-BMC opt-in for self-signed BMC certificates only — prefer `ca_cert_file`.
- **Supply chain**: `#![forbid(unsafe_code)]`, cargo-deny policy (`deny.toml`), cargo-audit allowlist with rationale (`.cargo/audit.toml`), non-root distroless container with read-only root filesystem.

## Development

```bash
cargo fmt --check
cargo clippy -- -D warnings     # note the `--` before -D warnings (PowerShell-safe)
cargo test
```

`#![forbid(unsafe_code)]` is enabled in the crate root; the dependency tree uses only safe Rust TLS (rustls).

## License

Apache-2.0
