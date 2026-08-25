//! 集成测试：完整采集周期、BMC 失败隔离、session 认证流程。
//!
//! 全部通过 nv-redfish-bmc-mock 模拟真实 BMC 的 HTTP 行为，验证
//! collect_fast/collect_slow 的分组采集链路与 establish_session 的会话流程。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish::schema::session::SessionCreate;
use nv_redfish_bmc_mock::Expect;
use redfish_exporter::bmc::establish_session;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::metrics::encode;
use redfish_exporter::registry::build_registry;
use serde_json::json;
use std::sync::Arc;

/// 测试 1：一个 MockBmc 覆盖快组（7 个 collector）+ 慢组（4 个 collector）
/// 的全部调用路径。
///
/// mock expectation 顺序必须与实际请求顺序一致：快组
/// sensors → power → processors → memory → systems → chassis_health → managers，
/// 慢组 storage → network → firmware → assembly → bios。
/// managers 因 root 无 Managers 链接而跳过（Ok(None)，无网络请求）。
#[tokio::test]
async fn full_scrape_cycle_with_mock_bmc() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["Chassis", "Systems", "UpdateService"]);

    // 快组：sensors（chassis 枚举）
    expect_chassis_round(&bmc, true);
    expect_sensor_payloads(&bmc);

    // 快组：power（chassis 枚举）
    expect_chassis_round(&bmc, true);
    expect_thermal_power_payloads(&bmc);

    // 快组：processors（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Processors"]);
    expect_processor_payloads(&bmc);

    // 快组：memory（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Memory"]);
    expect_memory_payloads(&bmc);

    // 快组：systems（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &[]);

    // 快组：chassis_health（chassis 枚举）
    expect_chassis_round(&bmc, true);

    // 快组：managers：root 无 Managers 链接 → 无网络请求

    // 慢组：storage（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Storage"]);
    expect_storage_payloads(&bmc);

    // 慢组：network：ethernet（systems 枚举）+ pcie（chassis 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["EthernetInterfaces"]);
    expect_ethernet_payloads(&bmc);
    expect_chassis_round(&bmc, true);
    expect_pcie_payloads(&bmc);

    // 慢组：firmware（UpdateService）
    expect_firmware_payloads(&bmc);

    // 慢组：assembly（chassis 枚举）
    expect_chassis_round(&bmc, true);
    expect_assembly_payloads(&bmc);

    // 慢组：bios（systems 枚举）
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Bios"]);
    expect_bios_payloads(&bmc);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let fast = collect_fast(Arc::clone(&bmc), &root, "bmc1").await.unwrap();
    let slow = collect_slow(Arc::clone(&bmc), &root, "bmc1").await.unwrap();
    let merged = merge_reports(fast, Some(&slow));
    let report = finalize_report(
        "bmc1",
        merged.metrics,
        merged.failed_resources,
        std::time::Instant::now(),
    );
    assert!(
        report.failed_resources.is_empty(),
        "failed_resources: {:?}",
        report.failed_resources
    );
    let metrics = &report.metrics;
    assert!(!metrics.is_empty());

    assert!(metrics.iter().any(|m| m.name == "redfish_up"
        && m.value == 1.0
        && labels_of(m).get("bmc") == Some(&"bmc1")));
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_scrape_duration_seconds")
    );

    let sensor_readings: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_sensor_reading")
        .collect();
    assert!(
        sensor_readings.len() >= 2,
        "expected ambient + thermal sensor readings, got {}",
        sensor_readings.len()
    );

    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_processor_temperature_celsius" && m.value == 55.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_processor_power_watts" && m.value == 60.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_power_consumption_watts" && m.value == 320.0)
    );
    assert!(metrics.iter().any(|m| {
        m.name == "redfish_health_status"
            && m.value == 1.0
            && labels_of(m).get("resource_type") == Some(&"system")
    }));
    assert!(metrics.iter().any(|m| m.name == "redfish_info"));
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_power_state" && m.value == 1.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_ethernet_interface_link_status")
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_bios_pending_changes" && m.value == 1.0)
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_bios_attribute" && m.value == 16.0)
    );

    // 端到端：ScrapeReport → prometheus Registry → 文本编码
    let registry = build_registry("bmc1", &report, 0).await.unwrap();
    let out = encode(&registry);
    assert!(out.contains("redfish_up{bmc=\"bmc1\"} 1"));
    assert!(out.contains("redfish_power_consumption_watts"));
    assert!(out.contains("redfish_health_status"));
}

