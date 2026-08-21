use anyhow::Context;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use thiserror::Error;

use crate::config::{AuthMethod, BmcConfig, SecretString};

pub type ReqwestClient = nv_redfish::bmc_http::reqwest::Client;
pub type HttpBmc<C> = nv_redfish::bmc_http::HttpBmc<C>;

#[derive(Debug, Error)]
pub enum BmcError {
    #[error("failed to build http client for bmc: {0}")]
    Transport(#[from] anyhow::Error),
    #[error("session establishment failed: {0}")]
    Session(String),
}

pub struct BmcHandle {
    pub name: String,
    pub bmc: Arc<HttpBmc<ReqwestClient>>,
    pub username: String,
    pub password: SecretString,
    pub auth: AuthMethod,
    /// Session 认证是否已成功建立（仅对 Session auth 有意义）。
    /// Arc<AtomicBool>：跨轮共享，scraper 各轮任务都可读取/置位。
    pub session_established: Arc<AtomicBool>,
}

/// 判断采集错误是否由 401 未授权引起（session token 过期等场景）。
///
/// nv-redfish-bmc-http 0.15.1 中非 2xx 响应统一映射为
/// `BmcError::InvalidResponse { status, .. }`（reqwest.rs:729-735），
/// 传输层错误才是 `BmcError::ReqwestError`，故此处匹配 InvalidResponse。
pub fn is_unauthorized(err: &nv_redfish::Error<HttpBmc<ReqwestClient>>) -> bool {
    matches!(
        err,
        nv_redfish::Error::Bmc(nv_redfish::bmc_http::reqwest::BmcError::InvalidResponse {
            status,
            ..
        }) if *status == reqwest::StatusCode::UNAUTHORIZED
    )
}

pub fn build_http_client(
    cfg: &BmcConfig,
    request_timeout: Duration,
) -> Result<ReqwestClient, BmcError> {
    let mut builder = reqwest::Client::builder()
        .timeout(request_timeout)
        .connect_timeout(std::time::Duration::from_secs(5))
        .user_agent("nv-redfish/v1");
    if cfg.insecure_skip_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(ca) = &cfg.ca_cert_file {
        let pem =
            std::fs::read(ca).with_context(|| format!("read ca_cert_file '{}'", ca.display()))?;
        let cert = reqwest::Certificate::from_pem(&pem)
            .with_context(|| format!("parse ca_cert_file '{}'", ca.display()))?;
        builder = builder.add_root_certificate(cert);
    }
    let client = builder.build().map_err(anyhow::Error::new)?;
    Ok(nv_redfish::bmc_http::reqwest::Client::with_client(client))
}

pub fn make_bmc(cfg: &BmcConfig, client: ReqwestClient) -> BmcHandle {
    let credentials = nv_redfish::bmc_http::BmcCredentials::username_password(
        cfg.username.clone(),
        Some(cfg.password.expose().to_string()),
    );
    let bmc = HttpBmc::new(
        client,
        cfg.host.clone(),
        credentials,
        nv_redfish::bmc_http::CacheSettings::default(),
    );
    BmcHandle {
        name: cfg.name.clone(),
        bmc: Arc::new(bmc),
        username: cfg.username.clone(),
        password: cfg.password.clone(),
        auth: cfg.auth,
        session_established: Arc::new(AtomicBool::new(false)),
    }
}

/// 建立 Redfish 会话并返回会话 token。
///
/// 泛型 `<B: nv_redfish::Bmc>`：token 的写回（`HttpBmc::set_credentials`）
/// 是 `HttpBmc` 特有能力，`nv_redfish::Bmc` trait 无凭据概念，
/// 故此处返回 token 由调用方应用（scraper.rs），这也使本流程
/// 可被 MockBmc 全流程测试。
pub async fn establish_session<B: nv_redfish::Bmc>(
    bmc: &Arc<B>,
    username: &str,
    password: &str,
) -> Result<String, BmcError> {
    let root = nv_redfish::ServiceRoot::new(Arc::clone(bmc))
        .await
        .map_err(|e| BmcError::Session(format!("service root: {e}")))?;
    let Some(session_service) = root
        .session_service()
        .await
        .map_err(|e| BmcError::Session(format!("session service: {e}")))?
    else {
        return Err(BmcError::Session("session service not available".into()));
    };
    let Some(sessions) = session_service
        .sessions()
        .await
        .map_err(|e| BmcError::Session(format!("sessions: {e}")))?
    else {
        return Err(BmcError::Session(
            "sessions collection not available".into(),
        ));
    };
    let create = nv_redfish::schema::session::SessionCreate::builder(
        username.to_string(),
        password.to_string(),
    )
    .build();
    let session = sessions
        .create_session(&create)
        .await
        .map_err(|e| BmcError::Session(format!("create session: {e}")))?;
    let Some(token) = session.auth_token() else {
        return Err(BmcError::Session(
            "session response without auth token".into(),
        ));
    };
    Ok(token.to_string())
}
