//! Workflow catalog: `~/.claude/workflows/*.js` and `<repo>/.claude/workflows/*.js`.
//!
//! The `export const meta = {...}` is JS, not JSON, and isn't executed: the literal is
//! scanned respecting strings and comments, and the top-level `name`, `description`,
//! `whenToUse`, `managesSource` (string) and `reviews` (boolean) are read. If something
//! can't be understood, the workflow is still listed under its file name.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::claude_fs::js_string_prop;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSource {
    User,
    Repo,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub name: String,
    pub description: Option<String>,
    pub when_to_use: Option<String>,
    /// Provider the workflow syncs on its own (`"linear"`): the app doesn't push status to it
    /// or comment.
    pub manages_source: Option<String>,
    /// The workflow already reviews (its result is the verdict): Nodal's gate is skipped.
    pub reviews: bool,
    pub source: WorkflowSource,
    pub path: String,
}

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

/// Name usable in `/<workflow> ...`: no spaces or anything `claude` would read as an option.
pub fn is_valid_workflow_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runs/fixtures/workflows")
    }

    #[test]
    fn parses_real_linear_issue_meta() {
        let src = std::fs::read_to_string(fixtures().join("user/linear-issue.js")).unwrap();
        let m = parse_meta(&src).unwrap();
        assert_eq!(m.name.as_deref(), Some("linear-issue"));
        assert!(m.description.unwrap().starts_with("Lleva una issue de Linear"));
        assert!(m.when_to_use.unwrap().contains("Nunca mergea."));
    }

    #[test]
    fn parses_real_demo_board_meta() {
        let src = std::fs::read_to_string(fixtures().join("user/demo-board.js")).unwrap();
        let m = parse_meta(&src).unwrap();
        assert_eq!(m.name.as_deref(), Some("demo-board"));
        assert!(m.description.unwrap().starts_with("Workflow de juguete"));
        // It's on the line after `whenToUse:` and has double quotes inside.
        assert!(m.when_to_use.unwrap().contains(r#"args: "volcanes""#));
    }

    #[test]
    fn nested_name_and_comments_do_not_confuse() {
        let src = "// meta = { name: 'fake' }\nexport const meta = {\n  /* name: 'other' */\n  phases: [{ name: 'phase' }],\n  \"name\": \"real\",\n  description: 'with \\'escape\\' and naïveté',\n}\n";
        let m = parse_meta(src).unwrap();
        assert_eq!(m.name.as_deref(), Some("real"));
        assert_eq!(m.description.as_deref(), Some("with 'escape' and naïveté"));
        assert_eq!(m.when_to_use, None);
    }

    #[test]
    fn manages_source_and_reviews() {
        let src = "export const meta = {\n  name: 'x',\n  managesSource: 'linear',\n  reviews: true,\n  phases: [{ reviews: false }],\n}\n";
        let m = parse_meta(src).unwrap();
        assert_eq!(m.manages_source.as_deref(), Some("linear"));
        assert_eq!(m.reviews, Some(true));
        let m = parse_meta("export const meta = { name: 'y', 'reviews': false, noreviews: true }").unwrap();
        assert_eq!(m.reviews, Some(false));
        assert_eq!(m.manages_source, None);
        // Only the nested one: doesn't count.
        let m = parse_meta("export const meta = { name: 'z', opts: { reviews: true } }").unwrap();
        assert_eq!(m.reviews, None);
        // The real fixtures don't declare it yet.
        let src = std::fs::read_to_string(fixtures().join("user/linear-issue.js")).unwrap();
        assert_eq!(parse_meta(&src).unwrap().reviews, None);
    }

    #[test]
    fn broken_or_missing_meta() {
        assert_eq!(parse_meta("const x = 1"), None);
        assert_eq!(parse_meta("export const meta = { name: 'unclosed'"), None);
        assert_eq!(parse_meta("export const meta = loadMeta()"), None);
        let m = parse_meta("export const meta = { name: someVariable }").unwrap();
        assert_eq!(m.name, None);
    }

    #[test]
    fn catalog_ignores_backups_and_repo_overrides_user() {
        let root = fixtures();
        let list = list_from(Some(&root.join("user")), Some(&root.join("repo")));
        let names: Vec<_> = list.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["demo-board", "linear-issue", "sin-meta"]);
        let li = &list[1];
        assert_eq!(li.source, WorkflowSource::Repo);
        assert_eq!(li.description.as_deref(), Some("Variante del repo con \"comillas\" escapadas"));
        assert_eq!(list[2].description, None);

        let only_user = list_from(Some(&root.join("user")), None);
        assert_eq!(only_user.len(), 2);
        assert!(only_user.iter().all(|w| w.source == WorkflowSource::User));
        assert!(list_from(Some(Path::new("/no/such/dir")), None).is_empty());
    }

    /// Against this machine's `~/.claude/workflows`: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_user_catalog() {
        let dir = crate::runs::claude_fs::claude_config_dir().unwrap().join("workflows");
        let list = list_from(Some(&dir), None);
        for w in &list {
            eprintln!("{} · {:?}", w.name, w.description);
        }
        assert!(list.iter().any(|w| w.name == "linear-issue" && w.description.is_some()));
        assert!(list.iter().all(|w| !w.path.contains(".bak")));
    }

    #[test]
    fn workflow_names() {
        assert!(is_valid_workflow_name("linear-issue"));
        assert!(is_valid_workflow_name("plugin:flow_2"));
        assert!(!is_valid_workflow_name("-x"));
        assert!(!is_valid_workflow_name("a b"));
        assert!(!is_valid_workflow_name(""));
    }
}
