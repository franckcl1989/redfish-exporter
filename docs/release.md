# 0.1.0 release and production rollout

Do not create `v0.1.0` until every pre-tag item below is complete on the exact
commit that will be tagged. Automated tests prove deterministic software
properties; they do not replace validation against the production BMC models,
firmware versions, network path, and Prometheus deployment.

## Pre-tag gates

- The `CI` workflow is green on the release commit, including format, clippy,
  full tests, documentation, actionlint, promtool, performance/resource guards,
  short soak, graceful-shutdown smoke, dependency policy, and production
  container smoke.
- Run the release commit in staging for at least two hours against every BMC
  vendor/model/firmware family planned for the first rollout. Use least-privilege
  read-only Redfish accounts and the same TLS/CA and proxy path as production.
- Confirm `redfish_up == 1`, no persistent `redfish_scrape_error`, stable process
  RSS, acceptable BMC request load, expected metric cardinality, and successful
  dashboard/alert evaluation. Investigate unsupported resources; do not hide
  unexpected errors with alert exclusions.
- Size `scrape_timeout` from measured full-group tail latency rather than the
  default alone. The 0.1.0 IEIT/Inspur NF5280M6 validation profile uses 180s
  with a 120s fast interval and 900s slow interval; a 120s deadline proved too
  tight after event-log growth and network-adapter fallback requests.
- Confirm the deployment injects BMC passwords and the inbound bearer token from
  a secret store, mounts configuration read-only, and uses certificate
  verification (`ca_cert_file`) rather than `insecure_skip_verify`.
- Record the approved image digest and retain the previous known-good digest for
  rollback. Review `CHANGELOG.md` and ensure `Cargo.toml` is version `0.1.0`.

## Publish and verify

Create and push the signed/approved `v0.1.0` tag. The release workflow refuses a
tag whose version differs from `Cargo.toml`, reruns all correctness and security
gates, verifies that the Linux binary is static, smoke-tests the release
container, then publishes the archive, checksums, SPDX SBOM, attestations, and
GHCR image.

After the workflow succeeds:

```bash
sha256sum --check SHA256SUMS
gh attestation verify redfish-exporter --repo franckcl1989/redfish-exporter
gh attestation verify redfish-exporter-0.1.0-linux-amd64.tar.gz \
  --repo franckcl1989/redfish-exporter
```

Deploy by immutable image digest, not a mutable semver tag. Start with one
canary BMC group, verify at least two complete scrape intervals plus the alerts
and readiness probe, then expand gradually.

## Rollback

If the canary loses expected series, creates sustained scrape errors, increases
BMC load unexpectedly, or violates the resource envelope, stop rollout and
restore the recorded previous image digest and its matching configuration/rules.
Preserve exporter and Prometheus logs plus the failing `/metrics` snapshot for
diagnosis. Rollback does not require changes on the BMC.
