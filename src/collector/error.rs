use crate::bmc::{HttpBmc, ReqwestClient};

pub type ConcreteBmc = HttpBmc<ReqwestClient>;

/// 判断采集错误是否为 404 Not Found（service root 缺失等场景）。
///
/// 非 2xx 响应统一映射为 `BmcError::InvalidResponse { status, .. }`
/// （同 bmc.rs is_unauthorized 的错误模型），此处精确匹配 404。
pub fn is_not_found(err: &nv_redfish::Error<ConcreteBmc>) -> bool {
    matches!(
        err,
        nv_redfish::Error::Bmc(nv_redfish::bmc_http::reqwest::BmcError::InvalidResponse {
            status,
            ..
        }) if *status == reqwest::StatusCode::NOT_FOUND
    )
}
