//! 资源基准（#[ignore]，本地跑）：mock 环境 RSS-vs-BMC 数曲线与每 BMC 内存系数，
//! 验证性能维移交的预编码缓存增量（真机 ~1.1MB/BMC 在 mock 规模下对应编码字节稳定性）。
//!   cargo test --profile release-gates --test resource_test -- --ignored --nocapture
//! 硬断言（无条件）：预编码字节逐 BMC 稳定（±1 字节 mock 值方差）、RSS 随 BMC 数单调不减。
//! 软断言（仅 $env:RES_ASSERT="1" 时启用，本地验证用，不进 CI）：每 BMC RSS 增量 < 25MB。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::Bmc as MockBmc;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::registry::{Snapshot, build_registry};
use std::sync::Arc;
use std::time::Instant;

fn expect_full_round(bmc: &Mock) {
    expect_service_root(bmc, &["Chassis", "Systems", "UpdateService"]);
    expect_chassis_round(bmc, true);
    expect_sensor_payloads(bmc);
    expect_chassis_round(bmc, true);
    expect_thermal_power_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Processors"]);
    expect_processor_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Memory"]);
    expect_memory_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &[]);
    expect_chassis_round(bmc, true);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Storage"]);
    expect_storage_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["EthernetInterfaces"]);
    expect_ethernet_payloads(bmc);
    expect_chassis_round(bmc, true);
    expect_pcie_payloads(bmc);
    expect_firmware_payloads(bmc);
    expect_chassis_round(bmc, true);
    expect_assembly_payloads(bmc);
    expect_systems_collection(bmc);
    expect_system(bmc, &["Bios"]);
    expect_bios_payloads(bmc);
}

fn rss_mb() -> f64 {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_process(pid);
    sys.process(pid)
        .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
        .unwrap_or(0.0)
}

/// RSS-vs-BMC 数曲线：1..=4 个 BMC 逐个入快照，记录每步 RSS 与预编码字节长度。
#[tokio::test]
#[ignore]
async fn resource_memory_per_bmc_curve() {
    let snap = Arc::new(Snapshot::new());
    let mut rss = Vec::with_capacity(4);
    let mut encoded = Vec::with_capacity(4);
    for n in 1..=4u32 {
        let name = format!("bmc{n}");
        let bmc = Arc::new(MockBmc::default());
        expect_full_round(&bmc);
        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let fast = collect_fast(Arc::clone(&bmc), &root, &name).await.unwrap();
        let slow = collect_slow(
            Arc::clone(&bmc),
            &root,
            &name,
            redfish_exporter::config::CollectorsConfig::all_enabled(),
        )
        .await
        .unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report(
            &name,
            merged.metrics,
            merged.failed_resources,
            Instant::now(),
        );
        assert!(
            report.failed_resources.is_empty(),
            "mock 全量采集不应有失败资源"
        );
        let registry = build_registry(&name, &report, 0).await.unwrap();
        snap.update(&name, registry);
        encoded.push(snap.encoded().expect("刚更新的 BMC 必须在统一快照中").len());
        rss.push(rss_mb());
    }
    let coef = (rss[3] - rss[0]) / 3.0;
    println!(
        "resource: rss_1_2_3_4_mb={} encoded_bytes_per_bmc={}",
        rss.iter()
            .map(|v| format!("{v:.2}"))
            .collect::<Vec<_>>()
            .join(","),
        encoded
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    println!("resource: per_bmc_rss_delta_mb={coef:.2}");
    // 统一预编码缓存随 BMC 数单调增长；第二个 BMC 起每个同规模 BMC 的样本增量
    // 应稳定（HELP/TYPE 与全局 build_info 只在第一份中出现）。
    assert!(
        encoded.windows(2).all(|w| w[1] > w[0]),
        "combined encoded bytes must grow with BMC count: {encoded:?}"
    );
    let deltas: Vec<_> = encoded.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        deltas
            .windows(2)
            .all(|w| (w[0] as i64 - w[1] as i64).abs() <= 1),
        "per-BMC combined-output increments must be stable within ±1 byte: {deltas:?}"
    );
    assert!(
        rss.windows(2).all(|w| w[1] >= w[0]),
        "RSS 应随 BMC 数单调不减: {rss:?}"
    );
    if std::env::var("RES_ASSERT").is_ok() {
        // 软阈值：mock 规模每 BMC 增量 < 25MB（registry 结构 + 编码缓存；真机全量快照才 ~44MB WS）。
        assert!(coef < 25.0, "per-BMC RSS delta {coef}MB >= 25MB");
    }
}
