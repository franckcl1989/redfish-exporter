use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::collector::ScrapeReport;
use crate::metrics::{Metric, MetricsError, SCRAPE_ERROR, UP, register_into};

/// 指标快照：RwLock 内为每个 BMC 各存一个 Arc 指针，update 按 BMC 名
/// 插入/替换，读取不克隆底层 registry。
pub struct Snapshot {
    inner: RwLock<HashMap<String, Arc<prometheus::Registry>>>,
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
        }
    }

    /// 插入或替换指定 BMC 的快照 registry（原子，按 BMC 隔离）。
    pub fn update(&self, bmc_name: &str, registry: Arc<prometheus::Registry>) {
        self.inner
            .write()
            .expect("snapshot rwlock poisoned")
            .insert(bmc_name.to_string(), registry);
    }

    /// 读取指定 BMC 的快照；该 BMC 从未采集过时为 None。
    pub fn registry(&self, bmc_name: &str) -> Option<Arc<prometheus::Registry>> {
        self.inner
            .read()
            .expect("snapshot rwlock poisoned")
            .get(bmc_name)
            .cloned()
    }

    /// 没有任何 BMC 的快照时返回 true（/metrics 据此返回 no-data-yet）。
    pub fn is_empty(&self) -> bool {
        self.inner
            .read()
            .expect("snapshot rwlock poisoned")
            .is_empty()
    }

    /// 所有 BMC 的 (name, registry) 快照对，按 BMC 名排序保证输出确定性。
    pub fn registries(&self) -> Vec<(String, Arc<prometheus::Registry>)> {
        let mut pairs: Vec<_> = self
            .inner
            .read()
            .expect("snapshot rwlock poisoned")
            .iter()
            .map(|(name, reg)| (name.clone(), Arc::clone(reg)))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }
}

/// 把一次采集的 ScrapeReport 构建成 prometheus Registry 并返回 Arc。
/// 有失败资源时补 redfish_up=0（collect_all 已产出时值一致，合并无冲突），
/// 每个失败资源追加 redfish_scrape_error{bmc,resource}=1。
pub async fn build_registry(
    bmc_name: &str,
    report: &ScrapeReport,
) -> Result<Arc<prometheus::Registry>, MetricsError> {
    let registry = prometheus::Registry::new();
    register_into(&report.metrics, &registry)?;
    if !report.failed_resources.is_empty() {
        let up = Metric::gauge(UP.0, UP.1)
            .label("bmc", bmc_name.to_string())
            .build(0.0);
        register_into(&[up], &registry)?;
    }
    for resource in &report.failed_resources {
        let error = Metric::gauge(SCRAPE_ERROR.0, SCRAPE_ERROR.1)
            .label("bmc", bmc_name.to_string())
            .label("resource", resource.clone())
            .build(1.0);
        register_into(&[error], &registry)?;
    }
    Ok(Arc::new(registry))
}
