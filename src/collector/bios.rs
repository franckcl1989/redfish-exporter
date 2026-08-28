use crate::config::MAX_BIOS_ATTRIBUTE_LIMIT;
use crate::metrics::Metric;
use nv_redfish::Bmc;
use nv_redfish::Resource as _;
use nv_redfish::core::EdmPrimitiveType;
use nv_redfish::core::RedfishSettings as _;
use std::sync::Arc;

const BIOS_PENDING: (&str, &str) = (
    "redfish_bios_pending_changes",
    "BIOS settings pending reboot",
);
const BIOS_ATTR: (&str, &str) = ("redfish_bios_attribute", "BIOS attribute value");
const BIOS_ATTR_INFO: (&str, &str) = ("redfish_bios_attribute_info", "BIOS string attribute");

pub async fn collect_bios<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
) -> Result<Vec<Metric>, String> {
    collect_bios_configured(_bmc, root, bmc_name, true, MAX_BIOS_ATTRIBUTE_LIMIT).await
}

pub async fn collect_bios_configured<B: Bmc>(
    _bmc: Arc<B>,
    root: &nv_redfish::ServiceRoot<B>,
    bmc_name: &str,
    include_attributes: bool,
    max_attributes: usize,
) -> Result<Vec<Metric>, String> {
    let mut out = Vec::new();
    let Some(systems) = root.systems().await.map_err(|e| format!("systems: {e}"))? else {
        return Ok(out);
    };
    let systems = systems
        .members()
        .await
        .map_err(|e| format!("systems members: {e}"))?;
    let mut attempted = 0usize;
    let mut failed = 0usize;
    let mut remaining_attributes = max_attributes;
    let mut truncated = false;
    for system in systems {
        let system_id = system.id().to_string();
        let bios = match system.bios().await {
            Ok(Some(bios)) => {
                attempted += 1;
                bios
            }
            Ok(None) => continue,
            Err(error) => {
                attempted += 1;
                failed += 1;
                tracing::warn!(
                    bmc = %bmc_name,
                    system = %system_id,
                    error = %error,
                    "BIOS resource fetch or parse failed"
                );
                continue;
            }
        };
        let raw = bios.raw();
        out.push(
            Metric::gauge(BIOS_PENDING.0, BIOS_PENDING.1)
                .label("bmc", bmc_name.to_string())
                .label("system", system_id.clone())
                .build(if raw.settings_object().is_some() {
                    1.0
                } else {
                    0.0
                }),
        );
        let Some(attrs) = &raw.attributes else {
            continue;
        };
        if !include_attributes {
            continue;
        }
        let mut attributes = attrs.dynamic_properties.iter().collect::<Vec<_>>();
        attributes.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        if attributes.len() > remaining_attributes {
            attributes.truncate(remaining_attributes);
            truncated = true;
        }
        remaining_attributes = remaining_attributes.saturating_sub(attributes.len());
        for (name, value) in attributes {
            push_attribute(&mut out, bmc_name, &system_id, name, value);
        }
        if remaining_attributes == 0 {
            break;
        }
    }
    if truncated {
        tracing::warn!(
            bmc = %bmc_name,
            max_attributes,
            "BIOS attribute metric limit reached; remaining attributes omitted"
        );
    }
    if attempted > 0 && failed == attempted {
        return Err(format!(
            "bios: all {failed}/{attempted} declared resources failed"
        ));
    }
    Ok(out)
}

fn push_attribute(
    out: &mut Vec<Metric>,
    bmc: &str,
    system: &str,
    name: &str,
    value: &Option<EdmPrimitiveType>,
) {
    let Some(value) = value else {
        return; // null 属性（如密码项）跳过，不产出指标
    };
    match value {
        EdmPrimitiveType::String(s) => {
            if s.eq_ignore_ascii_case("Enabled") {
                push_numeric(out, bmc, system, name, 1.0);
            } else if s.eq_ignore_ascii_case("Disabled") {
                push_numeric(out, bmc, system, name, 0.0);
            } else {
                out.push(
                    Metric::gauge(BIOS_ATTR_INFO.0, BIOS_ATTR_INFO.1)
                        .label("bmc", bmc.to_string())
                        .label("system", system.to_string())
                        .label("attribute", name.to_string())
                        .label("value", s.clone())
                        .build(1.0),
                );
            }
        }
        EdmPrimitiveType::Bool(b) => {
            push_numeric(out, bmc, system, name, if *b { 1.0 } else { 0.0 })
        }
        EdmPrimitiveType::Integer(i) => push_numeric(out, bmc, system, name, *i as f64),
        EdmPrimitiveType::Decimal(d) => push_numeric(out, bmc, system, name, *d),
    }
}

fn push_numeric(out: &mut Vec<Metric>, bmc: &str, system: &str, name: &str, value: f64) {
    out.push(
        Metric::gauge(BIOS_ATTR.0, BIOS_ATTR.1)
            .label("bmc", bmc.to_string())
            .label("system", system.to_string())
            .label("attribute", name.to_string())
            .build(value),
    );
}
