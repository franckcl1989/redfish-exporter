use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::bmc::{BmcHandle, build_http_client, establish_session, is_unauthorized, make_bmc};
use crate::collector::error::{ConcreteBmc, is_not_found};
use crate::collector::{ScrapeReport, collect_fast, collect_slow, finalize_report, merge_reports};
use crate::config::{AuthMethod, Config, StabilityConfig};
use crate::metrics::{Metric, SCRAPE_ERROR, UP};
use crate::recover_lock;
use crate::registry::{Snapshot, build_registry};
use crate::stability::{BmcState, RoundAction, RoundOutcome, on_round_result, round_action};

/// 在 deadline 内运行 future；超时返回 None。
pub async fn with_deadline<T>(timeout: Duration, fut: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(timeout, fut).await.ok()
}

type ConcreteSession = nv_redfish::session_service::Session<ConcreteBmc>;
/// 每 BMC 慢组最近采集时间与结果缓存（跨轮复用）。
type SlowState = Arc<Mutex<HashMap<String, (Instant, Option<ScrapeReport>)>>>;
/// 已建立会话（仅 session 认证 BMC），shutdown 时删除。
type SessionStore = Arc<Mutex<HashMap<String, Option<Arc<ConcreteSession>>>>>;

pub struct Scraper {
    bmcs: Vec<BmcHandle>,
    interval: Duration,
    timeout: Duration,
    slow_interval: Option<Duration>,
    slow_state: SlowState,
    sessions: SessionStore,
    snapshot: Arc<Snapshot>,
    /// 自适应调度配置（冷却阈值与退避上下界）。
    stability: StabilityConfig,
    /// per-BMC 稳定性状态（Healthy/SessionDegraded/Cooling），跨轮累计。
    states: HashMap<String, BmcState>,
}

