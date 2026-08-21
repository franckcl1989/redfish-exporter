use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bmc::{BmcHandle, build_http_client, establish_session, is_unauthorized, make_bmc};
use crate::collector::{ScrapeReport, collect_all};
use crate::config::{AuthMethod, Config};
use crate::metrics::{Metric, UP};
use crate::registry::{Snapshot, build_registry};

/// 在 deadline 内运行 future；超时返回 None。
pub async fn with_deadline<T>(timeout: Duration, fut: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(timeout, fut).await.ok()
}

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
            let client = build_http_client(bmc_cfg, cfg.request_timeout)?;
            let handle = make_bmc(bmc_cfg, client);
            if bmc_cfg.auth == AuthMethod::Session {
                match establish_session(&handle.bmc, &bmc_cfg.username, bmc_cfg.password.expose())
                    .await
                {
                    Ok(token) => {
                        handle
                            .bmc
                            .set_credentials(nv_redfish::bmc_http::BmcCredentials::token(token));
                        handle.session_established.store(true, Ordering::SeqCst);
                    }
                    Err(e) => {
                        warn!(
                            bmc = %bmc_cfg.name,
                            error = %e,
                            "session establishment failed, falling back to basic auth"
                        );
                    }
                }
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
        let timeout = self.timeout;
        let mut set = tokio::task::JoinSet::new();
        for handle in &self.bmcs {
            let name = handle.name.clone();
            let bmc = Arc::clone(&handle.bmc);
            let username = handle.username.clone();
            let password = handle.password.clone();
            let auth = handle.auth;
            let session_established = Arc::clone(&handle.session_established);
            set.spawn(async move {
                let result = match with_deadline(timeout, async {
                    if auth == AuthMethod::Session
                        && !session_established.load(Ordering::SeqCst)
                    {
                        if let Ok(token) =
                            establish_session(&bmc, &username, password.expose()).await
                        {
                            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(
                                token,
                            ));
                            session_established.store(true, Ordering::SeqCst);
                            info!(bmc = %name, "session established on first round");
                        } else {
                            warn!(
                                bmc = %name,
                                "session establishment failed, continuing with basic auth"
                            );
                        }
                    }
                    match collect_all(Arc::clone(&bmc), &name).await {
                        Ok(report) => Ok(report),
                        Err(err) if auth == AuthMethod::Session && is_unauthorized(&err) => {
                            info!(bmc = %name, "session rejected with 401, re-establishing session");
                            match establish_session(&bmc, &username, password.expose()).await {
                                Ok(token) => {
                                    bmc.set_credentials(
                                        nv_redfish::bmc_http::BmcCredentials::token(token),
                                    );
                                    info!(bmc = %name, "session re-established, retrying scrape");
                                    collect_all(Arc::clone(&bmc), &name).await
                                }
                                Err(e) => {
                                    warn!(
                                        bmc = %name,
                                        error = %e,
                                        "session re-establishment failed"
                                    );
                                    Err(err)
                                }
                            }
                        }
                        Err(err) => Err(err),
                    }
                })
                .await
                {
                    Some(r) => r.map_err(|e| e.to_string()),
                    None => Err("timeout".to_string()),
                };
                (name, result)
            });
        }
        while let Some(res) = set.join_next().await {
            match res {
                Ok((name, Ok(report))) => match build_registry(&name, &report).await {
                    Ok(reg) => {
                        self.snapshot.update(&name, reg);
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
                    let timed_out = e.contains("timeout");
                    let report = ScrapeReport {
                        metrics: vec![
                            Metric::gauge(UP.0, UP.1)
                                .label("bmc", name.clone())
                                .build(0.0),
                        ],
                        failed_resources: if timed_out {
                            vec!["timeout".to_string()]
                        } else {
                            vec!["bmc".to_string()]
                        },
                    };
                    match build_registry(&name, &report).await {
                        Ok(reg) => self.snapshot.update(&name, reg),
                        Err(e) => {
                            warn!(bmc = %name, error = %e, "failed to build registry");
                        }
                    }
                    if timed_out {
                        warn!(bmc = %name, "scrape timed out after {:?}", self.timeout);
                    } else {
                        warn!(bmc = %name, error = %e, "scrape failed");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "scrape task panicked");
                }
            }
        }
        info!(
            bmc_count,
            duration_ms = started.elapsed().as_millis(),
            "scrape round complete"
        );
    }
}
