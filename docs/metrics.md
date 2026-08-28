# Metrics reference

Metrics are Gauges unless their name ends in `_total`; `_total` families are Prometheus Counters. Metric names, help strings and labels below are extracted from the implementation (`src/metrics.rs` constants and the collectors in `src/collector/`) and are authoritative for version 0.1.0.

Values are pushed on every scrape; labels whose source field is absent on the BMC are emitted as the empty string (or `unknown` for health/state). The Prometheus registry is rebuilt per BMC per round from the scrape snapshot.

The default production profile excludes the two unbounded-cardinality data sets: per-entry event logs and full BIOS attributes. `redfish_event_log_entry`, `redfish_bios_attribute`, and `redfish_bios_attribute_info` require explicit `collectors` opt-ins and remain subject to per-BMC snapshot caps. `redfish_bios_pending_changes` is low-cardinality and stays enabled. On the validated IEIT/Inspur system, event logs plus BIOS attributes represented roughly 98% of the otherwise emitted series, so this default materially reduces Prometheus storage and churn without removing core hardware telemetry.

## Common labels

| Label          | Meaning                                                        |
|----------------|----------------------------------------------------------------|
| `bmc`          | Config name of the BMC (see `bmcs[].name` in the config file)  |
| `system`       | ComputerSystem resource id                                      |
| `chassis`      | Chassis resource id                                             |
| `manager`      | Manager resource id                                             |
| `service`      | LogService resource id                                          |
| `attribute`    | BIOS attribute name                                             |
| `id`           | Stable identity of the reported entity. For `redfish_health_status`, `redfish_info`, and sensor families this is the full Redfish `@odata.id` URI (including a fragment for embedded legacy members); specialized families retain the native leaf resource/member id and include parent labels where needed. |
| `storage`      | Storage (storage controller) resource id                        |
| `resource_type`| Redfish resource type reported by health, inventory and indicator metrics |
| `health`       | `OK` / `Warning` / `Critical` / `UnsupportedValue` / `unknown` |
| `state`        | Resource state (e.g. `Enabled`) / `unknown`                    |
| `status`       | Storage controller state (`StorageControllers[].Status.State`) |
| `model`        | Storage controller model name                                  |
| `firmware_version` | Storage controller firmware version                        |
| `wwn`          | Drive vendor identifier (Dell `DellPhysicalDisk.WWN`)          |
| `raid_status`  | Drive vendor OEM RAID status (Dell `DellPhysicalDisk.RaidStatus`) |
| `power_status` | Drive vendor OEM power status (Dell `DellPhysicalDisk.PowerStatus`) |

## A. Implemented metrics

### Common / per-BMC

| Metric                             | Labels                       | Help text from code                                      | Source |
|------------------------------------|------------------------------|----------------------------------------------------------|--------|
| `redfish_up`                       | `bmc`                        | Whether the last scrape of this BMC succeeded            | per-BMC scrape result (`finalize_report`); forced to 0 on resource failure, round failure or timeout |
| `redfish_scrape_duration_seconds`  | `bmc`                        | Duration of the last scrape of this BMC                  | round wall time (`finalize_report`) |
| `redfish_scrape_error`             | `bmc`, `resource`            | Set to 1 when the last scrape of a resource failed       | one series per failed resource (`registry.rs`); `resource="fast:timeout"`/`resource="slow:timeout"` for fast/slow-group deadline expiry, `resource="bmc"` for round failure (root-fetch errors whose message contains "timeout" are recorded as `resource="timeout"` — string-based discrimination, known backlog) |
| `redfish_scrape_errors_total`      | `bmc`                        | Total number of failed resources across all scrape rounds | cumulative counter in `Snapshot` (`registry.rs`), incremented per failed resource and once per failed/timed-out round |
| `redfish_build_info`               | `version`                    | Build information                                        | exporter crate version (`build_registry`, `registry.rs`), always `1.0` |
| `redfish_health_status`            | `bmc`, `resource_type`, `id`, `health`, `state` | Health and state of a resource      | `Status.Health` + `Status.State` of any collected resource; always `1.0` |
| `redfish_info`                     | `bmc`, `resource_type`, `id`, `key`, `value` | Resource-scoped static key-value information | inventory fields, always `1.0` |

