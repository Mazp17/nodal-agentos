//! Workflow catalog: `~/.claude/workflows/*.js` and `<repo>/.claude/workflows/*.js`.
//!
//! The `export const meta = {...}` is JS, not JSON, and isn't executed: the literal is
//! scanned respecting strings and comments, and the top-level `name`, `description`,
//! `whenToUse`, `managesSource` (string) and `reviews` (boolean) are read. If something
//! can't be understood, the workflow is still listed under its file name.

use std::path::{Path, PathBuf};

use super::claude_fs::js_string_prop;

/// Moved to `nodal_domain::model::executors`; re-exported so current uses don't break.
/// `is_valid_workflow_name` has no callers left through this path (`board::validate` now
/// imports it straight from `nodal_domain`), but stays reachable here until W4.
#[allow(unused_imports)]
pub use nodal_domain::model::executors::{is_valid_workflow_name, WorkflowInfo, WorkflowSource};

#[derive(Debug, Default, PartialEq)]
pub struct Meta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub when_to_use: Option<String>,
    pub manages_source: Option<String>,
    pub reviews: Option<bool>,
}

/// Walks `s` and returns a copy of the same byte length where comments and everything
/// nested at depth > 0 are blanked out. If `stop_at_close`, stops at the `}` that closes
/// level 0 and returns its index.
fn scan_top_level(s: &str, stop_at_close: bool) -> (String, Option<usize>) {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut i = 0;
    let blank = |out: &mut String, c: char| {
        // Same byte length so that indices into `out` work on `s`.
        for _ in 0..c.len_utf8() {
            out.push(' ');
        }
    };
    while i < bytes.len() {
        let c = s[i..].chars().next().unwrap_or(' ');
        let len = c.len_utf8();
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c as u32 == q as u32 {
                quote = None;
            }
            if depth == 0 { out.push(c) } else { blank(&mut out, c) }
            i += len;
            continue;
        }
        // Comments: blanked out (they may contain stray quotes).
        if s[i..].starts_with("//") {
            let end = s[i..].find('\n').map_or(s.len(), |n| i + n);
            s[i..end].chars().for_each(|c| blank(&mut out, c));
            i = end;
            continue;
        }
        if s[i..].starts_with("/*") {
            let end = s[i + 2..].find("*/").map_or(s.len(), |n| i + 2 + n + 2);
            s[i..end].chars().for_each(|c| blank(&mut out, c));
            i = end;
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                quote = Some(c as u8);
                if depth == 0 { out.push(c) } else { blank(&mut out, c) }
            }
            '{' | '[' | '(' => {
                depth += 1;
                blank(&mut out, c);
            }
            '}' | ']' | ')' => {
                if depth == 0 {
                    if stop_at_close && c == '}' {
                        return (out, Some(i));
                    }
                } else {
                    depth -= 1;
                }
                blank(&mut out, c);
            }
            _ if depth == 0 => out.push(c),
            _ => blank(&mut out, c),
        }
        i += len;
    }
    (out, None)
}

/// String value of `key` at the top level, with the key unquoted or quoted.
fn top_level_prop(flat: &str, key: &str) -> Option<String> {
    [key.to_string(), format!("\"{key}\""), format!("'{key}'")]
        .iter()
        .find_map(|k| js_string_prop(flat, k))
        .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|v| !v.is_empty())
}

/// Literal boolean value (`true`/`false`) of `key` at the top level.
fn top_level_bool(flat: &str, key: &str) -> Option<bool> {
    for k in [key.to_string(), format!("\"{key}\""), format!("'{key}'")] {
        let mut search = flat;
        while let Some(pos) = search.find(&k) {
            let before_ok = search[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'));
            let rest = search[pos + k.len()..].trim_start();
            if before_ok {
                if let Some(v) = rest.strip_prefix(':').map(str::trim_start) {
                    let word: String = v.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                    match word.as_str() {
                        "true" => return Some(true),
                        "false" => return Some(false),
                        _ => {}
                    }
                }
            }
            search = &search[pos + k.len()..];
        }
    }
    None
}

/// Reads `export const meta = { ... }`. `None` if there's no recognizable meta.
pub fn parse_meta(src: &str) -> Option<Meta> {
    let start = src.find("export const meta")?;
    let after = &src[start + "export const meta".len()..];
    let eq = after.find('=')?;
    if !after[..eq].trim().is_empty() && !after[..eq].trim_start().starts_with(':') {
        return None;
    }
    let rest = &after[eq + 1..];
    let open = rest.find('{')?;
    if !rest[..open].trim().is_empty() {
        return None;
    }
    let body = &rest[open + 1..];
    let (flat, close) = scan_top_level(body, true);
    close?;
    let meta = Meta {
        name: top_level_prop(&flat, "name"),
        description: top_level_prop(&flat, "description"),
        when_to_use: top_level_prop(&flat, "whenToUse"),
        manages_source: top_level_prop(&flat, "managesSource"),
        reviews: top_level_bool(&flat, "reviews"),
    };
    Some(meta)
}

/// Backup files (`x.js.bak`, `x.js.bak-2026…`, `x.bak.js`) and hidden ones don't count.
fn is_workflow_file(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
    p.is_file() && name.ends_with(".js") && !name.starts_with('.') && !name.contains(".bak")
}

fn read_dir_workflows(dir: &Path, source: WorkflowSource) -> Vec<WorkflowInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_workflow_file(p)).collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let meta = std::fs::read_to_string(&path).ok().and_then(|s| parse_meta(&s)).unwrap_or_default();
            WorkflowInfo {
                name: meta.name.unwrap_or(stem),
                description: meta.description,
                when_to_use: meta.when_to_use,
                manages_source: meta.manages_source,
                reviews: meta.reviews.unwrap_or(false),
                source,
                path: path.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

/// Catalog sorted by name. A repo workflow overrides the user's one with the same name.
pub fn list_from(user_dir: Option<&Path>, repo: Option<&Path>) -> Vec<WorkflowInfo> {
    let mut out: Vec<WorkflowInfo> = Vec::new();
    let user = user_dir.map(|d| read_dir_workflows(d, WorkflowSource::User)).unwrap_or_default();
    let repo = repo
        .map(|r| read_dir_workflows(&r.join(".claude").join("workflows"), WorkflowSource::Repo))
        .unwrap_or_default();
    for wf in user.into_iter().chain(repo) {
        match out.iter_mut().find(|w| w.name == wf.name) {
            Some(existing) => *existing = wf,
            None => out.push(wf),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests;
