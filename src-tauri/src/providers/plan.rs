//! Text Nodal generates for imported tasks: `plan.md`, acceptance criteria extracted from
//! the description, and the closing comment posted to the provider. All pure, except
//! `write_plan`.

use std::path::{Path, PathBuf};

use crate::domain::{ExtKind, TaskStatus, Verdict};

use super::ExternalItem;

pub const PLANS_DIR: &str = "tasks";
pub const PLAN_FILE: &str = "plan.md";
/// Last line of every imported plan.
pub const FOOTER: &str = "Do not update the task manager: Nodal syncs the status.";

/// `<data_dir>/tasks/<id>/plan.md` (same path as the plan of local tasks).
pub fn plan_path(data_dir: &Path, task_id: &str) -> PathBuf {
    data_dir.join(PLANS_DIR).join(task_id).join(PLAN_FILE)
}

/// Plan of an imported task: title with identifier, URL, parent, description and
/// `## Subtasks` with the children (checked if already finished).
pub fn render_plan(item: &ExternalItem) -> String {
    let mut out = format!("# {} · {}\n\n{}\n", item.identifier, item.title.trim(), item.url);
    if let Some(p) = &item.parent {
        out.push_str(&format!("\nParent: [{}]({}) {}\n", p.identifier, p.url, p.title.trim()));
    }
    out.push('\n');
    match item.description_md.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => out.push_str(d),
        None => out.push_str("_No description._"),
    }
    out.push('\n');
    if !item.children.is_empty() {
        out.push_str("\n## Subtasks\n\n");
        for c in &item.children {
            let done = matches!(c.state.kind, ExtKind::Completed | ExtKind::Canceled);
            out.push_str(&format!(
                "- [{}] [{}]({}) {} ({})\n",
                if done { "x" } else { " " },
                c.identifier,
                c.url,
                c.title.trim(),
                c.state.name
            ));
        }
    }
    out.push_str("\n---\n\n");
    out.push_str(FOOTER);
    out.push('\n');
    out
}

/// Writes the plan if it changed. Returns `true` if it wrote.
pub fn write_plan(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

// ---------- Acceptance criteria ----------

fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' => 'u',
            other => other,
        })
        .collect()
}

fn is_criteria_title(text: &str) -> bool {
    let t = fold(text.trim().trim_matches(|c: char| c == '*' || c == '_' || c == ':' || c.is_whitespace()));
    t.contains("acceptance") || t.starts_with("criterio") || t.contains("done when") || t.contains("definition of done")
}

/// Level of a markdown heading (`## X` → 2).
fn heading_level(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&level) && t[level..].starts_with(' ') {
        Some((level, t[level..].trim()))
    } else {
        None
    }
}

/// Line that acts as a title without being a heading: `**Acceptance criteria**`, `Done when:`.
fn pseudo_heading(line: &str) -> Option<&str> {
    let t = line.trim();
    let bold = (t.starts_with("**") && t.trim_end_matches(':').ends_with("**") && t.len() > 4)
        || (t.starts_with("__") && t.trim_end_matches(':').ends_with("__") && t.len() > 4);
    let colon = t.ends_with(':') && t.len() <= 60 && list_item(t).is_none();
    (bold || colon).then_some(t)
}

/// Text of a list item (`- x`, `* x`, `+ x`, `1. x`, `1) x`), without checkbox.
fn list_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
        r
    } else {
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        t[digits..].strip_prefix(". ").or_else(|| t[digits..].strip_prefix(") "))?
    };
    let rest = rest.trim_start();
    let rest = ["[ ] ", "[x] ", "[X] "].iter().find_map(|cb| rest.strip_prefix(cb)).unwrap_or(rest);
    Some(rest.trim())
}

/// Criteria from the description's "Acceptance…" / "Criterios…" / "Done when…" section:
/// the list items up to the next title or paragraph. Empty if there is no section.
pub fn extract_acceptance(md: &str) -> Vec<String> {
    let lines: Vec<&str> = md.lines().collect();
    let Some((start, level)) = lines.iter().enumerate().find_map(|(i, l)| match heading_level(l) {
        Some((lvl, text)) if is_criteria_title(text) => Some((i, Some(lvl))),
        None => pseudo_heading(l).filter(|t| is_criteria_title(t)).map(|_| (i, None)),
        _ => None,
    }) else {
        return Vec::new();
    };

    let mut out: Vec<String> = Vec::new();
    for line in &lines[start + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((lvl, _)) = heading_level(line) {
            if level.is_none_or(|l| lvl <= l) || !out.is_empty() {
                break;
            }
            continue;
        }
        if let Some(item) = list_item(line) {
            if !item.is_empty() {
                out.push(item.to_string());
            }
            continue;
        }
        if pseudo_heading(line).is_some() && !out.is_empty() {
            break;
        }
        // Indented continuation of the previous item.
        if line.starts_with("  ") || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
                continue;
            }
        }
        // Paragraph: an intro if there were no items yet; end of the section if there were.
        if !out.is_empty() {
            break;
        }
    }
    out
}

