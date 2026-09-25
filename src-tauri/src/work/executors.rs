//! Executor catalog: agents (`~/.claude/agents`, `<repo>/.claude/agents` and those of
//! enabled plugins), workflows (`runs::workflows`) and Claude.
//!
//! Plugins (verified on this machine with Claude Code 2.1.281):
//! - enabled: `enabledPlugins` (`"<plugin>@<marketplace>": true`) in
//!   `~/.claude/settings.json` and in `<repo>/.claude/settings{,.local}.json`;
//! - installed: `~/.claude/plugins/installed_plugins.json` (v2: `plugins` →
//!   `"<plugin>@<marketplace>"` → list of installs with `scope`, `projectPath` and
//!   `installPath`);
//! - agents: `<installPath>/agents/*.md`, which Claude Code names `<plugin>:<agent>`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::domain::{AgentSource, Executor};
use crate::runs::workflows::{self, WorkflowInfo};
use crate::util::clip_chars;

const DESCRIPTION_MAX: usize = 400;
const AGENT_FILE_MAX: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorInfo {
    pub executor: Executor,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
    pub manages_source: Option<String>,
    pub reviews: bool,
    pub path: Option<String>,
    pub source: Option<AgentSource>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
}

/// Name usable in `--agent <name>` (with `plugin:agent` for plugin agents).
pub fn is_valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

// ---------- YAML frontmatter (subset) ----------

fn unquote_double(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => {}
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'));
                }
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            c => out.push(c),
        }
    }
    None
}

fn unquote_single(s: &str) -> Option<String> {
    let inner = s.strip_prefix('\'')?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    None
}

fn scalar(raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with('"') {
        if let Some(s) = unquote_double(t) {
            return s;
        }
    }
    if t.starts_with('\'') {
        if let Some(s) = unquote_single(t) {
            return s;
        }
    }
    // Comment at the end of a plain scalar.
    t.split(" #").next().unwrap_or(t).trim().to_string()
}

fn split_list(raw: &str) -> Vec<String> {
    let t = raw.trim();
    let t = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(t);
    t.split(',').map(scalar).filter(|s| !s.is_empty()).collect()
}

/// Reads the leading `---` block. Supports plain and quoted scalars, `|`/`>` blocks and
/// lists (`[a, b]`, `a, b` or `- a`). Top-level keys only.
pub fn parse_frontmatter(src: &str) -> Option<Frontmatter> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut lines = src.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let body: Vec<&str> = lines.take_while(|l| l.trim_end() != "---").collect();
    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        i += 1;
        if line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else { continue };
        let key = key.trim().to_string();
        let rest = rest.trim();
        let indented = |l: &&&str| l.starts_with(' ') || l.starts_with('\t') || l.trim().is_empty();
        if rest.starts_with('|') || rest.starts_with('>') {
            let folded = rest.starts_with('>');
            let block: Vec<&str> = body[i..].iter().take_while(indented).copied().collect();
            i += block.len();
            let text: Vec<&str> = block.iter().map(|l| l.trim()).collect();
            let joined = if folded { text.join(" ") } else { text.join("\n") };
            map.insert(key, Value::String(joined.trim().to_string()));
        } else if rest.is_empty() {
            let block: Vec<&str> = body[i..].iter().take_while(indented).copied().collect();
            i += block.len();
            let items: Vec<Value> = block
                .iter()
                .filter_map(|l| l.trim().strip_prefix("- "))
                .map(|v| Value::String(scalar(v)))
                .collect();
            map.insert(key, Value::Array(items));
        } else if rest.starts_with('[') {
            map.insert(key, Value::Array(split_list(rest).into_iter().map(Value::String).collect()));
        } else {
            map.insert(key, Value::String(scalar(rest)));
        }
    }
    let text = |k: &str| map.get(k).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
    let tools = match map.get("tools") {
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).map(str::to_string).collect()),
        Some(Value::String(s)) => Some(split_list(s)),
        _ => None,
    };
    Some(Frontmatter { name: text("name"), description: text("description"), tools })
}

