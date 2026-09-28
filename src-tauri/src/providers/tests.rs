use super::*;

// `linear_errors_classify_by_status_and_code` moved to
// `nodal_linear::error::tests`, with the `From<LinearError> for ProviderError`
// impl it exercises.

#[test]
fn unknown_provider_is_rejected() {
    assert!(check_provider("linear").is_ok());
    assert!(check_provider("jira")
        .unwrap_err()
        .contains("Unknown provider"));
}
