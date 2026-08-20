use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bmc::{BmcHandle, build_http_client, establish_session, make_bmc};
use crate::collector::{ScrapeReport, collect_all};
use crate::config::{AuthMethod, Config};
use crate::metrics::{Metric, UP};
use crate::registry::{Snapshot, build_registry};

pub struct Scraper {
    bmcs: Vec<BmcHandle>,
    interval: Duration,
    timeout: Duration,
    snapshot: Arc<Snapshot>,
}

impl Scraper {
    /// 为每个 BMC 建立 HTTP client 与 handle；Session 认证时先建会话。
    pub async fn new(cfg: &Config, snapshot: Arc<Snapshot>) -> Result<Self, anyhow::Error> {
        let mut bmcs = Vec::with_capacity(cfg.bmcs.len());
        for bmc_cfg in &cfg.bmcs {
            let client = build_http_client(bmc_cfg)?;
            let handle = make_bmc(bmc_cfg, client);
            if bmc_cfg.auth == AuthMethod::Session {
                let token =
                    establish_session(&handle.bmc, &bmc_cfg.username, bmc_cfg.password.expose())
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "bmc '{}': session establishment failed: {e}",
                                bmc_cfg.name
                            )
                        })?;
                handle
                    .bmc
                    .set_credentials(nv_redfish::bmc_http::BmcCredentials::token(token));
            }
            bmcs.push(handle);
        }
        Ok(Scraper {
            bmcs,
            interval: cfg.scrape_interval,
            timeout: cfg.scrape_timeout,
            snapshot,
        })
    }

    /// 周期采集循环：每 tick 一轮，stop 信号在 tick 前触发则直接退出。
    pub fn run(self, mut stop: tokio::sync::watch::Receiver<bool>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = stop.changed() => break,
                    _ = interval.tick() => {}
                }
                self.scrape_once().await;
            }
        })
    }

    async fn scrape_once(&self) {
        let started = Instant::now();
        let bmc_count = self.bmcs.len();
        let mut set = tokio::task::JoinSet::new();
        for handle in &self.bmcs {
            let name = handle.name.clone();
            let bmc = Arc::clone(&handle.bmc);
            set.spawn(async move {
                let result = collect_all(bmc, &name).await;
                (name, result)
            });
        }
        let collect = async {
            while let Some(res) = set.join_next().await {
                match res {
                    Ok((name, Ok(report))) => match build_registry(&name, &report).await {
                        Ok(reg) => {
                            self.snapshot.update(reg);
                            info!(
                                bmc = %name,
                                failed = report.failed_resources.len(),
                                "scrape complete"
                            );
                        }
                        Err(e) => {
                            warn!(bmc = %name, error = %e, "failed to build registry");
                        }
                    },
                    Ok((name, Err(e))) => {
                        let report = ScrapeReport {
                            metrics: vec![
                                Metric::gauge(UP.0, UP.1)
                                    .label("bmc", name.clone())
                                    .build(0.0),
                            ],
                            failed_resources: vec!["bmc".to_string()],
                        };
                        match build_registry(&name, &report).await {
                            Ok(reg) => self.snapshot.update(reg),
                            Err(e) => {
                                warn!(bmc = %name, error = %e, "failed to build registry");
                            }
                        }
                        warn!(bmc = %name, error = %e, "scrape failed");
                    }
                    Err(e) => {
                        warn!(error = %e, "scrape task panicked");
                    }
                }
            }
        };
        match tokio::time::timeout(self.timeout, collect).await {
            Ok(()) => {}
            Err(_) => {
                warn!("scrape round timed out after {:?}", self.timeout);
                set.shutdown().await;
            }
        }
        info!(
            bmc_count,
            duration_ms = started.elapsed().as_millis(),
            "scrape round complete"
        );
    }
}
