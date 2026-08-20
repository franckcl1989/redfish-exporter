# Metrics reference

All metrics are **Gauge** type. Metric names, help strings and labels below are extracted from the implementation (`src/metrics.rs` constants and the collectors in `src/collector/`) and are authoritative for version 0.1.0.

Values are pushed on every scrape; labels whose source field is absent on the BMC are emitted as the empty string (or `unknown` for health/state). The Prometheus registry is rebuilt per BMC per round from the scrape snapshot.

## Common labels

| Label          | Meaning                                                        |
|----------------|----------------------------------------------------------------|
| `bmc`          | Config name of the BMC (see `bmcs[].name` in the config file)  |
| `system`       | ComputerSystem resource id                                      |
| `chassis`      | Chassis resource id                                             |
| `id`           | Resource id of the reported entity (processor, memory module, drive, volume, ethernet interface, PCIe device) |
| `storage`      | Storage (storage controller) resource id                        |
| `resource_type`| Redfish resource type reported by `redfish_health_status`      |
| `health`       | `OK` / `Warning` / `Critical` / `UnsupportedValue` / `unknown` |
| `state`        | Resource state (e.g. `Enabled`) / `unknown`                    |

## A. Implemented metrics

### Common / per-BMC

| Metric                             | Labels                       | Help text from code                                      | Source |
|------------------------------------|------------------------------|----------------------------------------------------------|--------|
| `redfish_up`                       | `bmc`                        | Whether the last scrape of this BMC succeeded            | per-BMC scrape result (`collect_all`); forced to 0 on total failure |
| `redfish_scrape_duration_seconds`  | `bmc`                        | Duration of the last scrape of this BMC                  | `collect_all` wall time |
| `redfish_scrape_error`             | `bmc`, `resource`            | Set to 1 when the last scrape of a resource failed       | one series per failed resource (`registry.rs`) |
| `redfish_health_status`            | `bmc`, `resource_type`, `id`, `health`, `state` | Health and state of a resource      | `Status.Health` + `Status.State` of any collected resource; always `1.0` |
| `redfish_info`                     | `bmc`, `key`, `value`        | Static key-value information about a BMC                 | inventory fields, always `1.0` |

`redfish_health_status` `resource_type` values: `system`, `chassis`, `manager`, `processor`, `memory`, `drive`, `volume`, `ethernet_interface`, `network_adapter`, `port`, `pcie_device`, `assembly`, `software_inventory`.

`redfish_info` keys: `manufacturer`, `model`, `processor_type`, `part_number`, `memory_type`, `serial_number`, `revision`, `sku`, `firmware_version`, `software_version`, `producer`, `mac_address`, `name`.

### Sensors (chassis)

| Metric                                      | Labels                                                        | Help text from code      | Source |
|---------------------------------------------|---------------------------------------------------------------|--------------------------|--------|
| `redfish_sensor_reading`                    | `bmc`, `chassis`, `name`, `units`, `sensor_type`, `health`, `state` | Sensor reading     | Chassis sensor links (`sensors.rs`), legacy `Thermal` temperatures, power supply sensors, `EnvironmentMetrics` sensors, `Controls` (`power.rs`) |
| `redfish_sensor_threshold_upper_critical`   | same as above                                                 | Sensor threshold         | `Thresholds.upper_critical` |
| `redfish_sensor_threshold_upper_warning`    | same as above                                                 | Sensor threshold         | `Thresholds.upper_caution` |
| `redfish_sensor_threshold_lower_warning`    | same as above                                                 | Sensor threshold         | `Thresholds.lower_caution` |
| `redfish_sensor_threshold_lower_critical`   | same as above                                                 | Sensor threshold         | `Thresholds.lower_critical` |

### Power

| Metric                            | Labels          | Help text from code                        | Source |
|-----------------------------------|-----------------|--------------------------------------------|--------|
| `redfish_power_consumption_watts` | `bmc`, `chassis`| Total chassis power consumption in watts   | legacy `PowerControl.PowerConsumedWatts` (first control) |

### Processors

| Metric                                | Labels                | Help text from code                              | Source |
|---------------------------------------|-----------------------|--------------------------------------------------|--------|
| `redfish_processor_temperature_celsius` | `bmc`, `system`, `id` | Processor temperature in Celsius                  | `ProcessorMetrics.temperature_celsius` |
| `redfish_processor_power_watts`       | `bmc`, `system`, `id` | Processor power in watts                          | `ProcessorMetrics.consumed_power_watt` |
| `redfish_processor_bandwidth_percent` | `bmc`, `system`, `id` | Processor bandwidth utilization percentage        | `ProcessorMetrics.bandwidth_percent` |

### Memory

| Metric                            | Labels                | Help text from code                          | Source |
|-----------------------------------|-----------------------|----------------------------------------------|--------|
| `redfish_memory_capacity_bytes`   | `bmc`, `system`, `id` | Memory capacity in bytes                     | `Memory.capacity_mib` × 1,048,576 |
| `redfish_memory_bandwidth_percent`| `bmc`, `system`, `id` | Memory bandwidth utilization percentage      | `MemoryMetrics.bandwidth_percent` |