`redfish_health_status` `resource_type` values: `system`, `chassis`, `manager`, `processor`, `memory`, `drive`, `volume`, `ethernet_interface`, `network_adapter`, `port`, `pcie_device`, `assembly`, `software_inventory`.

`redfish_info` keys: `manufacturer`, `model`, `processor_type`, `part_number`, `memory_type`, `serial_number`, `revision`, `sku`, `firmware_version`, `software_version`, `producer`, `mac_address`, `name`.

The generic health/info families use `@odata.id`, rather than a leaf `Id`, so resources such as `DIMM1` under different ComputerSystems cannot collapse into the same Prometheus series.

### Sensors (chassis)

| Metric                                      | Labels                                                        | Help text from code      | Source |
|---------------------------------------------|---------------------------------------------------------------|--------------------------|--------|
| `redfish_sensor_reading`                    | `bmc`, `chassis`, `id`, `name`, `units`, `sensor_type`, `health`, `state` | Sensor reading     | Chassis sensor links (`sensors.rs`), legacy `Thermal` temperatures, power supply sensors, `EnvironmentMetrics` sensors, `Controls` (`power.rs`) |
| `redfish_sensor_threshold_upper_critical`   | same as above                                                 | Sensor threshold         | `Thresholds.upper_critical` |
| `redfish_sensor_threshold_upper_warning`    | same as above                                                 | Sensor threshold         | `Thresholds.upper_caution` |
| `redfish_sensor_threshold_lower_warning`    | same as above                                                 | Sensor threshold         | `Thresholds.lower_caution` |
| `redfish_sensor_threshold_lower_critical`   | same as above                                                 | Sensor threshold         | `Thresholds.lower_critical` |

### Power

| Metric                                      | Labels           | Help text from code                                  | Source |
|---------------------------------------------|------------------|------------------------------------------------------|--------|
| `redfish_power_consumption_watts`           | `bmc`, `chassis` | Total chassis power consumption in watts             | legacy `PowerControl.PowerConsumedWatts` (first control) |
| `redfish_power_consumption_min_watts`       | `bmc`, `chassis` | Minimum chassis power consumption in watts over the measurement window | `PowerControl.PowerMetrics.MinConsumedWatts` |
| `redfish_power_consumption_max_watts`       | `bmc`, `chassis` | Maximum chassis power consumption in watts over the measurement window | `PowerControl.PowerMetrics.MaxConsumedWatts` |
| `redfish_power_consumption_avg_watts`       | `bmc`, `chassis` | Average chassis power consumption in watts over the measurement window | `PowerControl.PowerMetrics.AverageConsumedWatts` |
| `redfish_power_consumption_interval_minutes`| `bmc`, `chassis` | Power consumption measurement window in minutes      | `PowerControl.PowerMetrics.IntervalInMin` |
| `redfish_power_supply_efficiency_percent`   | `bmc`, `chassis`, `id` | Power supply efficiency in percent            | `PowerSupply.EfficiencyPercent` |
| `redfish_power_supply_input_watts`          | `bmc`, `chassis`, `id` | Power supply input power in watts             | `PowerSupply.PowerInputWatts` |
| `redfish_power_supply_capacity_watts`       | `bmc`, `chassis`, `id` | Power supply rated capacity in watts          | `PowerSupply.PowerCapacityWatts` |
| `redfish_power_supply_input_voltage`        | `bmc`, `chassis`, `id` | Power supply line input voltage in volts      | `PowerSupply.LineInputVoltage` |

Note: PSU metrics are read from the legacy Power document (embedded `PowerSupplies`, never fetched as independent resources — Dell exposes fragment URIs that would re-download the whole Power document, see AUDIT-9). PSUs additionally contribute `redfish_sensor_reading` series (see Sensors).

### Processors

