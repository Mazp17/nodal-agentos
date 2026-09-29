use std::fs;
use std::path::Path;

use super::*;
use crate::testutil::TempDir;

fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[test]
fn repo_settings_override_the_user_ones() {
    let t = TempDir::new("claude-settings");
    let (claude, repo) = (t.0.join("claude"), t.0.join("repo"));
    assert_eq!(read(Some(&claude), Some(&repo)), ClaudeDefaults::default());

    write(
        &claude.join("settings.json"),
        r#"{"model":"opus[1m]","effortLevel":"xhigh"}"#,
    );
    let user = read(Some(&claude), Some(&repo));
    assert_eq!(
        (user.model.as_deref(), user.effort.as_deref()),
        (Some("opus[1m]"), Some("xhigh"))
    );

    write(
        &repo.join(".claude/settings.json"),
        r#"{"model":"sonnet","effortLevel":""}"#,
    );
    write(&repo.join(".claude/settings.local.json"), "not json");
    let both = read(Some(&claude), Some(&repo));
    assert_eq!(
        (both.model.as_deref(), both.effort.as_deref()),
        (Some("sonnet"), Some("xhigh"))
    );
    assert_eq!(read(Some(&claude), None).model.as_deref(), Some("opus[1m]"));
}