impl Scraper {
    /// 为每个 BMC 建立 HTTP client 与 handle；Session 认证时先建会话。
    pub async fn new(cfg: &Config, snapshot: Arc<Snapshot>) -> Result<Self, anyhow::Error> {
        let sessions: SessionStore = Arc::new(Mutex::new(HashMap::new()));
        let mut bmcs = Vec::with_capacity(cfg.bmcs.len());
        for bmc_cfg in &cfg.bmcs {
            let client = build_http_client(bmc_cfg, cfg.request_timeout)?;
            let handle = make_bmc(bmc_cfg, client);
            if bmc_cfg.auth == AuthMethod::Session {
                match establish_session(&handle.bmc, &bmc_cfg.username, bmc_cfg.password.expose())
                    .await
                {
                    Ok(est) => {
                        handle
                            .bmc
                            .set_credentials(nv_redfish::bmc_http::BmcCredentials::token(
                                est.token,
                            ));
                        handle.session_established.store(true, Ordering::SeqCst);
                        recover_lock(sessions.lock()).insert(bmc_cfg.name.clone(), est.session);
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
            slow_interval: cfg.slow_interval,
            slow_state: Arc::new(Mutex::new(HashMap::new())),
            sessions,
            snapshot,
            stability: cfg.stability.clone(),
            states: HashMap::new(),
        })
    }

    /// 周期采集循环：每 tick 一轮，stop 信号在 tick 前触发则直接退出。
    pub fn run(mut self, mut stop: tokio::sync::watch::Receiver<bool>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(self.interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        // graceful shutdown：删除已建立会话，避免 BMC 侧会话泄漏。
                        // 先取出全部会话再 await，避免持锁跨 await。
                        let to_delete = recover_lock(self.sessions.lock())
                            .iter()
                            .filter_map(|(n, s)| s.as_ref().map(|s| (n.clone(), Arc::clone(s))))
                            .collect::<Vec<_>>();
                        for (name, s) in to_delete {
                            if let Err(e) = s.delete().await {
                                warn!(bmc = %name, error = %e, "session cleanup failed");
                            }
                        }
                        break;
                    }
                    _ = interval.tick() => {}
                }
                self.scrape_once().await;
            }
        })
    }

    async fn scrape_once(&mut self) {
        let started = Instant::now();
        let bmc_count = self.bmcs.len();
        let timeout = self.timeout;
        let slow_interval = self.slow_interval;
        let stability = self.stability.clone();
        let slow_state = Arc::clone(&self.slow_state);
        let sessions = Arc::clone(&self.sessions);
        let now = Instant::now();
        let mut set = tokio::task::JoinSet::new();
        let mut cooldowns = Vec::new();
        for handle in &self.bmcs {
            let state = self
                .states
                .entry(handle.name.clone())
                .or_insert(BmcState::Healthy { failures: 0 });
            let action = round_action(state, now);
            if matches!(action, RoundAction::CooldownWait) {
                cooldowns.push(handle.name.clone());
                continue;
            }
            let (attempt_session, use_basic_only) = match (action, &*state) {
                (RoundAction::SessionRetry, _) => (true, false),
                (RoundAction::Scrape, BmcState::SessionDegraded { .. }) => (false, true),
                (RoundAction::Scrape, _) => (true, false),
                (RoundAction::CooldownWait, _) => unreachable!(),
            };
            let handle = handle.clone();
            let slow_state = Arc::clone(&slow_state);
            let sessions = Arc::clone(&sessions);
            set.spawn(async move {
                let outcome = run_bmc_round(
                    &handle,
                    slow_interval,
                    &slow_state,
                    &sessions,
                    timeout,
                    attempt_session,
                    use_basic_only,
                )
                .await;
                (handle.name.clone(), outcome)
            });
        }
        for name in &cooldowns {
            // 冷却等待轮：不发请求，直接发布 up=0 + resource=cooldown，不累计 errors_total。
            match build_registry(name, &cooldown_report(), self.snapshot.scrape_errors(name)).await
            {
                Ok(reg) => {
                    self.snapshot.update(name, reg);
                    info!(bmc = %name, "bmc in cooldown, round skipped");
                }
                Err(e) => warn!(bmc = %name, error = %e, "failed to build cooldown registry"),
            }
        }
        while let Some(res) = set.join_next().await {
            match res {
                Ok((name, outcome)) => {
                    let state = self
                        .states
                        .entry(name.clone())
                        .or_insert(BmcState::Healthy { failures: 0 });
                    let failed = outcome
                        .report
                        .as_ref()
                        .map(|r| !r.failed_resources.is_empty())
                        .unwrap_or(true);
                    *state = on_round_result(state, &outcome, failed, &stability, Instant::now());
                    match outcome.report {
                        Some(report) => {
                            // 会话降级标记（ruling Q2）：推进后仍处于 SessionDegraded 的轮即追加
                            // redfish_scrape_error{resource="session-degraded"}=1（spec §4.3「持续提示」）——
                            // 成功轮（up=1）与部分失败轮（up=0 + 具体失败资源）均带标记；
                            // 标记以指标形式追加，不触碰 failed_resources，不影响 errors_total 累计判定。
                            let degraded_mark = matches!(state, BmcState::SessionDegraded { .. });
                            let report = if degraded_mark {
                                with_degraded_mark(report, &name)
                            } else {
                                report
                            };
                            let n = report.failed_resources.len() as u64;
                            self.snapshot.record_scrape_errors(&name, n);
                            match build_registry(&name, &report, self.snapshot.scrape_errors(&name))
                                .await
                            {
                                Ok(reg) => {
                                    self.snapshot.update(&name, reg);
                                    info!(
                                        bmc = %name,
                                        failed = report.failed_resources.len(),
                                        state = ?state,
                                        "scrape complete"
                                    );
                                }
                                Err(e) => {
                                    warn!(bmc = %name, error = %e, "failed to build registry")
                                }
                            }
                        }
                        None => {
                            let timed_out = outcome.message.contains("timeout");
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
                            self.snapshot.record_scrape_errors(&name, 1);
                            match build_registry(&name, &report, self.snapshot.scrape_errors(&name))
                                .await
                            {
                                Ok(reg) => self.snapshot.update(&name, reg),
                                Err(e) => {
                                    warn!(bmc = %name, error = %e, "failed to build registry")
                                }
                            }
                            if timed_out {
                                warn!(bmc = %name, state = ?state, "scrape timed out after {:?}", timeout);
                            } else {
                                warn!(bmc = %name, error = %outcome.message, state = ?state, "scrape failed");
                            }
                        }
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

/// 冷却等待轮的发布报告：up=0 + redfish_scrape_error{resource="cooldown"}（经 build_registry 生成），
/// 不累计 scrape_errors_total。
pub fn cooldown_report() -> ScrapeReport {
    ScrapeReport {
        metrics: vec![],
        failed_resources: vec!["cooldown".to_string()],
    }
}

/// 会话降级标记：basic 兜底成功轮追加 redfish_scrape_error{resource="session-degraded"}=1
/// （保持 up=1——failed_resources 为空不会触发 build_registry 的 up=0 覆盖）。
pub fn with_degraded_mark(mut report: ScrapeReport, bmc_name: &str) -> ScrapeReport {
    report.metrics.push(
        Metric::gauge(SCRAPE_ERROR.0, SCRAPE_ERROR.1)
            .label("bmc", bmc_name.to_string())
            .label("resource", "session-degraded".to_string())
            .build(1.0),
    );
    report
}

/// 慢组是否到点：无 slow_interval → 每轮都采（= 0.1.0 全量行为）；
/// 有 interval 且无上次时间（首轮）→ 到点即采；否则距上次采集已满 interval 才到点。
pub fn slow_due(last: Option<Instant>, interval: Option<Duration>) -> bool {
    match interval {
        None => true,
        Some(interval) => last.map(|t| t.elapsed() >= interval).unwrap_or(true),
    }
}

/// 应用一轮慢采结果：成功 → 返回带新时间戳的缓存项；失败 → 保留原缓存
/// （last-good 不清空），失败资源由调用方并入本轮 failed_resources。
pub fn apply_slow_result(
    cache: Option<(Instant, Option<ScrapeReport>)>,
    slow_result: Result<ScrapeReport, String>,
    now: Instant,
) -> (Option<(Instant, Option<ScrapeReport>)>, Vec<String>) {
    match slow_result {
        Ok(report) => (Some((now, Some(report))), Vec::new()),
        Err(resource) => (cache, vec![resource]),
    }
}

/// 合并快/慢组结果，并把慢组失败资源并入 failed_resources（去重）。
pub fn merge_round(
    fast: ScrapeReport,
    slow: Option<&ScrapeReport>,
    slow_failed: Vec<String>,
) -> ScrapeReport {
    let mut merged = merge_reports(fast, slow);
    for r in slow_failed {
        if !merged.failed_resources.contains(&r) {
            merged.failed_resources.push(r);
        }
    }
    merged
}

/// 单 BMC 单轮执行（含会话建立/401 重登），提取为 pub 接缝供真实 HTTP 故障注入测试（Task 5）。
/// attempt_session：本轮是否尝试 session 建立/重登；use_basic_only：Degraded 的 basic 兜底轮（先切 basic 凭据）。
pub async fn run_bmc_round(
    handle: &BmcHandle,
    slow_interval: Option<Duration>,
    slow_state: &SlowState,
    sessions: &SessionStore,
    timeout: Duration,
    attempt_session: bool,
    use_basic_only: bool,
) -> RoundOutcome {
    let name = handle.name.clone();
    let bmc = Arc::clone(&handle.bmc);
    let username = handle.username.clone();
    let password = handle.password.clone();
    let auth = handle.auth;
    let session_established = Arc::clone(&handle.session_established);

    if use_basic_only {
        bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::username_password(
            username.clone(),
            Some(password.expose().to_string()),
        ));
    }
    // 所有 session 建立失败轮（不止 401 重登轮）都记入 session_recovery_failed（re-ruling）：
    // session 坏死的 BMC（如 Inspur 建立永不成功）的失败轮（401 重登失败路径）据此经
    // Healthy 失败分支路由进 SessionDegraded——此后 session 建立按退避节奏重试，而非每轮
    // 反复打 session 端点（spec §4.1）；SessionRetry 轮重挂失败的兜底成功轮据此留在
    // SessionDegraded（不误升 Healthy）；basic 采集成功的轮走 ok 分支优先，保持 Healthy、
    // up=1 可达、不进降级。
    let mut session_recovery_failed = false;
    if auth == AuthMethod::Session && attempt_session && !session_established.load(Ordering::SeqCst)
    {
        if let Ok(est) = establish_session(&bmc, &username, password.expose()).await {
            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(est.token));
            session_established.store(true, Ordering::SeqCst);
            recover_lock(sessions.lock()).insert(name.clone(), est.session);
            info!(bmc = %name, "session established on first round");
        } else {
            warn!(bmc = %name, "session establishment failed, continuing with basic auth");
            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::username_password(
                username.clone(),
                Some(password.expose().to_string()),
            ));
            session_recovery_failed = true;
        }
    }
    match collect_round(
        Arc::clone(&bmc),
        &name,
        slow_interval,
        Arc::clone(slow_state),
        timeout,
    )
    .await
    {
        Ok(report) => RoundOutcome {
            report: Some(report),
            message: String::new(),
            session_recovery_failed,
            attempted_session: attempt_session,
        },
        Err(err) if auth == AuthMethod::Session && attempt_session && is_unauthorized(&err) => {
            info!(bmc = %name, "session rejected with 401, re-establishing session");
            // 重登前切回 basic 凭据：establish_session 的 ServiceRoot/SessionService
            // 请求若仍带已失效的 X-Auth-Token 会继续 401，导致无法恢复。
            bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::username_password(
                username.clone(),
                Some(password.expose().to_string()),
            ));
            match establish_session(&bmc, &username, password.expose()).await {
                Ok(est) => {
                    bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(est.token));
                    // 旧会话已因 401 过期（token 失效），无需 delete 清理；
                    // 仅更新存储的会话句柄供 shutdown 删除。
                    recover_lock(sessions.lock()).insert(name.clone(), est.session);
                    info!(bmc = %name, "session re-established, retrying scrape");
                    match collect_round(
                        Arc::clone(&bmc),
                        &name,
                        slow_interval,
                        Arc::clone(slow_state),
                        timeout,
                    )
                    .await
                    {
                        Ok(report) => RoundOutcome {
                            report: Some(report),
                            message: String::new(),
                            session_recovery_failed: false,
                            attempted_session: true,
                        },
                        Err(e) => RoundOutcome {
                            report: None,
                            message: e.to_string(),
                            session_recovery_failed: false,
                            attempted_session: true,
                        },
                    }
                }
                Err(e) => {
                    warn!(bmc = %name, error = %e, "session re-establishment failed");
                    // 会话已失效：复位建立标记（ruling Q1），使后续 SessionRetry 轮重新走
                    // 轮首建立流程；否则重挂永不重试且 basic 成功轮会误升 Healthy。
                    session_established.store(false, Ordering::SeqCst);
                    RoundOutcome {
                        report: None,
                        message: err.to_string(),
                        session_recovery_failed: true,
                        attempted_session: true,
                    }
                }
            }
        }
        Err(err) => RoundOutcome {
            report: None,
            message: err.to_string(),
            session_recovery_failed: false,
            attempted_session: attempt_session,
        },
    }
}

