use super::*;

#[test]
fn terminal_at_script_quotes_path() {
    let lines = terminal_at_script(
        Path::new("/Users/a b/it's \"x\""),
        Some(Path::new("/bin/claude")),
    );
    assert_eq!(
        lines[2],
        r#"do script "cd '/Users/a b/it'\\''s \"x\"' && '/bin/claude'""#
    );
    let lines = terminal_at_script(Path::new("/tmp"), None);
    assert_eq!(lines[2], r#"do script "cd '/tmp'""#);
}
