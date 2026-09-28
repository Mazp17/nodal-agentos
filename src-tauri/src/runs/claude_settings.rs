//! Claude Code's configured defaults (`model`, `effortLevel`) from its settings files, so the
//! UI can say what "default" means. Later layers win: `~/.claude/settings.json`, then the
//! repo's `.claude/settings.json` and `.claude/settings.local.json`. Managed (enterprise)
//! settings and env vars like `ANTHROPIC_MODEL` are not read.

use std::fs;
use std::path::Path;

use serde_json::Value;

/// Moved to `nodal_domain::model::chat`; re-exported so current uses don't break.
pub use nodal_domain::model::chat::ClaudeDefaults;

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
mod tests;
