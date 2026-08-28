use axum::{Router, routing::get};
use nv_redfish::ServiceRoot;
use nv_redfish_bmc_mock::{Bmc as MockBmc, Expect};
use redfish_exporter::bmc::{BmcHandle, build_http_client, make_bmc};
use redfish_exporter::config::{
    AuthMethod, BmcConfig, CollectorsConfig, SecretString, StabilityConfig,
};
use redfish_exporter::scraper::RoundOptions;
use redfish_exporter::stability::{
    BmcState, RoundAction, RoundOutcome, next_backoff, on_round_result, round_action,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

fn cfg() -> StabilityConfig {
    StabilityConfig {
        cooldown_failures: 3,
        cooldown_base: Duration::from_secs(60),
        cooldown_max: Duration::from_secs(300),
    }
}

fn round_options(attempt_session: bool, use_basic_only: bool) -> RoundOptions {
    RoundOptions {
        timeout: Duration::from_secs(5),
        attempt_session,
        use_basic_only,
        collectors: CollectorsConfig::default(),
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
fn healthy_basic_success_after_session_failure_still_degrades() {
    let now = Instant::now();
    let state = BmcState::Healthy { failures: 0 };
    let mut outcome = ok_outcome();
    outcome.attempted_session = true;
    outcome.session_recovery_failed = true;
    let state = on_round_result(&state, &outcome, false, &cfg(), now);
    assert!(matches!(state, BmcState::SessionDegraded { .. }));
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
fn cooling_retry_with_basic_fallback_enters_session_degraded() {
    let now = Instant::now();
    let state = BmcState::Cooling {
        failures: 3,
        next_attempt: now,
    };
    let mut outcome = ok_outcome();
    outcome.attempted_session = true;
    outcome.session_recovery_failed = true;

    let state = on_round_result(&state, &outcome, false, &cfg(), now);
    match state {
        BmcState::SessionDegraded {
            basic_failures: 0,
            session_failures: 1,
            next_session_retry,
        } => assert_eq!(next_session_retry, now + Duration::from_secs(60)),
        other => panic!("expected SessionDegraded, got {other:?}"),
    }
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

// ---------- 故障注入（真实 HTTP，Task 5） ----------
// MockBmc 无法表达 401 状态码，故用真实 axum 服务器驱动 run_bmc_round 与状态机端到端。

/// 起一个真实 HTTP 服务器：所有请求返回给定状态码。
fn spawn_server(status: axum::http::StatusCode) -> (String, tokio::task::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    // Windows：axum_server 的 from_tcp 要求 listener 非阻塞，否则 accept 出的连接
    // 在 IOCP 下 I/O 永久挂起（见 src/http.rs serve_on 同款注释）。
    listener.set_nonblocking(true).unwrap();
    let app = Router::new().fallback(move || async move { (status, "unauthorized") });
    let handle = tokio::spawn(async move {
        axum_server::from_tcp(listener)
            .unwrap()
            .serve(app.into_make_service())
            .await
            .unwrap();
    });
    (format!("http://{addr}"), handle)
}

/// 以指定认证方式构建 BmcHandle（不建会话——建立/重登由 run_bmc_round 轮内驱动）。
fn auth_handle(host: &str, auth: AuthMethod) -> BmcHandle {
    let cfg = BmcConfig {
        name: "inj".into(),
        host: Url::parse(host).unwrap(),
        username: "admin".into(),
        password: SecretString::new("pw".into()),
        auth,
        insecure_skip_verify: false,
        ca_cert_file: None,
    };
    let client = build_http_client(&cfg, Duration::from_secs(5)).unwrap();
    make_bmc(&cfg, client)
}

/// Session 认证的 BmcHandle。
fn session_handle(host: &str) -> BmcHandle {
    auth_handle(host, AuthMethod::Session)
}

/// Basic 认证的 BmcHandle。
fn basic_handle(host: &str) -> BmcHandle {
    auth_handle(host, AuthMethod::Basic)
}

/// 401-only 服务器：首轮 Healthy 尝试 session → 401 → 重登失败（session_recovery_failed）→
/// 进入 SessionDegraded；basic 兜底轮同样 401 → basic 连续失败 3 次 → Cooling。
#[tokio::test]
async fn fault_injection_401_only_server_degrades_then_cools() {
    let (host, _server) = spawn_server(axum::http::StatusCode::UNAUTHORIZED);
    let handle = session_handle(&host);
    let slow_state = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let cfg = cfg();
    // 第 1 轮：Healthy 尝试 session → 401 → 重登失败 → session_recovery_failed
    let o = redfish_exporter::scraper::run_bmc_round(
        &handle,
        None,
        &slow_state,
        &sessions,
        round_options(true, false),
    )
    .await;
    assert!(o.report.is_none());
    assert!(o.session_recovery_failed);
    let mut state = BmcState::Healthy { failures: 0 };
    state = on_round_result(&state, &o, true, &cfg, Instant::now());
    assert!(matches!(state, BmcState::SessionDegraded { .. }));
    // 第 2-4 轮：basic 兜底轮（attempted_session=false）→ 401 → basic_failures 累加至 Cooling
    for _ in 0..3 {
        let o = redfish_exporter::scraper::run_bmc_round(
            &handle,
            None,
            &slow_state,
            &sessions,
            round_options(false, true),
        )
        .await;
        assert!(o.report.is_none());
        assert!(!o.session_recovery_failed);
        state = on_round_result(&state, &o, true, &cfg, Instant::now());
    }
    assert!(matches!(state, BmcState::Cooling { .. }));
}

/// 会话 token 被拒（真实 401）而 basic 仅对服务根放行的服务器：带 Authorization: Basic 的
/// /redfish/v1 返回 200 最小服务根，其余请求（含 SessionService/Sessions 端点）一律 401。
/// 状态流：token 401 → 重登失败（服务根 basic GET 成功，但 SessionService/Sessions
/// 端点返回 401，重登走不通）→ session_recovery_failed → Healthy 降级 SessionDegraded
/// → basic 兜底轮成功（up=1、failed_resources 为空）→ 仍处 SessionDegraded 并带
/// session-degraded 标记。
#[tokio::test]
async fn fault_injection_session_401_basic_ok_degrades_with_mark() {
    use axum::response::IntoResponse;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/redfish/v1",
            get(|req: axum::extract::Request| async move {
                let basic = req
                    .headers()
                    .get(axum::http::header::AUTHORIZATION)
                    .is_some_and(|v| v.as_bytes().starts_with(b"Basic "));
                if basic {
                    axum::Json(serde_json::json!({
                        "@odata.id": "/redfish/v1",
                        "Id": "Root",
                        "Name": "Root",
                        "RedfishVersion": "1.0.0",
                        "Links": {
                            "Sessions": {
                                "@odata.id": "/redfish/v1/SessionService/Sessions",
                            },
                        },
                    }))
                    .into_response()
                } else {
                    (axum::http::StatusCode::UNAUTHORIZED, "unauthorized").into_response()
                }
            }),
        )
        .fallback(|| async { (axum::http::StatusCode::UNAUTHORIZED, "no") });
    // Windows：非阻塞 listener 要求，见 spawn_server 注释。
    listener.set_nonblocking(true).unwrap();
    let _server = tokio::spawn(async move {
        axum_server::from_tcp(listener)
            .unwrap()
            .serve(app.into_make_service())
            .await
            .unwrap();
    });
    let handle = session_handle(&format!("http://{addr}"));
    // 模拟"已建立但已失效"的会话：预置 token 凭据与 established 标记，使首轮采集
    // 以 X-Auth-Token 请求被 401 拒绝，走进真实的重登路径（新 handle 默认 basic 凭据，
    // 不预置则首轮走轮首建会话流程，而非 401 重登）。
    handle
        .bmc
        .set_credentials(nv_redfish::bmc_http::BmcCredentials::token(
            "stale-token".into(),
        ));
    handle
        .session_established
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let slow_state = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let cfg = cfg();
    // 第 1 轮：token 401 → 重登（basic 凭据建会话）→ 服务根 basic GET 成功但
    // SessionService/Sessions 端点返回 401 → 重登失败
    let o = redfish_exporter::scraper::run_bmc_round(
        &handle,
        None,
        &slow_state,
        &sessions,
        round_options(true, false),
    )
    .await;
    assert!(o.report.is_none());
    assert!(o.session_recovery_failed);
    let mut state = BmcState::Healthy { failures: 0 };
    state = on_round_result(&state, &o, true, &cfg, Instant::now());
    assert!(matches!(state, BmcState::SessionDegraded { .. }));
    // 第 2 轮：basic 兜底轮 → root 200（basic 放行），无导航链接的 collect_fast/slow
    // 全空成功 → up=1 且 failed_resources 为空；仍处 SessionDegraded。
    let o = redfish_exporter::scraper::run_bmc_round(
        &handle,
        None,
        &slow_state,
        &sessions,
        round_options(false, true),
    )
    .await;
    assert!(o.report.is_some(), "round 2 message: {}", o.message);
    let failed = o
        .report
        .as_ref()
        .is_some_and(|r| !r.failed_resources.is_empty());
    state = on_round_result(&state, &o, failed, &cfg, Instant::now());
    assert!(matches!(state, BmcState::SessionDegraded { .. }));
    let report = o.report.unwrap();
    assert!(report.failed_resources.is_empty());
    assert!(
        report
            .metrics
            .iter()
            .any(|m| m.name == "redfish_up" && m.value == 1.0)
    );
    // 降级标记：redfish_scrape_error{resource="session-degraded"}=1
    let marked = redfish_exporter::scraper::with_degraded_mark(report, "inj");
    assert!(
        marked
            .metrics
            .iter()
            .any(|m| m.name == "redfish_scrape_error")
    );
}

/// 乱码响应（spec §5.1「错误/乱码响应」行）：MockBmc 期望队列注入形状错误的
/// 服务根 JSON（合法 JSON、非 ServiceRoot 形状）→ ServiceRoot::new 解析失败
/// （BadResponseJson）→ 轮失败 → 连续 3 轮 → Cooling。
/// MockBmc 无法驱动 HttpBmc 专用的 run_bmc_round，故轮失败结果按 run_bmc_round
/// 非 401/404 错误臂的产出同构构造（report=None、session_recovery_failed=false、
/// attempted_session=true），状态机推进为真实代码。
#[tokio::test]
async fn fault_injection_garbled_root_mock_reaches_cooldown() {
    let bmc = Arc::new(MockBmc::<serde_json::Error>::default());
    bmc.expect(Expect::get(
        nv_redfish::core::ODataId::service_root(),
        json!({ "Id": 12345 }),
    ));
    let err = match ServiceRoot::new(Arc::clone(&bmc)).await {
        Err(e) => e,
        Ok(_) => panic!("wrong-shape root must fail deserialization"),
    };
    assert!(format!("{err}").contains("json"), "unexpected error: {err}");
    let outcome = RoundOutcome {
        report: None,
        message: err.to_string(),
        session_recovery_failed: false,
        attempted_session: true,
    };
    let cfg = cfg();
    let now = Instant::now();
    let mut state = BmcState::Healthy { failures: 0 };
    for _ in 0..3 {
        state = on_round_result(&state, &outcome, true, &cfg, now);
    }
    assert!(matches!(state, BmcState::Cooling { .. }));
}

/// 乱码响应端到端（真实 HTTP，spec §5.1 同一行）：服务器对一切请求返回 200 +
/// 合法 JSON 但非 ServiceRoot 形状（数组）→ HttpBmc 解析失败（JsonError，非
/// 401/404）→ run_bmc_round 轮失败（report=None）→ 连续 3 轮 → Cooling。
/// 与 MockBmc 注入测试互为补充：本测试走 ServiceRoot → collect_round →
/// run_bmc_round → 状态机完整真实链路。
#[tokio::test]
async fn fault_injection_garbled_response_round_fails_then_cools() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(|| async { (axum::http::StatusCode::OK, "[]") });
    // Windows：非阻塞 listener 要求，见 spawn_server 注释。
    listener.set_nonblocking(true).unwrap();
    let _server = tokio::spawn(async move {
        axum_server::from_tcp(listener)
            .unwrap()
            .serve(app.into_make_service())
            .await
            .unwrap();
    });
    let handle = basic_handle(&format!("http://{addr}"));
    let slow_state = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let cfg = cfg();
    let mut state = BmcState::Healthy { failures: 0 };
    for _ in 0..3 {
        let o = redfish_exporter::scraper::run_bmc_round(
            &handle,
            None,
            &slow_state,
            &sessions,
            round_options(true, false),
        )
        .await;
        assert!(o.report.is_none(), "garbled round must fail: {}", o.message);
        assert!(!o.session_recovery_failed);
        state = on_round_result(&state, &o, true, &cfg, Instant::now());
    }
    assert!(matches!(state, BmcState::Cooling { .. }));
}

/// A missing Redfish ServiceRoot is a real failed round. Treating it as an
/// empty success would reset the failure counter and hammer a bad endpoint
/// forever instead of entering adaptive cooldown.
#[tokio::test]
async fn fault_injection_root_404_round_fails_then_cools() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(|| async { axum::http::StatusCode::NOT_FOUND });
    listener.set_nonblocking(true).unwrap();
    let _server = tokio::spawn(async move {
        axum_server::from_tcp(listener)
            .unwrap()
            .serve(app.into_make_service())
            .await
            .unwrap();
    });
    let handle = basic_handle(&format!("http://{addr}"));
    let slow_state = Arc::new(Mutex::new(HashMap::new()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let cfg = cfg();
    let mut state = BmcState::Healthy { failures: 0 };
    for _ in 0..3 {
        let outcome = redfish_exporter::scraper::run_bmc_round(
            &handle,
            None,
            &slow_state,
            &sessions,
            round_options(true, false),
        )
        .await;
        assert!(
            outcome.report.is_none(),
            "404 ServiceRoot must fail: {}",
            outcome.message
        );
        assert!(
            outcome.message.contains("404"),
            "unexpected error: {outcome:?}"
        );
        state = on_round_result(&state, &outcome, true, &cfg, Instant::now());
    }
    assert!(matches!(state, BmcState::Cooling { .. }));
}
