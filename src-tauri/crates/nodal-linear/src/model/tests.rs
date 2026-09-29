use super::*;

#[test]
fn parses_viewer_and_teams() {
    let v: ViewerData = interpret_response(
        200,
        r#"{"data":{"viewer":{"name":"Ana","email":"a@x.com"}}}"#,
    )
    .unwrap();
    assert_eq!(
        v.viewer,
        Viewer {
            name: "Ana".into(),
            email: "a@x.com".into()
        }
    );

    let t: TeamsData = interpret_response(
        200,
        r#"{"data":{"teams":{"nodes":[{"id":"t1","key":"ACME","name":"Acme"}]}}}"#,
    )
    .unwrap();
    assert_eq!(t.teams.nodes[0].key, "ACME");
}

#[test]
fn auth_error_maps_to_invalid_key() {
    let body = r#"{"errors":[{"message":"Authentication required, not authenticated",
          "extensions":{"type":"authentication error","code":"AUTHENTICATION_ERROR",
          "userPresentableMessage":"You need to authenticate."}}]}"#;
    let e = interpret_response::<ViewerData>(400, body).unwrap_err();
    assert_eq!(e, LinearError::InvalidKey);
    assert_eq!(e.kind(), "invalidKey");
}

#[test]
fn rate_limit_and_generic_errors() {
    let rl =
        r#"{"errors":[{"message":"Rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#;
    assert_eq!(
        interpret_response::<ViewerData>(400, rl).unwrap_err(),
        LinearError::RateLimited
    );
    assert_eq!(
        interpret_response::<ViewerData>(429, "").unwrap_err(),
        LinearError::RateLimited
    );

    let other = r#"{"data":null,"errors":[{"message":"Argument Validation Error",
          "extensions":{"code":"INVALID_INPUT","userPresentableMessage":"Invalid filter"}}]}"#;
    assert_eq!(
        interpret_response::<ViewerData>(400, other).unwrap_err(),
        LinearError::Api("Invalid filter".into())
    );
    assert_eq!(
        interpret_response::<ViewerData>(401, "no json").unwrap_err(),
        LinearError::InvalidKey
    );
    assert!(matches!(
        interpret_response::<ViewerData>(502, "<html>").unwrap_err(),
        LinearError::Unavailable(m) if m.contains("502")
    ));
    assert!(matches!(
        interpret_response::<ViewerData>(200, r#"{"data":{}}"#).unwrap_err(),
        LinearError::Unavailable(_)
    ));
}

#[test]
fn error_serializes_as_kind_and_message() {
    let v = serde_json::to_value(LinearError::MissingKey).unwrap();
    assert_eq!(v["kind"], "missingKey");
    assert!(v["message"].as_str().unwrap().contains("API key"));
}
