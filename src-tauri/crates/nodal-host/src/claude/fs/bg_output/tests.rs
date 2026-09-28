use super::*;

#[test]
fn bg_line_real_format() {
    assert_eq!(parse_bg_line("backgrounded · ddb91222").as_deref(), Some("ddb91222"));
    assert_eq!(
        parse_bg_line("\u{1b}[2mbackgrounded\u{1b}[0m · \u{1b}[1mddb91222\u{1b}[0m").as_deref(),
        Some("ddb91222")
    );
    assert_eq!(parse_bg_line("  backgrounded · ab12cd34  \r").as_deref(), Some("ab12cd34"));
    assert_eq!(
        parse_bg_line("backgrounded · ab12cd34 · run `claude agents` to view").as_deref(),
        Some("ab12cd34")
    );
    assert_eq!(parse_bg_line("backgrounded"), None);
    assert_eq!(parse_bg_line("Error: not logged in"), None);
    assert_eq!(parse_bare_id("ddb91222\n").as_deref(), Some("ddb91222"));
    assert_eq!(parse_bare_id("hello world"), None);
}
