use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::collector::ScrapeReport;
use crate::metrics::{
    BUILD_INFO, Metric, MetricsError, SCRAPE_ERROR, SCRAPE_ERRORS_TOTAL, UP, register_into,
};

/// 指标快照：RwLock 内为每个 BMC 各存一个 Arc 指针，update 按 BMC 名
/// 插入/替换，读取不克隆底层 registry。errors 记录跨轮累计的失败资源数，
/// 每轮 build_registry 时把当前累计值烧进新 registry（registry 本身每轮重建，
/// 计数必须存在快照层）。
pub struct Snapshot {
    inner: RwLock<HashMap<String, Arc<prometheus::Registry>>>,
    errors: Mutex<HashMap<String, u64>>,
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
        }
    }

    /// 累计指定 BMC 的失败资源数（跨轮累加）。
    pub fn record_scrape_errors(&self, bmc: &str, count: u64) {
        *self
            .errors
            .lock()
            .expect("snapshot errors mutex poisoned")
            .entry(bmc.to_string())
            .or_insert(0) += count;
    }

    /// 指定 BMC 累计失败资源数；从未记录过时为 0。
    pub fn scrape_errors(&self, bmc: &str) -> u64 {
        self.errors
            .lock()
            .expect("snapshot errors mutex poisoned")
            .get(bmc)
            .copied()
            .unwrap_or(0)
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
/// 另注入 redfish_build_info{version}=1 与 redfish_scrape_errors_total{bmc}=error_total
/// （累计值来自快照层，registry 每轮重建后计数不丢失）。
pub async fn build_registry(
    bmc_name: &str,
    report: &ScrapeReport,
    error_total: u64,
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
    let build = Metric::gauge(BUILD_INFO.0, BUILD_INFO.1)
        .label("version", env!("CARGO_PKG_VERSION").to_string())
        .build(1.0);
    register_into(&[build], &registry)?;
    let errors = Metric::gauge(SCRAPE_ERRORS_TOTAL.0, SCRAPE_ERRORS_TOTAL.1)
        .label("bmc", bmc_name.to_string())
        .build(error_total as f64);
    register_into(&[errors], &registry)?;
    Ok(Arc::new(registry))
}
