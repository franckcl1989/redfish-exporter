//! 性能基准（#[ignore]，本地跑）：确定性 mock 环境的端到端轮耗时、编码耗时、
//! 预编码 /metrics 热路径耗时与 RSS 采样。
//!   cargo test --release --test perf_test -- --ignored --nocapture
//!   $env:PERF_ASSERT="1" 时启用软阈值断言（本地验证用，不进 CI）。

mod common;
use common::*;

use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::Bmc as MockBmc;
use redfish_exporter::collector::{collect_fast, collect_slow, finalize_report, merge_reports};
use redfish_exporter::metrics::encode_bytes;
use redfish_exporter::registry::{Snapshot, build_registry};
use std::sync::Arc;
use std::time::Instant;

fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

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

#[tokio::test]
#[ignore]
async fn perf_benchmark_mock_pipeline() {
    const ROUNDS: usize = 20;
    let mut round_ms = Vec::with_capacity(ROUNDS);
    let mut encode_ms = Vec::with_capacity(ROUNDS);
    let mut hot_ms = Vec::with_capacity(ROUNDS);
    let mut bytes_lens = Vec::with_capacity(ROUNDS);
    let snap = Arc::new(Snapshot::new());

    for _ in 0..ROUNDS {
        let bmc = Arc::new(MockBmc::default());
        expect_full_round(&bmc);
        let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
        let t0 = Instant::now();
        let fast = collect_fast(Arc::clone(&bmc), &root, "perf").await.unwrap();
        let slow = collect_slow(Arc::clone(&bmc), &root, "perf").await.unwrap();
        let merged = merge_reports(fast, Some(&slow));
        let report = finalize_report("perf", merged.metrics, merged.failed_resources, t0);
        assert!(report.failed_resources.is_empty());
        let registry = build_registry("perf", &report, 0).await.unwrap();
        round_ms.push(t0.elapsed().as_secs_f64() * 1000.0);

        let te = Instant::now();
        let encoded = encode_bytes(&registry);
        encode_ms.push(te.elapsed().as_secs_f64() * 1000.0);
        bytes_lens.push(encoded.len());

        // 预编码 /metrics 热路径：拼接各 BMC 条目字节（单 BMC）
        snap.update("perf", registry);
        let th = Instant::now();
        let entries = snap.registries();
        let total: usize = entries.iter().map(|(_, e)| e.encoded.len()).sum();
        let mut body: Vec<u8> = Vec::with_capacity(total);
        for (_, e) in &entries {
            body.extend_from_slice(&e.encoded);
        }
        hot_ms.push(th.elapsed().as_secs_f64() * 1000.0);
    }

    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_process(pid);
    let rss_mb = sys
        .process(pid)
        .map(|p| p.memory() as f64 / 1024.0 / 1024.0)
        .unwrap_or(0.0);

    let (rm, em, hm) = (median(&round_ms), median(&encode_ms), median(&hot_ms));
    let hm_min = hot_ms.iter().cloned().fold(f64::INFINITY, f64::min);
    let hm_max = hot_ms.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let bytes_min = *bytes_lens.iter().min().unwrap();
    let bytes_max = *bytes_lens.iter().max().unwrap();
    let bytes_med = median(&bytes_lens.iter().map(|&b| b as f64).collect::<Vec<_>>());
    println!(
        "perf: rounds={ROUNDS} round_ms_med={rm:.2} encode_ms_med={em:.2} hot_path_ms_min={hm_min:.4}/med={hm:.4}/max={hm_max:.4} bytes_min={bytes_min}/med={bytes_med:.0}/max={bytes_max} rss_mb={rss_mb:.1}"
    );
    if std::env::var("PERF_ASSERT").is_ok() {
        assert!(hm < 10.0, "hot path median {hm}ms >= 10ms");
        assert!(em < 500.0, "encode median {em}ms >= 500ms");
        assert!(rm < 2000.0, "round median {rm}ms >= 2000ms");
    }
}