// ---------- Closing comment ----------
// Built by the queue (`work::pump`) when a step finishes.

/// What goes into the comment Nodal leaves when a step finishes.
#[derive(Debug, Clone, Default)]
pub struct ClosingInfo<'a> {
    pub status: Option<TaskStatus>,
    /// Human-readable executor ("frontend-developer", "/plan-task", "Claude").
    pub executor: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub pr_url: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub verdict: Option<&'a Verdict>,
    /// Step notice (no report, stopped, no verdict…), in italics at the end.
    pub note: Option<&'a str>,
}

fn status_label(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Backlog => "Backlog",
        TaskStatus::Todo => "Todo",
        TaskStatus::InProgress => "In Progress",
        TaskStatus::InReview => "In Review",
        TaskStatus::Blocked => "Blocked",
        TaskStatus::Done => "Done",
        TaskStatus::Canceled => "Canceled",
    }
}

/// Closing comment in markdown: status, PR or branch, summary, unmet criteria and nits.
pub fn closing_comment(c: &ClosingInfo) -> String {
    let mut head = String::from("**Nodal**");
    if let Some(s) = c.status {
        head.push_str(&format!(" · {}", status_label(s)));
    }
    if let Some(e) = c.executor {
        head.push_str(&format!(" · {e}"));
    }
    let mut parts = vec![head];
    match (c.pr_url, c.branch) {
        (Some(pr), _) => parts.push(format!("PR: {pr}")),
        (None, Some(b)) => parts.push(format!("Branch: `{b}`")),
        _ => {}
    }
    let summary = c.summary.or(c.verdict.and_then(|v| v.summary.as_deref()));
    if let Some(s) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(s.to_string());
    }
    if let Some(v) = c.verdict {
        if !v.unmet.is_empty() {
            parts.push(format!("**Unmet criteria**\n{}", bullets(&v.unmet)));
        }
        if !v.nits.is_empty() {
            parts.push(format!("**Nits**\n{}", bullets(&v.nits)));
        }
    }
    if let Some(n) = c.note.map(str::trim).filter(|n| !n.is_empty()) {
        parts.push(format!("_{n}_"));
    }
    parts.join("\n\n")
}

