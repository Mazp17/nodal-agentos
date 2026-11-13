use super::*;

#[cfg(target_os = "macos")]
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

#[test]
fn windows_argv_prefers_wt_and_falls_back_to_cmd_start() {
    let dir = Path::new(r"C:\a b");
    let claude = Path::new(r"C:\c\claude.exe");
    let (prog, args) = windows_terminal_argv(Some(Path::new("wt.exe")), dir, Some(claude));
    assert_eq!(prog, "wt.exe");
    assert_eq!(args, ["-d", r"C:\a b", r"C:\c\claude.exe"]);
    let (prog, args) = windows_terminal_argv(None, dir, None);
    assert_eq!(prog, "cmd.exe");
    assert_eq!(args, ["/C", "start", "", "/D", r"C:\a b", "cmd.exe"]);
    let (_, args) = windows_terminal_argv(None, dir, Some(claude));
    assert_eq!(&args[6..], ["/K", r"C:\c\claude.exe"]);
}
