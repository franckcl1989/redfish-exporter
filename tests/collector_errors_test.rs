use nv_redfish::bmc_http::reqwest::BmcError;
use redfish_exporter::collector::error::is_not_found;

#[test]
fn classifies_404_as_not_found() {
    let err = nv_redfish::Error::Bmc(BmcError::InvalidResponse {
        url: "https://h/redfish/v1/Chassis".parse().unwrap(),
        status: reqwest::StatusCode::NOT_FOUND,
        text: "".into(),
    });
    assert!(is_not_found(&err));
}

#[test]
fn classifies_500_as_other() {
    let err = nv_redfish::Error::Bmc(BmcError::InvalidResponse {
        url: "https://h/redfish/v1/Chassis".parse().unwrap(),
        status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        text: "".into(),
    });
    assert!(!is_not_found(&err));
}
