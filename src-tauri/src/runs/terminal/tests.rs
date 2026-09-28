use super::*;

#[test]
fn attach_script_quotes_path() {
    let lines = attach_script(Path::new("/Users/a b/it's \"x\"/claude"), "ddb91222");
    assert_eq!(lines[0], "tell application \"Terminal\"");
    // Shell: '/Users/a b/it'\''s "x"/claude' attach ddb91222, then escaped for AppleScript.
    assert_eq!(
        lines[2],
        r#"do script "'/Users/a b/it'\\''s \"x\"/claude' attach ddb91222""#
    );
}

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
