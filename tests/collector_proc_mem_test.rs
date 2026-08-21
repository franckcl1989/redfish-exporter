use nv_redfish::ServiceRoot;
use nv_redfish::core::ODataId;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::collector::memory::collect_memory;
use redfish_exporter::collector::processors::collect_processors;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

type Mock = MockBmc<serde_json::Error>;

fn expect_service_root(bmc: &Mock) -> ODataId {
    let root_id = ODataId::service_root();
    bmc.expect(Expect::get(
        root_id.clone(),
        json!({
            "@odata.id": "/redfish/v1",
            "Id": "Root", "Name": "Root", "RedfishVersion": "1.0.0",
            "Links": { "Sessions": { "@odata.id": "/redfish/v1/SessionService/Sessions" } },
            "Chassis": { "@odata.id": "/redfish/v1/Chassis" },
            "Systems": { "@odata.id": "/redfish/v1/Systems" },
        }),
    ));
    root_id
}

fn expect_systems_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems",
        json!({
            "@odata.id": "/redfish/v1/Systems",
            "@odata.type": "#ComputerSystemCollection.ComputerSystemCollection",
            "Name": "Systems",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_system(bmc: &Mock, with_processors: bool, with_memory: bool) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1",
        "Id": "1", "Name": "System 1", "SystemType": "Physical",
        "Status": { "Health": "OK", "State": "Enabled" },
        "ProcessorSummary": { "Count": 2, "Model": "Xeon Gold" },
    });
    if with_processors {
        payload["Processors"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Processors" });
    }
    if with_memory {
        payload["Memory"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Memory" });
    }
    bmc.expect(Expect::get("/redfish/v1/Systems/1", payload));
}

fn expect_processor_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Processors",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Processors",
            "@odata.type": "#ProcessorCollection.ProcessorCollection",
            "Name": "Processor Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_processor(bmc: &Mock, with_metrics: bool) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1",
        "Id": "CPU1", "Name": "CPU 1", "ProcessorType": "CPU",
        "Status": { "Health": "OK", "State": "Enabled" },
        "Manufacturer": "Intel", "Model": "Xeon Gold 6338",
    });
    if with_metrics {
        payload["Metrics"] =
            json!({ "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1/Metrics" });
    }
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Processors/CPU1",
        payload,
    ));
}

fn expect_memory_collection(bmc: &Mock) {
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory",
            "@odata.type": "#MemoryCollection.MemoryCollection",
            "Name": "Memory Collection",
            "Members": [{ "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1" }],
            "Members@odata.count": 1,
        }),
    ));
}

fn expect_memory(bmc: &Mock, with_metrics: bool) {
    let mut payload = json!({
        "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1",
        "Id": "DIMM1", "Name": "DIMM 1", "MemoryType": "DRAM",
        "Status": { "Health": "OK", "State": "Enabled" },
        "Manufacturer": "Samsung", "PartNumber": "M393A2K43DB3-CWE",
        "CapacityMiB": 16384,
    });
    if with_metrics {
        payload["Metrics"] = json!({ "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1/Metrics" });
    }
    bmc.expect(Expect::get("/redfish/v1/Systems/1/Memory/DIMM1", payload));
}

fn labels_of(metric: &redfish_exporter::metrics::Metric) -> HashMap<&'static str, &str> {
    metric
        .labels
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect()
}

#[tokio::test]
async fn collects_processor_metrics() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true, true);
    expect_processor_collection(&bmc);
    expect_processor(&bmc, true);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Processors/CPU1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Processors/CPU1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "TemperatureCelsius": 55.0,
            "ConsumedPowerWatt": 60.0,
            "BandwidthPercent": 45.0,
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_processors(bmc, &root, "bmc1").await.unwrap();

    let temperature: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_processor_temperature_celsius")
        .collect();
    assert_eq!(temperature.len(), 1);
    assert_eq!(temperature[0].value, 55.0);
    let labels = labels_of(temperature[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"CPU1"));

    let power: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_processor_power_watts")
        .collect();
    assert_eq!(power.len(), 1);
    assert_eq!(power[0].value, 60.0);

    let bandwidth: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_processor_bandwidth_percent")
        .collect();
    assert_eq!(bandwidth.len(), 1);
    assert_eq!(bandwidth[0].value, 45.0);

    assert!(
        !metrics
            .iter()
            .any(|m| m.name == "redfish_processor_utilization_percent")
    );

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 1);
    let labels = labels_of(health[0]);
    assert_eq!(labels.get("resource_type"), Some(&"processor"));
    assert_eq!(labels.get("id"), Some(&"CPU1"));
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));
    assert_eq!(health[0].value, 1.0);

    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_info")
        .collect();
    let info_values: HashMap<_, _> = info
        .iter()
        .map(|m| {
            let labels = labels_of(m);
            (
                labels.get("key").copied().unwrap(),
                labels.get("value").copied().unwrap(),
            )
        })
        .collect();
    assert_eq!(info_values.get("manufacturer"), Some(&"Intel"));
    assert_eq!(info_values.get("model"), Some(&"Xeon Gold 6338"));
    assert_eq!(info_values.get("processor_type"), Some(&"Cpu"));
}

