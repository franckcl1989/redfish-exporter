use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::collector::ScrapeReport;
use crate::metrics::{
    BUILD_INFO, Metric, MetricsError, SCRAPE_ERROR, SCRAPE_ERRORS_TOTAL, UP, encode_bytes_into,
    register_into,
};
use crate::recover_lock;

/// 快照条目：registry 与发布时编码一次的文本（/metrics 直接拼接，零现场编码）。
pub struct RegistryEntry {
    pub registry: Arc<prometheus::Registry>,
    pub encoded: Vec<u8>,
}

/// 指标快照：RwLock 内为每个 BMC 各存一个 Arc 指针，update 按 BMC 名
/// 插入/替换，读取不克隆底层 registry。errors 记录跨轮累计的失败资源数，
/// 每轮 build_registry 时把当前累计值烧进新 registry（registry 本身每轮重建，
/// 计数必须存在快照层）。
pub struct Snapshot {
    inner: RwLock<HashMap<String, Arc<RegistryEntry>>>,
    errors: Mutex<HashMap<String, u64>>,
    /// 编码复用缓冲：跨轮增长一次后不再反复扩容（每轮 encode 写入后克隆入条目）。
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
            scratch: Mutex::new(Vec::new()),
        }
    }

    /// 累计指定 BMC 的失败资源数（跨轮累加）。
    pub fn record_scrape_errors(&self, bmc: &str, count: u64) {
        *recover_lock(self.errors.lock())
            .entry(bmc.to_string())
            .or_insert(0) += count;
    }

    /// 指定 BMC 累计失败资源数；从未记录过时为 0。
    pub fn scrape_errors(&self, bmc: &str) -> u64 {
        recover_lock(self.errors.lock())
            .get(bmc)
            .copied()
            .unwrap_or(0)
    }

    /// 插入或替换指定 BMC 的快照（原子，按 BMC 隔离）。发布时编码一次存入条目，
    /// /metrics 直接拼接预编码字节（编码成本移出请求路径）。
    pub fn update(&self, bmc_name: &str, registry: Arc<prometheus::Registry>) {
        // 顺序说明：先在 scratch 锁内 encode + clone，释放 scratch 锁后再取 inner 写锁
        // 插入——写锁只覆盖 insert 本身，发布路径的序列化面最小（scratch 为复用缓冲，
        // 其锁只在编码期间被持有，不与写锁重叠）。
        let encoded = {
            let mut scratch = recover_lock(self.scratch.lock());
            scratch.clear();
            encode_bytes_into(&registry, &mut scratch);
            scratch.clone()
        };
        let entry = Arc::new(RegistryEntry { registry, encoded });
        recover_lock(self.inner.write()).insert(bmc_name.to_string(), entry);
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
