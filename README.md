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

CLI options: `-c/--config <path>` (default `config.yaml`), `-p/--port <port>` (overrides `listen_addr` port), `--log-level <level>` (default `info`; `RUST_LOG` takes precedence).

## Configuration

See [`config.example.yaml`](config.example.yaml). All durations use `humantime` syntax (`30s`, `15m`, `1h`).

| Key                      | Default       | Description                                                      |
|--------------------------|---------------|------------------------------------------------------------------|
| `listen_addr`            | `0.0.0.0:9417`| HTTP listen address for `/metrics` and `/healthz`               |
| `scrape_interval`        | `30s`         | Interval between scrape rounds                                   |
| `scrape_timeout`         | `15s`         | Deadline for one scrape round; tasks are aborted when exceeded   |
| `request_timeout`        | `10s`         | Per-HTTP-request timeout against the BMC (overridden by the client-level timeout) |
| `bmcs`                   | required      | Non-empty list of BMC entries                                    |
| `bmcs[].name`            | required      | Unique name, used as the `bmc` metric label                      |
| `bmcs[].host`            | required      | BMC base URL, scheme `http` or `https`                           |
| `bmcs[].username`        | required      | Redfish account name                                             |
| `bmcs[].password`        | required      | Account password (never logged; `Debug` output is `[REDACTED]`)  |
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
| `redfish_health_status`                       | Health/state of any Redfish resource |
| `redfish_info`                                | Static key-value inventory info      |
| `redfish_sensor_reading` / `redfish_sensor_threshold_*` | Sensor readings and thresholds |
| `redfish_power_consumption_watts`             | Chassis power consumption            |
| `redfish_power_state`                         | System power state (1 = On)          |
| `redfish_processor_temperature_celsius`, `redfish_processor_power_watts`, `redfish_processor_bandwidth_percent` | Processor metrics |
| `redfish_memory_capacity_bytes`, `redfish_memory_bandwidth_percent` | Memory metrics      |
| `redfish_drive_*`, `redfish_volume_capacity_bytes` | Storage metrics              |
| `redfish_ethernet_interface_*`, `redfish_pcie_device_*` | Network metrics             |

## Alerting

Example Prometheus alert rules are provided in [`deploy/prometheus/redfish-alerts.yml`](deploy/prometheus/redfish-alerts.yml) (BMC unreachable, scrape errors, sensor thresholds, drive predictive failure, link down).

## Security

- **Secret redaction**: passwords are stored in a `SecretString` type whose `Debug` representation is `[REDACTED]`; they are never logged.
- **TLS**: TLS is provided by `rustls` — no OpenSSL dependency. Custom CA bundles via `ca_cert_file`.
- **`insecure_skip_verify`**: only for self-signed BMC certificates. It disables certificate verification entirely (including hostname checks); prefer `ca_cert_file` whenever possible.
- **Non-root containers**: the Kubernetes examples run as `runAsUser: 65532` with a read-only root filesystem.

## Development

```bash
cargo fmt --check
cargo clippy -- -D warnings     # note the `--` before -D warnings (PowerShell-safe)
cargo test
```

`#![forbid(unsafe_code)]` is enabled in the crate root; the dependency tree uses only safe Rust TLS (rustls).

## License

Apache-2.0