/// 处理器频率/电压指标：Dell（CurrentClockSpeedMhz/Volts）与浪潮
/// （Public.FrequencyMHz）OEM 字段按真机探测路径采集；缺失字段不产出。
#[tokio::test]
async fn processor_frequency_voltage_metrics() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc, &["Systems"]);
    expect_systems_collection(&bmc);
    expect_system(&bmc, &["Processors"]);
    expect_processor_payloads(&bmc);

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = redfish_exporter::collector::processors::collect_processors(bmc, &root, "bmc1")
        .await
        .unwrap();

    let registry = prometheus::Registry::new();
    redfish_exporter::metrics::register_into(&metrics, &registry).unwrap();
    let out = redfish_exporter::metrics::encode(&registry);

    assert!(
        out.contains("redfish_processor_frequency_mhz{bmc=\"bmc1\",id=\"CPU1\",system=\"1\"} 2100"),
        "{out}"
    );
    assert!(
        out.contains(
            "redfish_processor_max_frequency_mhz{bmc=\"bmc1\",id=\"CPU2\",system=\"1\"} 3400"
        ),
        "{out}"
    );
    assert!(
        out.contains("redfish_processor_voltage_volts{bmc=\"bmc1\",id=\"CPU1\",system=\"1\"} 1.6"),
        "{out}"
    );
    // CPU2 无 OEM 字段：不得产出频率/电压指标（最大频率来自标准字段，仍产出）。
    assert!(
        !out.contains("redfish_processor_voltage_volts{bmc=\"bmc1\",id=\"CPU2\""),
        "{out}"
    );
    assert!(
        !out.contains("redfish_processor_frequency_mhz{bmc=\"bmc1\",id=\"CPU2\""),
        "{out}"
    );
}

