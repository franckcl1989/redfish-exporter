#![forbid(unsafe_code)]

pub mod auth;
pub mod bmc;
pub mod collector;
pub mod config;
pub mod http;
pub mod metrics;
pub mod pagination;
pub mod registry;
pub mod scraper;

/// 恢复被 panic 污染的锁守卫：中毒说明曾有任务在持锁时 panic，
/// 恢复后继续服务（避免连锁 panic），同时记录 error 日志留痕。
pub(crate) fn recover_lock<T>(r: std::sync::LockResult<T>) -> T {
    r.unwrap_or_else(|p| {
        tracing::error!("recovered from a poisoned lock (a task panicked while holding it)");
        p.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::recover_lock;

    #[test]
    fn recover_lock_returns_guard_after_poisoning() {
        let m = Arc::new(Mutex::new(0u32));
        let holder = m.lock().unwrap();
        let m2 = Arc::clone(&m);
        // 持锁线程 panic → mutex 中毒（线程先阻塞在 lock 上，主线程 drop(holder) 后取得守卫再 panic）
        let t = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison while holding the guard");
        });
        drop(holder);
        let _ = t.join();
        // 锁已中毒：lock() 返回 Err(PoisonError)
        let r = m.lock();
        assert!(r.is_err());
        // recover_lock 恢复出守卫，值完整
        let recovered = recover_lock(r);
        assert_eq!(*recovered, 0);
    }
}
