# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-25

### Added

- **Initial implementation** — multi-BMC Prometheus exporter for Redfish BMCs built on [nv-redfish](https://github.com/nickel-org/nv-redfish) 0.15.1:
  - Periodic scraping with a per-BMC atomic snapshot cache; `/metrics` serves the last good snapshot with zero live encoding.
  - Resource-level and BMC-level failure isolation (`redfish_up`, `redfish_scrape_error`, cumulative `redfish_scrape_errors_total`).
  - 18 original metric families: health/status, inventory info, sensors and thresholds, chassis power (consumption stats, PSU details, power state), processors, memory (capacity, bandwidth, ECC alarm trips), drives/volumes, network interfaces and PCIe devices, event logs, BIOS attributes.
  - Fast/slow scheduling (`slow_interval` with last-good caching) and a per-BMC scrape deadline.
  - Session management: session authentication, automatic 401 re-login, basic-auth fallback, server-side session cleanup on shutdown.
  - Event-log pagination walker (both `@odata.nextLink` and `Members@odata.nextLink` variants) with loop, page-size and member-count defenses.
  - Operations endpoints `/healthz`, `/info`, `/discover`, `/reload`.
- **Security hardening** (docs/audit/2026-08-21-security-hardening.md):
  - Layered config: optional bearer-token auth (constant-time compare, `auth_token` / `auth_token_file`) and server-side TLS via rustls.
  - Threat model and security baseline matrix in `docs/security.md`.
  - Secret redaction (`SecretString` + zeroize), env-var credential override (`REDFISH_EXPORTER_PASSWORD_<NAME>`), URL-userinfo rejection, https→http redirect downgrade blocking.
  - Supply chain: `#![forbid(unsafe_code)]`, cargo-deny policy, cargo-audit allowlist, non-root distroless container.
  - Default listen address tightened to `127.0.0.1:9417`.
- **Stability hardening** (docs/audit/2026-08-24-stability.md):
  - Adaptive cooldown state machine per BMC (consecutive-failure threshold, exponential backoff 60s→300s cap).
  - Session-degraded mode: basic-auth collection continues while session re-establishment backs off.
  - Fault-injection test matrix (real 401, garbled responses, mock expectation queues) and graceful SIGTERM shutdown with a CI smoke job.
- **Performance hardening** (docs/audit/2026-08-24-performance.md):
  - Pre-encoded snapshot cache (`/metrics` hot path serves stored bytes; measured p50 2.4–10.7ms at ~10k series, byte-identical to live encoding).
  - `register_into` label-lookup optimization and preallocation.
  - Mock pipeline benchmark (`tests/perf_test.rs`) and real-hardware before/after comparison.
- **Resource hardening** (docs/audit/2026-08-24-resource.md):
  - Four-axis real-hardware measurement (memory / CPU / connections / request volume) with artifacts, documented memory model and bounded-growth analysis.
  - Benchmark comparison against idrac_exporter, fishymetrics and sapcc.
- **Function hardening** (docs/audit/2026-08-24-function.md):
  - 8 new metric families, probe-gated on real hardware (Dell iDRAC / Inspur): processor current/max frequency and voltage, storage controller info and status, drive vendor identifier and OEM status, system/chassis indicator LED.
  - Metrics reference updated to the full 26-family catalog (`docs/metrics.md`).
- **Quality hardening** (docs/audit/2026-08-24-quality.md):
  - Six-dimension acceptance records under `docs/audit/`, release assets: Apache-2.0 `LICENSE`, this changelog, README completeness pass, release workflow review.

### Known limitations / backlog

- N× collection walk: each collector independently enumerates collections (documented trade-off, `docs/design.md`); a shared pre-fetch is backlog.
- `/reload` validates and hot-replaces the config, but BMC add/remove or credential changes require a restart.
- Probe NOT MET items (firmware/data limits, tracked in the function record): RAID battery, drive lifetime/PPID, CPU-level power, Inspur storage subtree (BMC returns HTTP 500).
- quick-xml build-time advisories (RUSTSEC-2026-0194/0195) are whitelisted in `.cargo/audit.toml`; they do not enter the runtime binary and are tracked against the nv-redfish upstream.
- gzip/streaming output and streaming response-size caps are backlog.
- Soak ≥2h and real-machine long-run observation remain pre-production follow-ups.
- Grafana dashboard, Kubernetes manifests and alert rules in `deploy/` are examples and should be tuned per environment.
