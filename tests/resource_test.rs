//! 资源基准（#[ignore]，本地跑）：mock 环境 RSS-vs-BMC 数曲线与每 BMC 内存系数，
//! 验证性能维移交的预编码缓存增量（真机 ~1.1MB/BMC 在 mock 规模下对应编码字节稳定性）。
//!   cargo test --release --test resource_test -- --ignored --nocapture
//!   $env:RES_ASSERT="1" 时启用软阈值断言（本地验证用，不进 CI）。

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
        let bmc = Arc::new(MockBmc::default());
        expect_full_round(&bmc);
        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let fast = collect_fast(Arc::clone(&bmc), &root, "res").await.unwrap();
        let slow = collect_slow(Arc::clone(&bmc), &root, "res").await.unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report(
            "res",
            merged.metrics,
            merged.failed_resources,
            Instant::now(),
        );
        assert!(
            report.failed_resources.is_empty(),
            "mock 全量采集不应有失败资源"
        );
        let registry = build_registry("res", &report, 0).await.unwrap();
        snap.update(&format!("bmc{n}"), registry);
        let entries = snap.registries();
        let entry = entries
            .iter()
            .find(|(name, _)| name == &format!("bmc{n}"))
            .expect("刚更新的 BMC 必须在快照中");
        encoded.push(entry.1.encoded.len());
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
    // 同一 mock 规模的预编码字节必须逐 BMC 稳定（预编码缓存确定性）。
    assert!(
        encoded.windows(2).all(|w| w[0] == w[1]),
        "per-BMC encoded bytes must be stable: {encoded:?}"
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