#[tokio::test]
async fn collects_memory_metrics() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, false, true);
    expect_memory_collection(&bmc);
    expect_memory(&bmc, true);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "BandwidthPercent": 30.0,
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_memory(bmc, &root, "bmc1").await.unwrap();

    let capacity: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_memory_capacity_bytes")
        .collect();
    assert_eq!(capacity.len(), 1);
    assert_eq!(capacity[0].value, 16384.0 * 1048576.0);
    let labels = labels_of(capacity[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"DIMM1"));

    let bandwidth: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_memory_bandwidth_percent")
        .collect();
    assert_eq!(bandwidth.len(), 1);
    assert_eq!(bandwidth[0].value, 30.0);

    let health: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 1);
    let labels = labels_of(health[0]);
    assert_eq!(labels.get("resource_type"), Some(&"memory"));
    assert_eq!(labels.get("id"), Some(&"DIMM1"));
    assert_eq!(labels.get("health"), Some(&"OK"));
    assert_eq!(labels.get("state"), Some(&"Enabled"));

    let info: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_info")
        .collect();
    let info_values: HashMap<_, _> = info
        .iter()
        .map(|m| {
            let labels = labels_of(m);
            (
                labels.get("key").copied().unwrap(),
                labels.get("value").copied().unwrap(),
            )
        })
        .collect();
    assert_eq!(info_values.get("manufacturer"), Some(&"Samsung"));
    assert_eq!(info_values.get("part_number"), Some(&"M393A2K43DB3-CWE"));
    assert_eq!(info_values.get("memory_type"), Some(&"Dram"));
}

#[tokio::test]
async fn collects_memory_ecc_alarm_trips() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, false, true);
    expect_memory_collection(&bmc);
    expect_memory(&bmc, true);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "BandwidthPercent": 30.0,
            "HealthData": {
                "AlarmTrips": {
                    "CorrectableECCError": true,
                    "UncorrectableECCError": false,
                }
            },
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_memory(bmc, &root, "bmc1").await.unwrap();

    let correctable: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_memory_correctable_errors")
        .collect();
    assert_eq!(correctable.len(), 1);
    assert_eq!(correctable[0].value, 1.0);
    let labels = labels_of(correctable[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"DIMM1"));

    let uncorrectable: Vec<_> = metrics
        .iter()
        .filter(|m| m.name == "redfish_memory_uncorrectable_errors")
        .collect();
    assert_eq!(uncorrectable.len(), 1);
    assert_eq!(uncorrectable[0].value, 0.0);
    let labels = labels_of(uncorrectable[0]);
    assert_eq!(labels.get("bmc"), Some(&"bmc1"));
    assert_eq!(labels.get("system"), Some(&"1"));
    assert_eq!(labels.get("id"), Some(&"DIMM1"));
}

#[tokio::test]
async fn memory_metrics_without_health_data_emit_no_error_series() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, false, true);
    expect_memory_collection(&bmc);
    expect_memory(&bmc, true);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1/Metrics",
            "Id": "Metrics", "Name": "Metrics",
            "BandwidthPercent": 30.0,
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let metrics = collect_memory(bmc, &root, "bmc1").await.unwrap();

    assert!(
        !metrics
            .iter()
            .any(|m| m.name == "redfish_memory_correctable_errors"
                || m.name == "redfish_memory_uncorrectable_errors")
    );
    assert!(
        metrics
            .iter()
            .any(|m| m.name == "redfish_memory_bandwidth_percent")
    );
}

#[tokio::test]
async fn missing_metrics_links_produce_info_only() {
    let bmc = Arc::new(Mock::default());
    expect_service_root(&bmc);
    expect_systems_collection(&bmc);
    expect_system(&bmc, true, true);
    expect_processor_collection(&bmc);
    expect_processor(&bmc, false);
    expect_systems_collection(&bmc);
    expect_system(&bmc, false, true);
    expect_memory_collection(&bmc);
    bmc.expect(Expect::get(
        "/redfish/v1/Systems/1/Memory/DIMM1",
        json!({
            "@odata.id": "/redfish/v1/Systems/1/Memory/DIMM1",
            "Id": "DIMM1", "Name": "DIMM 1", "MemoryType": "DRAM",
            "Status": { "Health": "OK", "State": "Enabled" },
            "Manufacturer": "Samsung", "PartNumber": "M393A2K43DB3-CWE",
        }),
    ));

    let root = ServiceRoot::new(Arc::clone(&bmc)).await.unwrap();
    let proc_metrics = collect_processors(Arc::clone(&bmc), &root, "bmc1")
        .await
        .unwrap();
    let mem_metrics = collect_memory(bmc, &root, "bmc1").await.unwrap();

    for metrics in [&proc_metrics, &mem_metrics] {
        assert!(
            !metrics
                .iter()
                .any(|m| m.name.starts_with("redfish_processor_"))
        );
        assert!(
            !metrics
                .iter()
                .any(|m| m.name.starts_with("redfish_memory_"))
        );
    }

    let health: Vec<_> = proc_metrics
        .iter()
        .chain(&mem_metrics)
        .filter(|m| m.name == "redfish_health_status")
        .collect();
    assert_eq!(health.len(), 2);
    for m in &health {
        assert_eq!(m.value, 1.0);
    }

    let info: Vec<_> = proc_metrics
        .iter()
        .chain(&mem_metrics)
        .filter(|m| m.name == "redfish_info")
        .collect();
    assert!(info.iter().any(|m| {
        let labels = labels_of(m);
        labels.get("key") == Some(&"manufacturer") && labels.get("value") == Some(&"Intel")
    }));
    assert!(info.iter().any(|m| {
        let labels = labels_of(m);
        labels.get("key") == Some(&"part_number")
            && labels.get("value") == Some(&"M393A2K43DB3-CWE")
    }));
}
