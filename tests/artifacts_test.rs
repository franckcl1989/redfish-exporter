use redfish_exporter::config::load_config;
use serde::Deserialize as _;
use serde_yaml_ng::Value;

fn yaml_documents(source: &str) -> Vec<Value> {
    serde_yaml_ng::Deserializer::from_str(source)
        .map(|document| Value::deserialize(document).expect("valid YAML document"))
        .collect()
}

#[test]
fn shipped_configuration_and_observability_assets_parse() {
    let config = load_config(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/config.example.yaml"
    )))
    .expect("config.example.yaml must pass parsing and semantic validation");
    assert_eq!(config.bmcs.len(), 1);

    let deployment = yaml_documents(include_str!("../deploy/kubernetes/deployment.yaml"));
    assert_eq!(deployment.len(), 3);
    let kinds: Vec<_> = deployment
        .iter()
        .map(|doc| doc["kind"].as_str().expect("Kubernetes kind"))
        .collect();
    assert_eq!(kinds, ["ConfigMap", "Service", "Deployment"]);

    let monitor = yaml_documents(include_str!("../deploy/kubernetes/service-monitor.yaml"));
    assert_eq!(monitor.len(), 1);
    assert_eq!(monitor[0]["kind"].as_str(), Some("ServiceMonitor"));

    let alerts = yaml_documents(include_str!("../deploy/prometheus/redfish-alerts.yml"));
    assert_eq!(alerts.len(), 1);
    assert_eq!(
        alerts[0]["groups"][0]["rules"]
            .as_sequence()
            .expect("Prometheus rule list")
            .len(),
        5
    );

    let dashboard: serde_json::Value =
        serde_json::from_str(include_str!("../deploy/grafana/redfish-dashboard.json"))
            .expect("Grafana dashboard must be valid JSON");
    assert!(
        dashboard["panels"]
            .as_array()
            .is_some_and(|panels| !panels.is_empty())
    );
}

#[test]
fn deployment_and_release_hardening_invariants_are_pinned() {
    let deployment = include_str!("../deploy/kubernetes/deployment.yaml");
    for required in [
        "automountServiceAccountToken: false",
        "runAsNonRoot: true",
        "readOnlyRootFilesystem: true",
        "allowPrivilegeEscalation: false",
        "type: RuntimeDefault",
        "path: /readyz",
        "path: /healthz",
    ] {
        assert!(
            deployment.contains(required),
            "missing deployment invariant: {required}"
        );
    }
    assert!(
        !deployment.contains("password: \""),
        "password must not be in ConfigMap"
    );
    assert!(deployment.contains("secretKeyRef:"));

    let dockerfile = include_str!("../Dockerfile");
    assert_eq!(dockerfile.matches("@sha256:").count(), 2);
    assert!(dockerfile.contains("cargo build --release --locked"));
    assert!(dockerfile.contains("USER nonroot:nonroot"));

    let release = include_str!("../.github/workflows/release.yml");
    for required in [
        "cargo test --locked --all-targets --all-features",
        "readelf --program-headers",
        "format: spdx-json",
        "syft-version: v1.51.0",
        "SYFT_SOURCE_NAME: redfish-exporter",
        "SYFT_SOURCE_VERSION: ${{ env.RELEASE_VERSION }}",
        "Attest release binary provenance",
        "Attest release binary SBOM",
        "Attest release archive",
        "Attest container image",
        "provenance: mode=max",
        "sbom: true",
        "Smoke release candidate container",
    ] {
        assert!(
            release.contains(required),
            "missing release invariant: {required}"
        );
    }
    assert!(release.contains("artifact-metadata: write"));
    assert!(release.contains("CHANGELOG.md config.example.yaml"));
}

#[test]
fn every_github_action_reference_is_an_immutable_commit() {
    for (name, workflow) in [
        ("ci", include_str!("../.github/workflows/ci.yml")),
        ("release", include_str!("../.github/workflows/release.yml")),
    ] {
        let mut references = 0;
        for line in workflow.lines() {
            let trimmed = line.trim();
            let Some(action) = trimmed.strip_prefix("- uses: ") else {
                continue;
            };
            references += 1;
            let (_, revision) = action
                .split_once('@')
                .unwrap_or_else(|| panic!("{name}: action has no revision: {action}"));
            let revision = revision
                .split_ascii_whitespace()
                .next()
                .expect("action revision");
            assert_eq!(
                revision.len(),
                40,
                "{name}: action is not SHA-pinned: {action}"
            );
            assert!(
                revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{name}: invalid action commit: {action}"
            );
        }
        assert!(
            references > 0,
            "{name}: expected at least one action reference"
        );
    }
}