/// 单 BMC 单轮：快组必采（独立 deadline），慢组按 slow_interval 到期才采（独立 deadline，
/// 结果缓存跨轮复用）。快/慢组各自截止，慢 BMC 的慢组超时不再拖死快组、也不会饿死慢组缓存。
/// 返回 nv_redfish::Error 而非 String，供 401 重登/404 判定直接匹配状态码。
async fn collect_round(
    bmc: Arc<ConcreteBmc>,
    name: &str,
    slow_interval: Option<Duration>,
    slow_state: SlowState,
    timeout: Duration,
) -> Result<ScrapeReport, nv_redfish::Error<ConcreteBmc>> {
    let root = match nv_redfish::ServiceRoot::new(Arc::clone(&bmc)).await {
        Ok(r) => r,
        Err(e) if is_not_found(&e) => {
            debug!(bmc = %name, error = %e, "service root 404, skipping round");
            // 不按失败轮处理（不计 scrape_errors）：以 up=0 快照保 /metrics 序列，
            // 结构同下方失败轮的手工 report（finalize_report 空 failed 会给 up=1，不可用）。
            return Ok(ScrapeReport {
                metrics: vec![
                    Metric::gauge(UP.0, UP.1)
                        .label("bmc", name.to_string())
                        .build(0.0),
                ],
                failed_resources: vec![],
            });
        }
        Err(e) => return Err(e),
    };
    let started = Instant::now();
    let fast = match with_deadline(timeout, collect_fast(Arc::clone(&bmc), &root, name)).await {
        Some(Ok(r)) => r,
        Some(Err(resource)) => ScrapeReport {
            metrics: vec![],
            failed_resources: vec![resource],
        },
        None => {
            warn!(bmc = %name, "fast group timed out after {:?}", timeout);
            return Ok(ScrapeReport {
                metrics: vec![
                    Metric::gauge(UP.0, UP.1)
                        .label("bmc", name.to_string())
                        .build(0.0),
                ],
                failed_resources: vec!["fast:timeout".into()],
            });
        }
    };
    let cache = recover_lock(slow_state.lock()).get(name).cloned();
    let mut slow_report = cache.as_ref().and_then(|(_, r)| r.clone());
    let mut slow_failed = Vec::new();
    if slow_due(cache.as_ref().map(|(t, _)| *t), slow_interval) {
        let result = match with_deadline(timeout, collect_slow(Arc::clone(&bmc), &root, name)).await
        {
            Some(r) => r,
            None => {
                warn!(bmc = %name, "slow group timed out after {:?}", timeout);
                Err("slow:timeout".to_string())
            }
        };
        let (new_cache, failed) = apply_slow_result(cache, result, Instant::now());
        if failed.is_empty() {
            slow_report = new_cache.as_ref().and_then(|(_, r)| r.clone());
            if let Some(entry) = new_cache {
                recover_lock(slow_state.lock()).insert(name.to_string(), entry);
            }
        } else {
            // 失败也更新时间戳：慢组按 slow_interval 节奏重试，避免每轮重试打爆慢 BMC。
            slow_failed = failed;
            recover_lock(slow_state.lock())
                .insert(name.to_string(), (Instant::now(), slow_report.clone()));
            warn!(
                bmc = %name,
                resource = %slow_failed.join(","),
                "slow group failed; keeping last-good slow metrics, retrying at next slow interval"
            );
        }
    }
    let merged = merge_round(fast, slow_report.as_ref(), slow_failed);
    Ok(finalize_report(
        name,
        merged.metrics,
        merged.failed_resources,
        started,
    ))
}
