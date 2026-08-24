//! 长稳 soak（#[ignore]，发布前手动运行）：
//!   cargo test --release --test soak_test -- --ignored
//! 时长由 SOAK_SECS 环境变量控制（默认 14400s=4h，验收实跑 ≥7200s）。
//! 健康 MockBmc 长期循环：断言轮耗时稳定、无失败资源、输出大小恒定、RSS 无泄漏式增长。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::Bmc as MockBmc;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::registry::build_registry;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
#[ignore]
async fn soak_healthy_mock_bmc() {
    let secs: u64 = std::env::var("SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14400);
    let interval = Duration::from_secs(1);
    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs);
    let mut samples: Vec<(f64, f64, f64, usize)> = Vec::new(); // (elapsed_s, round_ms, rss_mb, bytes)
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut lookup_failed = false;

    while Instant::now() < deadline {
        let bmc = Arc::new(MockBmc::default());
        expect_service_root(&bmc, &["Chassis", "Systems", "UpdateService"]);
        expect_chassis_round(&bmc, true);
        expect_sensor_payloads(&bmc);
        expect_chassis_round(&bmc, true);
        expect_thermal_power_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Processors"]);
        expect_processor_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Memory"]);
        expect_memory_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &[]);
        expect_chassis_round(&bmc, true);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Storage"]);
        expect_storage_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["EthernetInterfaces"]);
        expect_ethernet_payloads(&bmc);
        expect_chassis_round(&bmc, true);
        expect_pcie_payloads(&bmc);
        expect_firmware_payloads(&bmc);
        expect_chassis_round(&bmc, true);
        expect_assembly_payloads(&bmc);
        expect_systems_collection(&bmc);
        expect_system(&bmc, &["Bios"]);
        expect_bios_payloads(&bmc);

        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let t0 = Instant::now();
        let fast = collect_fast(Arc::clone(&bmc), &root, "soak").await.unwrap();
        let slow = collect_slow(Arc::clone(&bmc), &root, "soak").await.unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report("soak", merged.metrics, merged.failed_resources, t0);
        assert!(
            report.failed_resources.is_empty(),
            "soak round failed: {:?}",
            report.failed_resources
        );
        let registry = build_registry("soak", &report, 0).await.unwrap();
        let out = redfish_exporter::metrics::encode(&registry);
        let round_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let rss_mb = match (sys.refresh_process(pid), sys.process(pid)) {
            (true, Some(p)) => p.memory() as f64 / 1024.0 / 1024.0,
            // RSS 采样失败不静默记 0：标记后由结尾断言统一判红。
            _ => {
                lookup_failed = true;
                0.0
            }
        };
        samples.push((start.elapsed().as_secs_f64(), round_ms, rss_mb, out.len()));
        tokio::time::sleep(interval).await;
    }

    assert!(!lookup_failed, "RSS sampling failed during soak");
    assert!(
        samples.len() >= 10,
        "soak too short: {} rounds",
        samples.len()
    );
    let mid = samples.len() / 2;
    let first = &samples[..mid];
    let second = &samples[mid..];
    let median = |xs: &[(f64, f64, f64, usize)], idx: usize| -> f64 {
        let mut v: Vec<f64> = xs
            .iter()
            .map(|s| match idx {
                1 => s.1,
                2 => s.2,
                _ => s.3 as f64,
            })
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let m_round_first = median(first, 1);
    let m_round_second = median(second, 1);
    assert!(
        m_round_second <= m_round_first * 3.0 + 10.0,
        "round time drifted: first median {m_round_first}ms, second {m_round_second}ms"
    );
    let m_rss_first = median(first, 2);
    let m_rss_second = median(second, 2);
    assert!(
        m_rss_second <= m_rss_first * 1.5 + 5.0,
        "RSS drifted: first median {m_rss_first}MB, second {m_rss_second}MB"
    );
    let m_bytes_first = median(first, 3);
    let m_bytes_second = median(second, 3);
    assert_eq!(
        m_bytes_first as usize, m_bytes_second as usize,
        "output size drifted"
    );
    println!(
        "soak ok: {} rounds, round_ms med {}->{}, rss_mb med {}->{}, bytes {}",
        samples.len(),
        m_round_first,
        m_round_second,
        m_rss_first,
        m_rss_second,
        m_bytes_second
    );
}
