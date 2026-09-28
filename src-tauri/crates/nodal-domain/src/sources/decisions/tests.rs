use super::*;

#[test]
fn backoff_grows_and_caps() {
    assert_eq!(backoff_ms(1), 30_000);
    assert_eq!(backoff_ms(2), 60_000);
    assert_eq!(backoff_ms(3), 120_000);
    assert_eq!(backoff_ms(50), BACKOFF_MAX_MS);
}
