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
- For the 0.1.0 validation profile, run the release commit in staging for at
  least 15 minutes against every BMC vendor/model/firmware family planned for
  the first rollout. The run must include the initial full collection, at least
  one subsequent slow-group refresh, and at least five fast-group refreshes; set
  a shorter validation-only slow interval if needed to fit that coverage into
  the gate. Use least-privilege read-only Redfish accounts and the same TLS/CA
  and proxy path as production.
- Confirm `redfish_up == 1`, no persistent `redfish_scrape_error`, stable process
  RSS, acceptable BMC request load, expected metric cardinality, and successful
  dashboard/alert evaluation. Investigate unsupported resources; do not hide
  unexpected errors with alert exclusions.
- Size `scrape_timeout` from measured full-group tail latency rather than the
  default alone. The 0.1.0 IEIT/Inspur NF5280M6 production profile uses 180s
  with a 120s fast interval and 900s slow interval; the 15-minute acceptance
  run uses a validation-only 420s slow interval to complete two slow groups. A
  120s deadline proved too tight after event-log growth and network-adapter
  fallback requests.
- Confirm the deployment injects BMC passwords and the inbound bearer token from
  a secret store, mounts configuration read-only, and uses certificate
  verification (`ca_cert_file`) rather than `insecure_skip_verify`.
- Record the approved image digest and retain the previous known-good digest for
  rollback. Review `CHANGELOG.md` and ensure `Cargo.toml` is version `0.1.0`.

The completed 0.1.0 candidate evidence is recorded in
[`docs/audit/2026-08-28-release-candidate.md`](audit/2026-08-28-release-candidate.md).

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
and readiness probe, then expand gradually. The 15-minute release gate is not a
substitute for post-deployment memory, cardinality, and BMC-load monitoring.

## Partial-release recovery

The normal release workflow creates the public GitHub Release only after the
container image has been pushed and its registry attestation succeeds. If an
older run has already uploaded valid binary assets but fails only while
publishing the container, keep the release as a draft and do not move or
recreate the immutable tag. Run the narrowly scoped recovery workflow instead:

```bash
gh workflow run repair-release-container.yml -f tag=v0.1.0
```

The workflow checks out that exact tag, verifies that the tag points at `HEAD`
and matches `Cargo.toml`, smoke-tests the rebuilt image, publishes semver tags
with BuildKit provenance and SBOM attestations, adds the GitHub registry
attestation, and verifies that the pushed digest resolves. Publish the draft
release only after this workflow and the downloaded asset/attestation checks
all succeed.

## Rollback

If the canary loses expected series, creates sustained scrape errors, increases
BMC load unexpectedly, or violates the resource envelope, stop rollout and
restore the recorded previous image digest and its matching configuration/rules.
Preserve exporter and Prometheus logs plus the failing `/metrics` snapshot for
diagnosis. Rollback does not require changes on the BMC.
