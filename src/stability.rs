//! per-BMC 稳定性状态机（spec `docs/superpowers/specs/2026-08-24-stability-hardening-design.md` §4）。
//! 纯函数 + 数据，供 scraper 调度与单元测试（与 scraper.rs 的 slow_due 等测试接缝同法）。

use std::time::{Duration, Instant};

use crate::collector::ScrapeReport;
use crate::config::StabilityConfig;

/// 单 BMC 稳定状态（spec §4.1 状态机）。
#[derive(Debug, Clone, PartialEq)]
pub enum BmcState {
    /// 正常采集；failures 为连续失败轮计数。
    Healthy { failures: u32 },
    /// 会话降级：basic 凭据兜底采集（每轮），session 按退避节奏重试。
    SessionDegraded {
        basic_failures: u32,
        session_failures: u32,
        next_session_retry: Instant,
    },
    /// 全停冷却：到期重试（指数退避）。
    Cooling {
        failures: u32,
        next_attempt: Instant,
    },
}

/// 一轮调度动作（scraper 在 spawn 前按状态与时钟判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundAction {
    /// 正常采集轮（Healthy / Cooling 到期重试 / Degraded 的 basic 轮）。
    Scrape,
    /// Degraded 且 session 退避到期：先试 session 重挂，失败则 basic 兜底（同轮内）。
    SessionRetry,
    /// 冷却等待：本轮不发任何请求，直接发布 up=0 + resource=cooldown。
    CooldownWait,
}

/// 单轮执行结果（run_bmc_round 产出）。
#[derive(Debug)]
pub struct RoundOutcome {
    /// 成功轮的报告（含 up=0 的部分失败轮；失败判定由调用方用 `failed` 归一化）。
    pub report: Option<ScrapeReport>,
    /// 失败轮的描述（timeout/bmc 等，与既有失败报告语义一致）。
    pub message: String,
    /// 本轮尝试了 session 建立/重登且失败（401 重登失败或会话重挂失败）。
    pub session_recovery_failed: bool,
    /// 本轮是否尝试过 session 建立/重登（Healthy 轮/Cooling 重试/SessionRetry 为 true，basic 兜底轮为 false）。
    pub attempted_session: bool,
}

/// 指数退避：min(max, base × 2^(failures-1))；failures 从 1 起（spec：60/120/240/300s 封顶）。
pub fn next_backoff(failures: u32, base: Duration, max: Duration) -> Duration {
    let factor = 1u32
        .checked_shl(failures.saturating_sub(1).min(31))
        .unwrap_or(u32::MAX);
    base.saturating_mul(factor).min(max)
}

/// 判定本轮动作。
pub fn round_action(state: &BmcState, now: Instant) -> RoundAction {
    match state {
        BmcState::Healthy { .. } => RoundAction::Scrape,
        BmcState::Cooling { next_attempt, .. } => {
            if now >= *next_attempt {
                RoundAction::Scrape
            } else {
                RoundAction::CooldownWait
            }
        }
        BmcState::SessionDegraded {
            next_session_retry, ..
        } => {
            if now >= *next_session_retry {
                RoundAction::SessionRetry
            } else {
                RoundAction::Scrape
            }
        }
    }
}

/// 应用一轮结果推进状态机。
/// `failed`：本轮是否失败（report 为 None 或 failed_resources 非空）。
pub fn on_round_result(
    state: &BmcState,
    outcome: &RoundOutcome,
    failed: bool,
    cfg: &StabilityConfig,
    now: Instant,
) -> BmcState {
    let ok = !failed && outcome.report.is_some();
    match state {
        BmcState::Healthy { failures } => {
            if outcome.session_recovery_failed {
                // 不变量（run_bmc_round 的守卫保证）：session_recovery_failed ⇒
                // attempted_session——该标志仅在 attempt_session=true 的轮内
                // （轮首建会话失败 / 401 重登失败）置位。
                debug_assert!(outcome.attempted_session);
                BmcState::SessionDegraded {
                    basic_failures: 0,
                    session_failures: 1,
                    next_session_retry: now + next_backoff(1, cfg.cooldown_base, cfg.cooldown_max),
                }
            } else if ok {
                BmcState::Healthy { failures: 0 }
            } else {
                let f = failures.saturating_add(1);
                if f >= cfg.cooldown_failures {
                    BmcState::Cooling {
                        failures: f,
                        next_attempt: now + next_backoff(f, cfg.cooldown_base, cfg.cooldown_max),
                    }
                } else {
                    BmcState::Healthy { failures: f }
                }
            }
        }
        BmcState::Cooling { failures, .. } => {
            if outcome.session_recovery_failed {
                // 冷却到期后若 session 仍不可用、但 basic 兜底成功，应进入降级态，
                // 否则会误回 Healthy 并在每一轮重复冲击 SessionService。
                debug_assert!(outcome.attempted_session);
                BmcState::SessionDegraded {
                    basic_failures: 0,
                    session_failures: 1,
                    next_session_retry: now + next_backoff(1, cfg.cooldown_base, cfg.cooldown_max),
                }
            } else if ok {
                BmcState::Healthy { failures: 0 }
            } else {
                let f = failures.saturating_add(1);
                BmcState::Cooling {
                    failures: f,
                    next_attempt: now + next_backoff(f, cfg.cooldown_base, cfg.cooldown_max),
                }
            }
        }
        BmcState::SessionDegraded {
            basic_failures,
            session_failures,
            next_session_retry,
        } => {
            if ok {
                if outcome.attempted_session && !outcome.session_recovery_failed {
                    // session 重挂成功
                    BmcState::Healthy { failures: 0 }
                } else if outcome.attempted_session {
                    // session 仍失败但 basic 成功：继续降级，session 退避推进
                    let sf = session_failures.saturating_add(1);
                    BmcState::SessionDegraded {
                        basic_failures: 0,
                        session_failures: sf,
                        next_session_retry: now
                            + next_backoff(sf, cfg.cooldown_base, cfg.cooldown_max),
                    }
                } else {
                    // 纯 basic 轮成功
                    BmcState::SessionDegraded {
                        basic_failures: 0,
                        session_failures: *session_failures,
                        next_session_retry: *next_session_retry,
                    }
                }
            } else {
                let bf = basic_failures.saturating_add(1);
                let sf = if outcome.attempted_session {
                    session_failures.saturating_add(1)
                } else {
                    *session_failures
                };
                let nxt = if outcome.attempted_session {
                    now + next_backoff(sf, cfg.cooldown_base, cfg.cooldown_max)
                } else {
                    *next_session_retry
                };
                if bf >= cfg.cooldown_failures {
                    BmcState::Cooling {
                        failures: bf,
                        next_attempt: now + next_backoff(bf, cfg.cooldown_base, cfg.cooldown_max),
                    }
                } else {
                    BmcState::SessionDegraded {
                        basic_failures: bf,
                        session_failures: sf,
                        next_session_retry: nxt,
                    }
                }
            }
        }
    }
}
