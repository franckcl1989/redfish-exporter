use redfish_exporter::config::StabilityConfig;
use redfish_exporter::stability::{
    BmcState, RoundAction, RoundOutcome, next_backoff, on_round_result, round_action,
};
use std::time::{Duration, Instant};

fn cfg() -> StabilityConfig {
    StabilityConfig {
        cooldown_failures: 3,
        cooldown_base: Duration::from_secs(60),
        cooldown_max: Duration::from_secs(300),
    }
}

fn ok_outcome() -> RoundOutcome {
    RoundOutcome {
        report: Some(redfish_exporter::collector::ScrapeReport {
            metrics: vec![],
            failed_resources: vec![],
        }),
        message: String::new(),
        session_recovery_failed: false,
        attempted_session: false,
    }
}

fn err_outcome(session: bool, attempted: bool) -> RoundOutcome {
    RoundOutcome {
        report: None,
        message: "fail".into(),
        session_recovery_failed: session,
        attempted_session: attempted,
    }
}

#[test]
fn backoff_sequence_matches_spec() {
    let (b, m) = (Duration::from_secs(60), Duration::from_secs(300));
    assert_eq!(next_backoff(1, b, m), Duration::from_secs(60));
    assert_eq!(next_backoff(2, b, m), Duration::from_secs(120));
    assert_eq!(next_backoff(3, b, m), Duration::from_secs(240));
    assert_eq!(next_backoff(4, b, m), Duration::from_secs(300));
    assert_eq!(next_backoff(10, b, m), Duration::from_secs(300));
}

#[test]
fn round_action_healthy_scrapes() {
    assert_eq!(
        round_action(&BmcState::Healthy { failures: 0 }, Instant::now()),
        RoundAction::Scrape
    );
}

#[test]
fn round_action_cooling_waits_then_scrapes() {
    let now = Instant::now();
    let c = BmcState::Cooling {
        failures: 3,
        next_attempt: now + Duration::from_secs(60),
    };
    assert_eq!(round_action(&c, now), RoundAction::CooldownWait);
    assert_eq!(
        round_action(&c, now + Duration::from_secs(59)),
        RoundAction::CooldownWait
    );
    assert_eq!(
        round_action(&c, now + Duration::from_secs(61)),
        RoundAction::Scrape
    );
}

#[test]
fn round_action_degraded_retries_session_on_schedule() {
    let now = Instant::now();
    let d = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 1,
        next_session_retry: now + Duration::from_secs(60),
    };
    assert_eq!(round_action(&d, now), RoundAction::Scrape);
    assert_eq!(
        round_action(&d, now + Duration::from_secs(61)),
        RoundAction::SessionRetry
    );
}

#[test]
fn healthy_failures_count_to_cooldown() {
    let now = Instant::now();
    let mut s = BmcState::Healthy { failures: 0 };
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 1 });
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 2 });
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert!(matches!(s, BmcState::Cooling { failures: 3, .. }));
}

#[test]
fn healthy_success_resets_counter() {
    let now = Instant::now();
    let s = BmcState::Healthy { failures: 2 };
    let s = on_round_result(&s, &ok_outcome(), false, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 0 });
}

#[test]
fn healthy_session_recovery_failure_degrades() {
    let now = Instant::now();
    let s = BmcState::Healthy { failures: 0 };
    let s = on_round_result(&s, &err_outcome(true, true), true, &cfg(), now);
    match s {
        BmcState::SessionDegraded {
            basic_failures: 0,
            session_failures: 1,
            ..
        } => {}
        other => panic!("expected SessionDegraded, got {other:?}"),
    }
}

#[test]
fn cooling_retry_success_recovers() {
    let now = Instant::now();
    let s = BmcState::Cooling {
        failures: 3,
        next_attempt: now,
    };
    let s = on_round_result(&s, &ok_outcome(), false, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 0 });
}

#[test]
fn cooling_retry_failure_extends_backoff() {
    let now = Instant::now();
    let s = BmcState::Cooling {
        failures: 3,
        next_attempt: now,
    };
    let s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    match s {
        BmcState::Cooling {
            failures: 4,
            next_attempt,
        } => {
            assert_eq!(next_attempt, now + Duration::from_secs(300));
        }
        other => panic!("expected Cooling, got {other:?}"),
    }
}

#[test]
fn degraded_basic_success_stays_degraded_with_reset_basic_failures() {
    let now = Instant::now();
    let s = BmcState::SessionDegraded {
        basic_failures: 2,
        session_failures: 1,
        next_session_retry: now + Duration::from_secs(60),
    };
    let s = on_round_result(&s, &ok_outcome(), false, &cfg(), now);
    match s {
        BmcState::SessionDegraded {
            basic_failures: 0,
            session_failures: 1,
            ..
        } => {}
        other => panic!("expected SessionDegraded, got {other:?}"),
    }
}

#[test]
fn degraded_session_retry_success_recovers_to_healthy() {
    let now = Instant::now();
    let s = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 2,
        next_session_retry: now,
    };
    let mut ok = ok_outcome();
    ok.attempted_session = true;
    let s = on_round_result(&s, &ok, false, &cfg(), now);
    assert_eq!(s, BmcState::Healthy { failures: 0 });
}

#[test]
fn degraded_session_retry_failure_but_basic_ok_keeps_degraded() {
    let now = Instant::now();
    let s = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 2,
        next_session_retry: now,
    };
    let mut ok = ok_outcome();
    ok.attempted_session = true;
    ok.session_recovery_failed = true;
    let s = on_round_result(&s, &ok, false, &cfg(), now);
    match s {
        BmcState::SessionDegraded {
            basic_failures: 0,
            session_failures: 3,
            ..
        } => {}
        other => panic!("expected SessionDegraded with session_failures=3, got {other:?}"),
    }
}

#[test]
fn degraded_basic_failures_reach_cooldown() {
    let now = Instant::now();
    let mut s = BmcState::SessionDegraded {
        basic_failures: 0,
        session_failures: 1,
        next_session_retry: now + Duration::from_secs(60),
    };
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    s = on_round_result(&s, &err_outcome(false, false), true, &cfg(), now);
    assert!(matches!(s, BmcState::Cooling { .. }));
}