fn bullets(items: &[String]) -> String {
    items.iter().map(|i| format!("- {}", i.trim())).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::domain::{ExternalState, Priority, ScopeRef};
    use crate::providers::{ChildItem, ItemRef};

    fn state(id: &str, name: &str, kind: ExtKind) -> ExternalState {
        ExternalState { id: id.into(), name: name.into(), kind, color: None }
    }

    pub fn item(n: u32, desc: Option<&str>) -> ExternalItem {
        ExternalItem {
            external_id: format!("uuid-{n}"),
            identifier: format!("ENG-{n}"),
            url: format!("https://linear.app/acme/issue/ENG-{n}/x"),
            title: format!("Issue {n}"),
            description_md: desc.map(Into::into),
            state: state("s-todo", "Todo", ExtKind::Unstarted),
            scopes: vec![ScopeRef { kind: "team".into(), id: "team-eng".into(), name: "Engineering".into() }],
            parent: None,
            children: Vec::new(),
            labels: Vec::new(),
            assignee: None,
            priority: Priority::None,
            updated_at: "2026-09-20T10:00:00.000Z".into(),
            created_at: Some("2026-09-01T10:00:00.000Z".into()),
            closed_at: None,
        }
    }

    #[test]
    fn plan_snapshot() {
        let mut it = item(142, Some("Change the site logo.\n\n## Acceptance criteria\n- New logo in the header\n"));
        it.title = "Website rebrand".into();
        it.parent = Some(ItemRef {
            external_id: "uuid-100".into(),
            identifier: "ENG-100".into(),
            title: "New brand".into(),
            url: "https://linear.app/acme/issue/ENG-100/new-brand".into(),
        });
        it.children = vec![
            ChildItem {
                external_id: "uuid-143".into(),
                identifier: "ENG-143".into(),
                title: "Header".into(),
                url: "https://linear.app/acme/issue/ENG-143/header".into(),
                state: state("s-done", "Done", ExtKind::Completed),
            },
            ChildItem {
                external_id: "uuid-144".into(),
                identifier: "ENG-144".into(),
                title: "Favicon".into(),
                url: "https://linear.app/acme/issue/ENG-144/favicon".into(),
                state: state("s-todo", "Todo", ExtKind::Unstarted),
            },
        ];
        let expected = "\
# ENG-142 · Website rebrand

https://linear.app/acme/issue/ENG-142/x

Parent: [ENG-100](https://linear.app/acme/issue/ENG-100/new-brand) New brand

Change the site logo.

## Acceptance criteria
- New logo in the header

## Subtasks

- [x] [ENG-143](https://linear.app/acme/issue/ENG-143/header) Header (Done)
- [ ] [ENG-144](https://linear.app/acme/issue/ENG-144/favicon) Favicon (Todo)

---

Do not update the task manager: Nodal syncs the status.
";
        assert_eq!(render_plan(&it), expected);
    }

    #[test]
    fn plan_without_description_or_children() {
        let p = render_plan(&item(7, None));
        assert!(p.contains("_No description._"));
        assert!(!p.contains("## Subtasks"));
        assert!(p.trim_end().ends_with(FOOTER));
    }

    #[test]
    fn write_plan_only_when_changed() {
        let dir = std::env::temp_dir().join(format!("nodal-plan-{}", crate::util::new_id('x', 1)));
        let path = plan_path(&dir, "t1");
        assert!(write_plan(&path, "a").unwrap());
        assert!(!write_plan(&path, "a").unwrap());
        assert!(write_plan(&path, "b").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn extracts_criteria_from_heading_section() {
        let md = "Intro.\n\n## Acceptance Criteria\n\nThe feature is done when:\n\n- [ ] Logo in the header\n- [x] Favicon\n  with transparent background\n1. Green tests\n\n## Notes\n- not a criterion\n";
        assert_eq!(
            extract_acceptance(md),
            vec!["Logo in the header", "Favicon with transparent background", "Green tests"]
        );
    }

    #[test]
    fn extracts_criteria_from_bold_or_colon_titles() {
        let md = "**Criterios de aceptación**\n* One\n* Two\n\nAnother paragraph.\n- three\n";
        assert_eq!(extract_acceptance(md), vec!["One", "Two"]);
        let md = "Context.\n\nDone when:\n- Deploy ok\n- Docs\n";
        assert_eq!(extract_acceptance(md), vec!["Deploy ok", "Docs"]);
        let md = "### Criterios\n- a\n#### Sub\n- b\n## End\n- c";
        assert_eq!(extract_acceptance(md), vec!["a"]);
    }

    #[test]
    fn no_section_no_criteria() {
        assert!(extract_acceptance("Just text.\n- some random list\n").is_empty());
        assert!(extract_acceptance("").is_empty());
        assert!(extract_acceptance("## Acceptance\n\nNothing in a list.").is_empty());
    }

    #[test]
    fn closing_comment_with_verdict() {
        let v = Verdict {
            pass: false,
            unmet: vec!["Favicon updated".into()],
            nits: vec!["Rename variable".into()],
            summary: Some("Favicon is missing.".into()),
        };
        let c = closing_comment(&ClosingInfo {
            status: Some(TaskStatus::Blocked),
            executor: Some("frontend-developer"),
            branch: Some("nodal/eng-142"),
            verdict: Some(&v),
            ..Default::default()
        });
        assert_eq!(
            c,
            "**Nodal** · Blocked · frontend-developer\n\nBranch: `nodal/eng-142`\n\nFavicon is missing.\n\n**Unmet criteria**\n- Favicon updated\n\n**Nits**\n- Rename variable"
        );
        let c = closing_comment(&ClosingInfo {
            status: Some(TaskStatus::InReview),
            pr_url: Some("https://example.com/pr/1"),
            branch: Some("b"),
            note: Some("Finished without a report."),
            ..Default::default()
        });
        assert!(c.contains("PR: https://example.com/pr/1") && !c.contains("Branch"));
        assert!(c.ends_with("_Finished without a report._"));
    }
}
