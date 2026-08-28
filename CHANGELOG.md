# Changelog

All notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-28

### Added

- Concurrent multi-BMC Redfish collection with independent fast and slow
  schedules, cached atomic Prometheus snapshots, and bounded pagination.
- System, chassis, processor, memory, storage, network, sensor, power,
  firmware, assembly, event-log, and BIOS metrics with vendor-extension probes.
- Basic and Redfish session authentication, session re-login/backoff, and
  best-effort server-side session cleanup during graceful shutdown.
- Optional inbound bearer authentication and TLS, custom BMC CA bundles,
  health/readiness probes, Kubernetes manifests, alerts, and a Grafana dashboard.
- Release checksums, a locked-dependency SPDX SBOM, provenance/SBOM
  attestations, and a static non-root distroless container image.

### Security

- Safe localhost default, strict configuration validation, secret redaction and
  zeroization, same-origin redirect enforcement, request/scrape deadlines, and
  response/pagination limits.
- SHA-pinned GitHub Actions and digest-pinned build/runtime container bases.
- Cargo audit and cargo-deny policy gates with documented build-only advisory
  exceptions.

### Fixed

- Prometheus family merging across BMCs, metric type correctness, duplicate
  series handling, and globally unique resource identities.
- Cancellation, panic, and graceful-shutdown behavior; partial collector and
  invalid-pagination failures now remain observable instead of appearing as
  successful empty scrapes.
- ServiceRoot 404 responses now count as failed rounds and advance adaptive
  cooldown instead of resetting failure state while publishing `redfish_up=0`.
- Optional Redfish links remain non-errors when absent, while request/parse
  failures on all declared processor, memory, storage, network, power, BIOS,
  Assembly, or event-log paths now fail the affected collector instead of
  silently publishing an empty success; partial failures are logged.
- IEIT/Inspur firmware vendor code `17034` (an advertised synthetic RAID member
  when no controller is installed) is treated as an absent optional resource,
  while unrelated HTTP 500 responses still fail storage collection.
- Network-adapter collections whose annotated members omit inline `Id` now fall
  back to resolving each complete member by `@odata.id`, retaining adapter
  health and inventory instead of emitting a permanent parse warning.

[0.1.0]: https://github.com/franckcl1989/redfish-exporter/releases/tag/v0.1.0
