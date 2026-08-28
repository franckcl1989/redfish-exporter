use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use bytes::Bytes;

use crate::collector::ScrapeReport;
use crate::metrics::{
    BUILD_INFO, Metric, MetricsError, SCRAPE_ERROR, SCRAPE_ERRORS_TOTAL, UP,
    encode_metric_families_into, register_into,
};
use crate::recover_lock;

/// 单 BMC registry 条目。HTTP exposition 在 Snapshot 层合并后统一预编码，避免
/// 多 registry 文本直接拼接产生重复 HELP/TYPE 与重复 build_info 样本。
pub struct RegistryEntry {
    pub registry: Arc<prometheus::Registry>,
}

/// 指标快照：RwLock 内为每个 BMC 各存一个 Arc 指针，update 按 BMC 名
/// 插入/替换，读取不克隆底层 registry。errors 记录跨轮累计的失败资源数，
/// 每轮 build_registry 时把当前累计值烧进新 registry（registry 本身每轮重建，
/// 计数必须存在快照层）。
pub struct Snapshot {
    inner: RwLock<HashMap<String, Arc<RegistryEntry>>>,
    errors: Mutex<HashMap<String, u64>>,
    /// 全部 BMC 合并后的合法 Prometheus exposition；Bytes clone 为 O(1)，HTTP
    /// 热路径不重新编码也不复制整个响应体。
    encoded: RwLock<Option<Bytes>>,
    /// 合并编码复用缓冲：跨轮增长一次后不再反复扩容。
    scratch: Mutex<Vec<u8>>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    pub fn new() -> Self {
        Snapshot {
            inner: RwLock::new(HashMap::new()),
            errors: Mutex::new(HashMap::new()),
            encoded: RwLock::new(None),
            scratch: Mutex::new(Vec::new()),
        }
    }

    /// 累计指定 BMC 的失败资源数（跨轮累加）。
    pub fn record_scrape_errors(&self, bmc: &str, count: u64) {
        let mut errors = recover_lock(self.errors.lock());
        let current = errors.entry(bmc.to_string()).or_insert(0);
        *current = current.saturating_add(count);
    }

    /// 指定 BMC 累计失败资源数；从未记录过时为 0。
    pub fn scrape_errors(&self, bmc: &str) -> u64 {
        recover_lock(self.errors.lock())
            .get(bmc)
            .copied()
            .unwrap_or(0)
    }

    /// 插入或替换指定 BMC 的快照，并把全部 registry 合并成一份 exposition。
    /// 同名指标族的样本被合并后只编码一组 HELP/TYPE；全局 build_info 只保留一份。
    /// inner 写锁覆盖合并过程，从而并发 update 不会以旧结果覆盖新结果。
    pub fn update(&self, bmc_name: &str, registry: Arc<prometheus::Registry>) {
        let mut inner = recover_lock(self.inner.write());
        inner.insert(bmc_name.to_string(), Arc::new(RegistryEntry { registry }));

        let mut names: Vec<_> = inner.keys().cloned().collect();
        names.sort_unstable();
        let mut families: BTreeMap<String, prometheus::proto::MetricFamily> = BTreeMap::new();
        for name in names {
            for mut family in inner[&name].registry.gather() {
                let family_name = family.name().to_string();
                match families.entry(family_name) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(family);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let existing = entry.get_mut();
                        // build_info is exporter-global and identical in every per-BMC registry.
                        if existing.name() == BUILD_INFO.0 {
                            continue;
                        }
                        assert_eq!(
                            existing.help(),
                            family.help(),
                            "metric help changed across BMCs"
                        );
                        assert_eq!(
                            existing.type_(),
                            family.type_(),
                            "metric type changed across BMCs"
                        );
                        existing.mut_metric().append(family.mut_metric());
                    }
                }
            }
        }

        let families: Vec<_> = families.into_values().collect();
        let bytes = {
            let mut scratch = recover_lock(self.scratch.lock());
            scratch.clear();
            encode_metric_families_into(&families, &mut scratch);
            Bytes::copy_from_slice(&scratch)
        };
        *recover_lock(self.encoded.write()) = Some(bytes);
    }

    /// 返回全部 BMC 的统一预编码 exposition。Bytes clone 仅增加引用计数。
    pub fn encoded(&self) -> Option<Bytes> {
        recover_lock(self.encoded.read()).clone()
    }

    /// 读取指定 BMC 的快照；该 BMC 从未采集过时为 None。
    pub fn registry(&self, bmc_name: &str) -> Option<Arc<prometheus::Registry>> {
        recover_lock(self.inner.read())
            .get(bmc_name)
            .map(|e| Arc::clone(&e.registry))
    }

    /// 没有任何 BMC 的快照时返回 true（/metrics 据此返回 no-data-yet）。
    pub fn is_empty(&self) -> bool {
        recover_lock(self.inner.read()).is_empty()
    }

    /// 已发布过至少一轮快照的 BMC 数量。
    pub fn registry_count(&self) -> usize {
        recover_lock(self.inner.read()).len()
    }

    /// 所有 BMC 的 (name, entry) 快照对，按 BMC 名排序保证输出确定性。
    pub fn registries(&self) -> Vec<(String, Arc<RegistryEntry>)> {
        let mut pairs: Vec<_> = recover_lock(self.inner.read())
            .iter()
            .map(|(name, entry)| (name.clone(), Arc::clone(entry)))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }
}

/// 把一次采集的 ScrapeReport 构建成 prometheus Registry 并返回 Arc。
/// 有失败资源时补 redfish_up=0（finalize_report 已产出时值一致，合并无冲突），
/// 每个失败资源追加 redfish_scrape_error{bmc,resource}=1。
/// 另注入 redfish_build_info{version}=1 与 redfish_scrape_errors_total{bmc}=error_total
/// （累计值来自快照层，registry 每轮重建后计数不丢失）。
pub async fn build_registry(
    bmc_name: &str,
    report: &ScrapeReport,
    error_total: u64,
) -> Result<Arc<prometheus::Registry>, MetricsError> {
    let registry = prometheus::Registry::new();
    let mut metrics = report.metrics.clone();
    let reported_up = metrics
        .iter()
        .find(|metric| metric.name == UP.0)
        .map_or(1.0, |metric| metric.value);
    metrics.retain(|metric| metric.name != UP.0);
    let up = Metric::gauge(UP.0, UP.1)
        .label("bmc", bmc_name.to_string())
        .build(if report.failed_resources.is_empty() {
            reported_up
        } else {
            0.0
        });
    metrics.push(up);
    for resource in &report.failed_resources {
        let error = Metric::gauge(SCRAPE_ERROR.0, SCRAPE_ERROR.1)
            .label("bmc", bmc_name.to_string())
            .label("resource", resource.clone())
            .build(1.0);
        metrics.push(error);
    }
    let build = Metric::gauge(BUILD_INFO.0, BUILD_INFO.1)
        .label("version", env!("CARGO_PKG_VERSION").to_string())
        .build(1.0);
    metrics.push(build);
    let errors = Metric::counter(SCRAPE_ERRORS_TOTAL.0, SCRAPE_ERRORS_TOTAL.1)
        .label("bmc", bmc_name.to_string())
        .build(error_total as f64);
    metrics.push(errors);
    register_into(&metrics, &registry)?;
    Ok(Arc::new(registry))
}
