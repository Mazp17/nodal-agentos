//! Claude Code's configured defaults (`model`, `effortLevel`) from its settings files, so the
//! UI can say what "default" means. Later layers win: `~/.claude/settings.json`, then the
//! repo's `.claude/settings.json` and `.claude/settings.local.json`. Managed (enterprise)
//! settings and env vars like `ANTHROPIC_MODEL` are not read.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDefaults {
    /// As written in the settings (`opus`, `opus[1m]`, a full model id…); `None` if unset.
    pub model: Option<String>,
    pub effort: Option<String>,
}

fn non_empty(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn read(claude_dir: Option<&Path>, repo: Option<&Path>) -> ClaudeDefaults {
    let mut files = Vec::new();
    if let Some(d) = claude_dir {
        files.push(d.join("settings.json"));
    }
    if let Some(r) = repo {
        files.push(r.join(".claude").join("settings.json"));
        files.push(r.join(".claude").join("settings.local.json"));
    }
    let mut out = ClaudeDefaults::default();
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if let Some(m) = non_empty(&v, "model") {
            out.model = Some(m);
        }
        if let Some(e) = non_empty(&v, "effortLevel") {
            out.effort = Some(e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::paths::tests::TempDir;

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn repo_settings_override_the_user_ones() {
        let t = TempDir::new("claude-settings");
        let (claude, repo) = (t.0.join("claude"), t.0.join("repo"));
        assert_eq!(read(Some(&claude), Some(&repo)), ClaudeDefaults::default());

        write(&claude.join("settings.json"), r#"{"model":"opus[1m]","effortLevel":"xhigh"}"#);
        let user = read(Some(&claude), Some(&repo));
        assert_eq!((user.model.as_deref(), user.effort.as_deref()), (Some("opus[1m]"), Some("xhigh")));

        write(&repo.join(".claude/settings.json"), r#"{"model":"sonnet","effortLevel":""}"#);
        write(&repo.join(".claude/settings.local.json"), "not json");
        let both = read(Some(&claude), Some(&repo));
        assert_eq!((both.model.as_deref(), both.effort.as_deref()), (Some("sonnet"), Some("xhigh")));
        assert_eq!(read(Some(&claude), None).model.as_deref(), Some("opus[1m]"));
    }
}