| Metric                                | Labels                | Help text from code                              | Source |
|---------------------------------------|-----------------------|--------------------------------------------------|--------|
| `redfish_processor_temperature_celsius` | `bmc`, `system`, `id` | Processor temperature in Celsius                  | `ProcessorMetrics.temperature_celsius` |
| `redfish_processor_power_watts`       | `bmc`, `system`, `id` | Processor power in watts                          | `ProcessorMetrics.consumed_power_watt` |
| `redfish_processor_bandwidth_percent` | `bmc`, `system`, `id` | Processor bandwidth utilization percentage        | `ProcessorMetrics.bandwidth_percent` |
| `redfish_processor_frequency_mhz`     | `bmc`, `system`, `id` | Current operating frequency of the processor in MHz | `Oem.Dell.DellProcessor.CurrentClockSpeedMhz` (Dell) / `Oem.Public.FrequencyMHz` (Inspur); series emitted only when the vendor field is present |
| `redfish_processor_max_frequency_mhz` | `bmc`, `system`, `id` | Maximum rated frequency of the processor in MHz   | standard `MaxSpeedMHz` |
| `redfish_processor_voltage_volts`     | `bmc`, `system`, `id` | Processor input voltage in volts (vendor OEM field when present) | `Oem.Dell.DellProcessor.Volts` (Dell, string value parsed to float); series emitted only when the field is present and numeric |

### Memory

| Metric                            | Labels                | Help text from code                          | Source |
|-----------------------------------|-----------------------|----------------------------------------------|--------|
| `redfish_memory_capacity_bytes`   | `bmc`, `system`, `id` | Memory capacity in bytes                     | `Memory.capacity_mib` × 1,048,576 |
| `redfish_memory_bandwidth_percent`| `bmc`, `system`, `id` | Memory bandwidth utilization percentage      | `MemoryMetrics.bandwidth_percent` |
| `redfish_memory_correctable_errors`   | `bmc`, `system`, `id` | Memory correctable ECC alarm trip, 1 = tripped | `MemoryMetrics.HealthData.AlarmTrips.CorrectableECCError` |
| `redfish_memory_uncorrectable_errors` | `bmc`, `system`, `id` | Memory uncorrectable ECC alarm trip, 1 = tripped | `MemoryMetrics.HealthData.AlarmTrips.UncorrectableECCError` |

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
| `redfish_storage_controller_info`            | `bmc`, `system`, `storage`, `id`, `model`, `firmware_version` | Storage controller model and firmware information, 1 = present | `StorageControllers[]` `Model` / `FirmwareVersion` (standard field; Dell only on probed machines); series emitted only when `Model` is present |
| `redfish_storage_controller_status`          | `bmc`, `system`, `storage`, `id`, `status` | Storage controller status, 1 = present with status label | `StorageControllers[].Status.State` |
| `redfish_drive_info`                         | `bmc`, `system`, `storage`, `id`, `wwn` | Drive vendor identifier information, 1 = present | `Oem.Dell.DellPhysicalDisk.WWN` (Dell only); series emitted only when the field is present |
| `redfish_drive_oem_status`                   | `bmc`, `system`, `storage`, `id`, `raid_status`, `power_status` | Drive vendor OEM status, 1 = present with status labels | `Oem.Dell.DellPhysicalDisk.RaidStatus` / `PowerStatus` (Dell only); emitted when at least one of the two is present |

The `redfish_drive_io_*_errors_total` and `redfish_scrape_errors_total` families are registered as Prometheus Counters. Their absolute values are rebuilt from the latest BMC reading/cumulative exporter state, so they remain monotonic while the source BMC and exporter process do not reset.

### Network

| Metric                                 | Labels                | Help text from code                                      | Source |
|----------------------------------------|-----------------------|----------------------------------------------------------|--------|
| `redfish_ethernet_interface_link_status`| `bmc`, `system`, `id` | Ethernet link status, 1 = up                             | `EthernetInterface.link_status` (1 = LinkUp, 0 = otherwise) |
| `redfish_ethernet_interface_speed_mbps` | `bmc`, `system`, `id` | Ethernet link speed in Mbps                              | `EthernetInterface.speed_mbps` |
| `redfish_pcie_device_lanes_in_use`     | `bmc`, `chassis`, `id`| Number of PCIe lanes in use by the device                | `PcieInterface.lanes_in_use` |
| `redfish_pcie_device_max_lanes`        | `bmc`, `chassis`, `id`| Maximum number of PCIe lanes supported by the device     | `PcieInterface.max_lanes` |

### System