/// Description for display: the first paragraph, without the `<example>`s, on one line.
pub fn short_description(d: &str) -> String {
    let d = d.replace("\\n", "\n");
    let first = d.split("\n\n").next().unwrap_or(&d);
    let first = first.split("<example>").next().unwrap_or(first);
    clip_chars(&first.split_whitespace().collect::<Vec<_>>().join(" "), DESCRIPTION_MAX)
}

// ---------- Agents ----------

#[derive(Debug, Clone, PartialEq)]
pub struct AgentDef {
    /// Name for `--agent` (`plugin:agent` for plugin agents).
    pub name: String,
    pub source: AgentSource,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
    pub path: PathBuf,
}

fn read_agents_dir(dir: &Path, source: AgentSource, prefix: Option<&str>) -> Vec<AgentDef> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| e == "md")
                && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= AGENT_FILE_MAX))
        .filter_map(|path| {
            let fm = std::fs::read_to_string(&path).ok().and_then(|s| parse_frontmatter(&s))?;
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let base = fm.name.clone().unwrap_or(stem);
            let name = match prefix {
                Some(p) => format!("{p}:{base}"),
                None => base,
            };
            is_valid_agent_name(&name).then(|| AgentDef {
                name,
                source,
                description: fm.description.as_deref().map(short_description),
                tools: fm.tools,
                path,
            })
        })
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// `enabledPlugins` of a settings.json: the `true` ones.
fn enabled_in(settings: &Path) -> Vec<String> {
    let Some(Value::Object(map)) = read_json(settings).and_then(|v| v.get("enabledPlugins").cloned()) else {
        return Vec::new();
    };
    map.into_iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k).collect()
}

/// Enabled plugins with their install folder: `(plugin name, installPath)`.
/// An install with `scope: "project"` only counts for its `projectPath`.
pub fn enabled_plugins(claude_dir: &Path, repo: Option<&Path>) -> Vec<(String, PathBuf)> {
    let mut enabled = enabled_in(&claude_dir.join("settings.json"));
    if let Some(r) = repo {
        enabled.extend(enabled_in(&r.join(".claude").join("settings.json")));
        enabled.extend(enabled_in(&r.join(".claude").join("settings.local.json")));
    }
    enabled.sort();
    enabled.dedup();
    let Some(installed) = read_json(&claude_dir.join("plugins").join("installed_plugins.json")) else {
        return Vec::new();
    };
    let Some(Value::Object(plugins)) = installed.get("plugins") else { return Vec::new() };
    let mut out = Vec::new();
    for id in enabled {
        let Some(Value::Array(installs)) = plugins.get(&id) else { continue };
        let pick = installs.iter().find(|i| {
            match i.get("scope").and_then(Value::as_str) {
                Some("project") | Some("local") => repo.is_some_and(|r| {
                    i.get("projectPath").and_then(Value::as_str).is_some_and(|p| Path::new(p) == r)
                }),
                _ => true,
            }
        });
        if let Some(path) = pick.and_then(|i| i.get("installPath")).and_then(Value::as_str) {
            let name = id.split('@').next().unwrap_or(&id).to_string();
            out.push((name, PathBuf::from(path)));
        }
    }
    out
}

