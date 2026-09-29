//! `CommandError` must serialize exactly like today's `Result<T, String>` (a plain JSON
//! string): the frontend reads `error` as a string, and Tauri serializes a command's `Err`
//! with `serde_json`, so a bare `String` and `CommandError` must produce the same bytes.

use super::*;

#[test]
fn serializes_as_a_plain_string_like_today_s_string_error() {
    let e = CommandError::from("Couldn't find the project.".to_string());
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        serde_json::to_value("Couldn't find the project.").unwrap()
    );
}

#[test]
fn from_string_and_app_error_round_trip_the_message() {
    let from_string = CommandError::from("boom".to_string());
    assert_eq!(serde_json::to_value(&from_string).unwrap(), "boom");

    let app_err: CommandError = nodal_app::AppError::from("kaboom").into();
    assert_eq!(serde_json::to_value(&app_err).unwrap(), "kaboom");
}