/// 测试 2：BMC 失败隔离。
///
/// - 健康 BMC：完整流程成功（redfish_up=1）。
/// - 期望耗尽：root 成功但后续 GET 无期望 → 各 collector 失败进入
///   failed_resources，快/慢组合并后 up=0（隔离语义）。
/// - 根级失败：root GET 期望不匹配（UnexpectedGet）→ ServiceRoot::new 返回 Err
///   （仅 ServiceRoot 层错误会传播，collector 层错误被隔离）。
#[tokio::test]
async fn bmc_failure_isolation() {
    // 健康 BMC：root 仅含 Chassis 链接，快组 sensors/power/chassis_health 与
    // 慢组 network/assembly 共 5 个 chassis 枚举 collector 各自完成一轮
    // 集合+成员 GET，其余 collector 返回 Ok(空)。
    let ok = Arc::new(Mock::default());
    expect_service_root(&ok, &["Chassis"]);
    for _ in 0..5 {
        expect_chassis_round(&ok, false);
    }
    let root = ServiceRoot::new(Arc::clone(&ok)).await.unwrap();
    let fast = collect_fast(Arc::clone(&ok), &root, "ok-bmc")
        .await
        .unwrap();
    let slow = collect_slow(Arc::clone(&ok), &root, "ok-bmc")
        .await
        .unwrap();
    let merged = merge_reports(fast, Some(&slow));
    let report = finalize_report(
        "ok-bmc",
        merged.metrics,
        merged.failed_resources,
        std::time::Instant::now(),
    );
    assert!(report.failed_resources.is_empty());
    assert!(report.metrics.iter().any(|m| m.name == "redfish_up"
        && m.value == 1.0
        && labels_of(m).get("bmc") == Some(&"ok-bmc")));

    // 期望耗尽：root GET 之后队列为空，每个 collector 枚举失败，
    // 隔离进 failed_resources，redfish_up=0（scraper 层据此降级）。
    let exhausted = Arc::new(Mock::default());
    expect_service_root(&exhausted, &["Chassis"]);
    let root = ServiceRoot::new(Arc::clone(&exhausted)).await.unwrap();
    let fast = collect_fast(Arc::clone(&exhausted), &root, "exhausted-bmc")
        .await
        .unwrap();
    let slow = collect_slow(Arc::clone(&exhausted), &root, "exhausted-bmc")
        .await
        .unwrap();
    let merged = merge_reports(fast, Some(&slow));
    let report = finalize_report(
        "exhausted-bmc",
        merged.metrics,
        merged.failed_resources,
        std::time::Instant::now(),
    );
    assert!(!report.failed_resources.is_empty());
    assert!(report.metrics.iter().any(|m| m.name == "redfish_up"
        && m.value == 0.0
        && labels_of(m).get("bmc") == Some(&"exhausted-bmc")));

    // 根级失败：root GET 命中不匹配的期望（UnexpectedGet），
    // ServiceRoot::new 失败 → 错误传播（collect_round 中返回 Err）。
    let bad = Arc::new(Mock::default());
    bad.expect(Expect::get("/redfish/v1/not-the-real-root", json!({})));
    assert!(ServiceRoot::new(bad).await.is_err());
}

/// 测试 3：session 认证流程（泛型化 establish_session + MockBmc 全流程）。
///
/// - 单元断言：SessionCreate 构造字段正确；BmcCredentials::token 的
///   Debug 输出不泄漏 token。
/// - 全流程：root GET → SessionService GET → Sessions 集合 GET →
///   create_session（POST 负载 UserName/Password）→ 返回会话 token
///   （由调用方应用，scraper.rs 的 set_credentials 是 HttpBmc 特有能力）。
#[tokio::test]
async fn session_auth_flow() {
    let create = SessionCreate::builder("admin".into(), "secret".into()).build();
    assert_eq!(create.user_name, "admin");
    assert_eq!(create.password, "secret");
    assert!(create.token.is_none());
    assert!(create.links.is_none());

    let creds = nv_redfish::bmc_http::BmcCredentials::token("super-secret-token".into());
    let debug = format!("{creds:?}");
    assert!(!debug.contains("super-secret-token"));
    assert!(debug.contains("[REDACTED]"));

    let bmc = Arc::new(Mock::default());
    bmc.expect(Expect::get(
        ODataId::service_root(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "SessionService": { "@odata.id": "/redfish/v1/SessionService" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService",
        json!({
            "@odata.id": "/redfish/v1/SessionService",
            "Id": "SessionService", "Name": "Session Service",
            "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" },
        }),
    ));
    bmc.expect(Expect::get(
        "/redfish/v1/SessionService/Sessions",
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions",
            "@odata.type": "#SessionCollection.SessionCollection",
            "Name": "Sessions",
            "Members": [],
            "Members@odata.count": 0,
        }),
    ));
    bmc.expect(Expect::create_session(
        "/redfish/v1/SessionService/Sessions",
        json!({ "UserName": "admin", "Password": "secret" }),
        json!({
            "@odata.id": "/redfish/v1/SessionService/Sessions/1",
            "Id": "1", "Name": "User Session",
            "UserName": "admin",
            "SessionType": "Redfish",
        }),
        "session-token-123",
        "/redfish/v1/SessionService/Sessions/1",
    ));

    let est = establish_session(&bmc, "admin", "secret").await.unwrap();
    assert_eq!(est.token, "session-token-123");
}