### Storage

| Metric                                       | Labels                          | Help text from code                                                            | Source |
|----------------------------------------------|---------------------------------|--------------------------------------------------------------------------------|--------|
| `redfish_drive_capacity_bytes`               | `bmc`, `system`, `storage`, `id`| Drive capacity in bytes                                                        | `Drive.capacity_bytes` |
| `redfish_drive_life_left_percent`            | `bmc`, `system`, `storage`, `id`| Drive remaining media life percentage                                          | `Drive.predicted_media_life_left_percent` |
| `redfish_drive_predictive_failure`           | `bmc`, `system`, `storage`, `id`| Whether the drive predicts a failure in the near future, 1 = failure predicted | `Drive.failure_predicted` |
| `redfish_drive_io_read_correctable_errors_total`   | `bmc`, `system`, `storage`, `id`| Lifetime number of correctable read errors reported by the drive      | `DriveMetrics.correctable_io_read_error_count` |
| `redfish_drive_io_write_correctable_errors_total`  | `bmc`, `system`, `storage`, `id`| Lifetime number of correctable write errors reported by the drive     | `DriveMetrics.correctable_io_write_error_count` |
| `redfish_drive_io_read_uncorrectable_errors_total` | `bmc`, `system`, `storage`, `id`| Lifetime number of uncorrectable read errors reported by the drive    | `DriveMetrics.uncorrectable_io_read_error_count` |
| `redfish_drive_io_write_uncorrectable_errors_total`| `bmc`, `system`, `storage`, `id`| Lifetime number of uncorrectable write errors reported by the drive   | `DriveMetrics.uncorrectable_io_write_error_count` |
| `redfish_volume_capacity_bytes`              | `bmc`, `system`, `storage`, `id`| Volume capacity in bytes                                                      | `Volume.capacity_bytes` |

Note: the `redfish_drive_io_*_errors_total` metrics follow counter naming conventions (`_total` suffix) but are registered as **Gauge** carrying the last scraped lifetime value, consistent with the snapshot-cache design.

### Network

| Metric                                 | Labels                | Help text from code                                      | Source |
|----------------------------------------|-----------------------|----------------------------------------------------------|--------|
| `redfish_ethernet_interface_link_status`| `bmc`, `system`, `id` | Ethernet link status, 1 = up                             | `EthernetInterface.link_status` (1 = LinkUp, 0 = otherwise) |
| `redfish_ethernet_interface_speed_mbps` | `bmc`, `system`, `id` | Ethernet link speed in Mbps                              | `EthernetInterface.speed_mbps` |
| `redfish_pcie_device_lanes_in_use`     | `bmc`, `id`           | Number of PCIe lanes in use by the device                | `PcieInterface.lanes_in_use` |
| `redfish_pcie_device_max_lanes`        | `bmc`, `id`           | Maximum number of PCIe lanes supported by the device     | `PcieInterface.max_lanes` |

### System

| Metric            | Labels          | Help text from code                | Source |
|-------------------|-----------------|------------------------------------|--------|
| `redfish_power_state` | `bmc`, `system` | Power state of a system, 1 = On  | `System.power_state`; series emitted only when the state is `On` |

## B. Recorded but not implemented

The following areas were evaluated during 0.1.0 scoping and are deliberately **not collected**. Reasons and directions for future versions:

| Item | Reason | Direction |
|------|--------|-----------|
| LogService, EventService, TaskService, AccountService, SessionService metrics | Out of scope for 0.1.0 (read-only sensor/compliance focus); session usage is internal | dedicated collectors; EventService subscription-based eventing is a larger design |
| BIOS, BootOptions, SecureBoot, HostInterfaces, ManagerNetworkProtocol | Out of scope; volatile/firmware-config rather than monitoring data | `redfish_info`-style versioning collectors |
| All OEM extension resources | Vendor-specific schemas, high maintenance cost | opt-in collectors behind config flags |
| `redfish_power_input_watts` | Constant defined in `src/metrics.rs` but no collector pushes it (chassis input power is not reported by the legacy Power schema in nv-redfish) | emit when the BMC distinguishes input vs consumed power |
| `redfish_processor_utilization_percent` | `ProcessorSummary` has no utilization field (constant reserved in `metrics.rs`) | collect per-core utilization / `OperatingConfig` once exposed by nv-redfish |
| `redfish_drive_utilization_percent` | `DriveMetrics` has no utilization field (constant reserved in `metrics.rs`) | SMART-based estimate or nv-redfish schema extension |
| PCIe link rate (GT/s) | `PcieDevice` has no link-speed field | nv-redfish schema extension (needs `PcieDevice.PcieDeviceProperties` extension data) |
| StorageController health | Storage references are `ReferenceLeaf` (no embedded data); drives/volumes are collected instead | fetch each controller resource individually |
