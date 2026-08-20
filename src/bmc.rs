use anyhow::Context;
use std::sync::Arc;
use thiserror::Error;

use crate::config::BmcConfig;

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
}

pub fn build_http_client(cfg: &BmcConfig) -> Result<ReqwestClient, BmcError> {
    let mut builder =
        reqwest::Client::builder().danger_accept_invalid_certs(cfg.insecure_skip_verify);
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
    }
}

pub async fn establish_session(
    bmc: &Arc<HttpBmc<ReqwestClient>>,
    username: &str,
    password: &str,
) -> Result<(), BmcError> {
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
    bmc.set_credentials(nv_redfish::bmc_http::BmcCredentials::token(
        token.to_string(),
    ));
    Ok(())
}