/// Agents visible to the repo. A repo agent overrides the user's one with the same name.
pub fn list_agents(claude_dir: Option<&Path>, repo: Option<&Path>) -> Vec<AgentDef> {
    let mut out: Vec<AgentDef> = Vec::new();
    let user = claude_dir.map(|d| read_agents_dir(&d.join("agents"), AgentSource::User, None)).unwrap_or_default();
    let in_repo = repo
        .map(|r| read_agents_dir(&r.join(".claude").join("agents"), AgentSource::Repo, None))
        .unwrap_or_default();
    for a in user.into_iter().chain(in_repo) {
        match out.iter_mut().find(|x| x.name == a.name) {
            Some(existing) => *existing = a,
            None => out.push(a),
        }
    }
    if let Some(d) = claude_dir {
        for (plugin, install) in enabled_plugins(d, repo) {
            out.extend(read_agents_dir(&install.join("agents"), AgentSource::Plugin, Some(&plugin)));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ---------- Catalog ----------

pub fn workflow_info(w: &WorkflowInfo) -> ExecutorInfo {
    ExecutorInfo {
        executor: Executor::Workflow { name: w.name.clone() },
        description: w.description.clone(),
        tools: None,
        manages_source: w.manages_source.clone(),
        reviews: w.reviews,
        path: Some(w.path.clone()),
        source: None,
    }
}

/// Claude first, then agents and workflows (each group by name).
pub fn catalog(claude_dir: Option<&Path>, repo: Option<&Path>) -> Vec<ExecutorInfo> {
    let mut out = vec![ExecutorInfo {
        executor: Executor::Claude,
        description: Some("A regular Claude Code session with the task as the prompt.".into()),
        tools: None,
        manages_source: None,
        reviews: false,
        path: None,
        source: None,
    }];
    out.extend(list_agents(claude_dir, repo).into_iter().map(|a| ExecutorInfo {
        executor: Executor::Agent { name: a.name, source: a.source },
        description: a.description,
        tools: a.tools,
        manages_source: None,
        reviews: false,
        path: Some(a.path.to_string_lossy().into_owned()),
        source: Some(a.source),
    }));
    let wf_dir = claude_dir.map(|d| d.join("workflows"));
    out.extend(workflows::list_from(wf_dir.as_deref(), repo).iter().map(workflow_info));
    out
}

/// Workflow meta (`managesSource`, `reviews`) by name, if it's in the catalog.
pub fn find_workflow(claude_dir: Option<&Path>, repo: Option<&Path>, name: &str) -> Option<ExecutorInfo> {
    let wf_dir = claude_dir.map(|d| d.join("workflows"));
    workflows::list_from(wf_dir.as_deref(), repo).iter().find(|w| w.name == name).map(workflow_info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::paths::tests::TempDir;

    #[test]
    fn frontmatter_real_shapes() {
        // Like `~/.claude/agents/frontend-developer.md`: double-quoted description with \n.
        let fe = "---\nname: frontend-developer\ndescription: \"Use when building apps. Specifically:\\n\\n<example>\\nContext: x\\n</example>\"\ntools: Read, Write, Edit, Bash, Glob, Grep\n---\n\nYou are a senior frontend developer.\n";
        let f = parse_frontmatter(fe).unwrap();
        assert_eq!(f.name.as_deref(), Some("frontend-developer"));
        assert_eq!(f.tools.as_deref().unwrap(), ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]);
        assert_eq!(short_description(&f.description.unwrap()), "Use when building apps. Specifically:");
        // Like `code-reviewer.md`: a literal `\\n` inside double quotes.
        let cr = "---\nname: code-reviewer\ndescription: \"Reviews code.\\\\n\\\\n<example>\\\\nx\"\ntools: Read, Grep\n---\n";
        let f = parse_frontmatter(cr).unwrap();
        assert_eq!(short_description(&f.description.unwrap()), "Reviews code.");
        // YAML list, folded block, single quotes.
        let other = "---\nname: 'db-helper'\ndescription: >\n  Helps with\n  databases\ntools:\n  - Read\n  - Bash\nmodel: sonnet\n---\n";
        let f = parse_frontmatter(other).unwrap();
        assert_eq!(f.name.as_deref(), Some("db-helper"));
        assert_eq!(f.description.as_deref(), Some("Helps with databases"));
        assert_eq!(f.tools.unwrap(), ["Read", "Bash"]);
        assert_eq!(parse_frontmatter("no frontmatter"), None);
        let f = parse_frontmatter("---\nname: x\ntools: [Read, \"Write\"]\n---").unwrap();
        assert_eq!(f.tools.unwrap(), ["Read", "Write"]);
        assert_eq!(parse_frontmatter("---\nname: y\n---\n").unwrap().tools, None);
    }

    #[test]
    fn agent_names() {
        assert!(is_valid_agent_name("code-reviewer"));
        assert!(is_valid_agent_name("pr-review-toolkit:code-reviewer"));
        assert!(!is_valid_agent_name("-x"));
        assert!(!is_valid_agent_name("a b"));
        assert!(!is_valid_agent_name(""));
    }

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    #[test]
    fn catalog_merges_user_repo_plugins_and_workflows() {
        let t = TempDir::new("executors");
        let claude = t.0.join("claude");
        let repo = t.0.join("repo");
        write(&claude.join("agents/code-reviewer.md"), "---\nname: code-reviewer\ndescription: user version\ntools: Read\n---\n");
        write(&claude.join("agents/frontend-developer.md"), "---\nname: frontend-developer\ndescription: fe\n---\n");
        write(&claude.join("agents/.hidden.md"), "---\nname: hidden\n---\n");
        write(&claude.join("agents/notes.txt"), "x");
        write(&repo.join(".claude/agents/code-reviewer.md"), "---\nname: code-reviewer\ndescription: repo version\n---\n");
        // Enabled plugin with agents, another installed but disabled, one from another project.
        let inst = t.0.join("cache/toolkit/1.0");
        write(&inst.join("agents/security.md"), "---\nname: security\ndescription: sec\n---\n");
        let off = t.0.join("cache/off/1.0");
        write(&off.join("agents/nope.md"), "---\nname: nope\n---\n");
        let other = t.0.join("cache/proj/1.0");
        write(&other.join("agents/elsewhere.md"), "---\nname: elsewhere\n---\n");
        write(
            &claude.join("settings.json"),
            r#"{"enabledPlugins": {"toolkit@market": true, "off@market": false, "proj@market": true}}"#,
        );
        let installed = serde_json::json!({
            "version": 2,
            "plugins": {
                "toolkit@market": [{"scope": "user", "installPath": inst}],
                "off@market": [{"scope": "user", "installPath": off}],
                "proj@market": [{"scope": "project", "projectPath": "/somewhere/else", "installPath": other}]
            }
        });
        write(&claude.join("plugins/installed_plugins.json"), &installed.to_string());
        write(&claude.join("workflows/plan-task.js"), "export const meta = { name: 'plan-task', reviews: true }\n");
        write(&claude.join("workflows/linear-issue.js"), "export const meta = { name: 'linear-issue', managesSource: 'linear', reviews: true }\n");

        let list = catalog(Some(&claude), Some(&repo));
        let names: Vec<String> = list
            .iter()
            .map(|e| match &e.executor {
                Executor::Claude => "claude".to_string(),
                Executor::Agent { name, source } => format!("agent:{name}:{}", source.as_str()),
                Executor::Workflow { name } => format!("wf:{name}"),
            })
            .collect();
        assert_eq!(
            names,
            [
                "claude",
                "agent:code-reviewer:repo",
                "agent:frontend-developer:user",
                "agent:toolkit:security:plugin",
                "wf:linear-issue",
                "wf:plan-task"
            ]
        );
        assert_eq!(list[1].description.as_deref(), Some("repo version"));
        let li = &list[4];
        assert_eq!(li.manages_source.as_deref(), Some("linear"));
        assert!(li.reviews);
        assert!(find_workflow(Some(&claude), Some(&repo), "plan-task").unwrap().reviews);
        assert!(find_workflow(Some(&claude), None, "nope").is_none());
        // No repo: only the user's agents and user-scoped plugins.
        let no_repo = catalog(Some(&claude), None);
        assert!(no_repo.iter().any(|e| e.description.as_deref() == Some("user version")));
        assert!(!no_repo.iter().any(|e| matches!(&e.executor, Executor::Agent { name, .. } if name.contains("elsewhere"))));
    }

    /// Against this machine's `~/.claude`: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_catalog() {
        let dir = crate::runs::claude_fs::claude_config_dir().unwrap();
        for e in catalog(Some(&dir), None) {
            eprintln!("{:?} · {:?} · {:?}", e.executor, e.tools, e.description.map(|d| clip_chars(&d, 60)));
        }
    }
}
