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
    let mut out = Vec::new();
    let Some(systems) = root.systems().await.map_err(|e| format!("systems: {e}"))? else {
        return Ok(out);
    };
    let systems = systems
        .members()
        .await
        .map_err(|e| format!("systems members: {e}"))?;
    for system in systems {
        let system_id = system.id().to_string();
        // BIOS 解析失败（如属性值含列表/对象）时跳过该系统
        let Ok(Some(bios)) = system.bios().await else {
            continue;
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
        for (name, value) in &attrs.dynamic_properties {
            push_attribute(&mut out, bmc_name, &system_id, name, value);
        }
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