| Metric            | Labels          | Help text from code                | Source |
|-------------------|-----------------|------------------------------------|--------|
| `redfish_power_state` | `bmc`, `system` | Power state of a system, 1 = On and 0 = any other known state | `System.power_state`; absent only when the state is unavailable |
| `redfish_indicator_led` | `bmc`, `resource_type`, `id`, `state` | Indicator LED state, 1 = present with state label | `IndicatorLED` of Systems (`resource_type="system"`) and Chassis (`resource_type="chassis"`); series emitted only when the field is present |

### Event log

This collector is disabled by default. Set `collectors.event_logs: true` to enable it. `collectors.event_log_limit` defaults to 500 entries per BMC snapshot and is hard-limited to 5,000. Pagination stops successfully at the configured cap; a warning is logged instead of marking an otherwise healthy BMC down.

| Metric                    | Labels                                       | Help text from code                                            | Source |
|---------------------------|----------------------------------------------|----------------------------------------------------------------|--------|
| `redfish_event_log_entry` | `bmc`, `manager`, `service`, `severity`, `message`, `id` | Event log entry, value is the entry creation time as a Unix timestamp | `LogService.Entries` members (`logs.rs`), fetched with the pagination walker (`Members@odata.nextLink`); slow group |

`severity` is the `LogEntry.Severity` label mapping (`OK` / `Warning` / `Critical`, unknown variants exposed as `UnsupportedValue`; empty when absent, e.g. Inspur AuditLog); `message` is the `LogEntry.Message` text (empty when absent). Entries without a parseable `Created` timestamp are skipped. Series are stale-prone (log entries are not deleted by the exporter), so alerts should use `changes()`-style PromQL rather than `absent()`.

### BIOS

`redfish_bios_pending_changes` is always collected. Full attributes are disabled by default; set `collectors.bios_attributes: true` to enable them. `collectors.bios_attribute_limit` defaults to, and cannot exceed, 10,000 attributes per BMC snapshot. When a limit is reached, attributes are selected in deterministic name order to avoid series churn.

| Metric                            | Labels                     | Help text from code                   | Source |
|-----------------------------------|----------------------------|---------------------------------------|--------|
| `redfish_bios_attribute`          | `bmc`, `system`, `attribute` | BIOS attribute value               | `Bios.Attributes` numeric / boolean / `Enabled` / `Disabled` values (`bios.rs`); slow group |
| `redfish_bios_attribute_info`     | `bmc`, `system`, `attribute`, `value` | BIOS string attribute       | `Bios.Attributes` string values, always `1.0`; slow group |
| `redfish_bios_pending_changes`    | `bmc`, `system`            | BIOS settings pending reboot           | presence of `@Redfish.Settings` on the Bios resource (`bios.rs`), 1 = pending; slow group |

Null attribute values (including password fields such as Dell `SysPassword`/Inspur `AdministratorPassword`) are skipped and never exported. A system whose BIOS payload fails to parse fails the BIOS collector when no other declared BIOS resource succeeds. The IEIT/Inspur 166 KB payload was validated live for 0.1.0 (4,065 numeric/boolean and 1,690 string samples).

## B. Recorded but not implemented

The following areas were evaluated during 0.1.0 scoping and are deliberately **not collected**. Reasons and directions for future versions:

| Item | Reason | Direction |
|------|--------|-----------|
| EventService, TaskService, AccountService, SessionService metrics | Out of scope for 0.1.0 (read-only sensor/compliance focus); session usage is internal | dedicated collectors; EventService subscription-based eventing is a larger design |
| BootOptions, SecureBoot, HostInterfaces, ManagerNetworkProtocol | Out of scope; volatile/firmware-config rather than monitoring data | `redfish_info`-style versioning collectors |
| All OEM extension resources | Vendor-specific schemas, high maintenance cost | opt-in collectors behind config flags |
| `redfish_power_input_watts` | Constant defined in `src/metrics.rs` but no collector pushes it (chassis input power is not reported by the legacy Power schema in nv-redfish) | emit when the BMC distinguishes input vs consumed power |
| `redfish_processor_utilization_percent` | `ProcessorSummary` has no utilization field | collect per-core utilization / `OperatingConfig` once exposed by nv-redfish |
| `redfish_drive_utilization_percent` | `DriveMetrics` has no utilization field | SMART-based estimate or nv-redfish schema extension |
| PCIe link rate (GT/s) | `PcieDevice` has no link-speed field | nv-redfish schema extension (needs `PcieDevice.PcieDeviceProperties` extension data) |
