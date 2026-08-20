use std::sync::{Arc, RwLock};

use crate::collector::ScrapeReport;
use crate::metrics::{Metric, MetricsError, SCRAPE_ERROR, UP, register_into};

/// 指标快照：RwLock 内仅存 Arc 指针，update 是原子替换，读取不克隆底层 registry。
pub struct Snapshot {
    inner: RwLock<Option<Arc<prometheus::Registry>>>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    pub fn new() -> Self {
        Snapshot {
            inner: RwLock::new(None),
        }
    }

    /// 原子替换快照内容。
    pub fn update(&self, registry: Arc<prometheus::Registry>) {
        *self.inner.write().expect("snapshot rwlock poisoned") = Some(registry);
    }

    /// 读取当前快照；从未采集过时为 None。
    pub fn registry(&self) -> Option<Arc<prometheus::Registry>> {
        self.inner.read().expect("snapshot rwlock poisoned").clone()
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
