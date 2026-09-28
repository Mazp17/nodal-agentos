use super::*;

#[test]
fn iso_from_ms_matches_known_dates() {
    assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(iso_from_ms(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    assert_eq!(iso_from_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
}
