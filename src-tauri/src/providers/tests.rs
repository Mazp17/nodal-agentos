use super::*;

#[test]
fn linear_errors_classify_by_status_and_code() {
    use crate::linear::model::interpret_response;
    let kind = |status: u16, body: &str| {
        ProviderError::from(interpret_response::<serde_json::Value>(status, body).unwrap_err()).kind
    };
    assert_eq!(kind(502, "<html>"), ErrorKind::Transient);
    assert_eq!(kind(200, r#"{"data":null}"#), ErrorKind::Transient);
    assert_eq!(kind(429, ""), ErrorKind::RateLimited);
    assert_eq!(kind(401, ""), ErrorKind::Auth);
    let gql = |code: &str, msg: &str| {
        format!(r#"{{"errors":[{{"message":"{msg}","extensions":{{"code":"{code}"}}}}]}}"#)
    };
    assert_eq!(
        kind(400, &gql("AUTHENTICATION_ERROR", "x")),
        ErrorKind::Auth
    );
    assert_eq!(kind(400, &gql("RATELIMITED", "x")), ErrorKind::RateLimited);
    assert_eq!(
        kind(500, &gql("INTERNAL_SERVER_ERROR", "boom")),
        ErrorKind::Transient
    );
    // The text doesn't decide: a rejection that mentions "unavailable" is still permanent.
    assert_eq!(
        kind(
            400,
            &gql("INVALID_INPUT", "state unavailable for this team")
        ),
        ErrorKind::Permanent
    );
    assert_eq!(kind(400, &gql("FORBIDDEN", "no")), ErrorKind::Permanent);
}

#[test]
fn unknown_provider_is_rejected() {
    assert!(check_provider("linear").is_ok());
    assert!(check_provider("jira")
        .unwrap_err()
        .contains("Unknown provider"));
}
