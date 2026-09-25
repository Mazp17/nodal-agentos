//! Input validation (projects, repos, tasks, executors, settings). Anything that touches
//! disk is blocking: it's called from `blocking`.

use std::path::{Path, PathBuf};

use crate::domain::{Executor, MAX_CONCURRENCY};
use crate::runs::workflows::is_valid_workflow_name;

use super::executors::is_valid_agent_name;

pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_NAME_CHARS: usize = 80;
/// Cap for text plans and for reading a plan (file or text).
pub const MAX_PLAN_BYTES: u64 = 512 * 1024;
const MAX_LABELS: usize = 20;
const MAX_LABEL_CHARS: usize = 40;
const MAX_CRITERIA: usize = 50;
const MAX_CRITERION_CHARS: usize = 1000;
const MAX_EXTRA_CHARS: usize = 8000;

/// Editors allowed for "Open in editor" (binary looked up in PATH).
pub const EDITORS: [&str; 9] = ["code", "code-insiders", "cursor", "windsurf", "zed", "subl", "idea", "webstorm", "fleet"];

/// Default project palette (when no color is sent).
pub const PALETTE: [&str; 8] = [
    "oklch(0.74 0.15 55)",
    "oklch(0.72 0.13 250)",
    "oklch(0.74 0.12 150)",
    "oklch(0.70 0.15 320)",
    "oklch(0.78 0.13 88)",
    "oklch(0.68 0.16 25)",
    "oklch(0.72 0.10 200)",
    "oklch(0.70 0.12 290)",
];

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn title(title: &str) -> Result<String, String> {
    let t = one_line(title);
    if t.is_empty() {
        return Err("The title is empty.".into());
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("The title is too long (max {MAX_TITLE_CHARS} characters)."));
    }
    Ok(t)
}

pub fn name(name: &str, what: &str) -> Result<String, String> {
    let t = one_line(name);
    if t.is_empty() {
        return Err(format!("The {what} name is empty."));
    }
    if t.chars().count() > MAX_NAME_CHARS {
        return Err(format!("The {what} name is too long (max {MAX_NAME_CHARS} characters)."));
    }
    Ok(t)
}

/// 2 to 6 uppercase letters or digits, starting with a letter (`PAY`, `WEB2`). Uppercased.
pub fn project_key(key: &str) -> Result<String, String> {
    let k = key.trim().to_ascii_uppercase();
    let ok = (2..=6).contains(&k.len())
        && k.starts_with(|c: char| c.is_ascii_uppercase())
        && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if ok {
        Ok(k)
    } else {
        Err(format!("Invalid key \"{}\": use 2-6 letters or digits, starting with a letter.", key.trim()))
    }
}

/// `#rgb`/`#rrggbb` or `oklch(...)` with numbers.
pub fn color(c: &str) -> Result<String, String> {
    let c = c.trim();
    let hex = c.strip_prefix('#').is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|x| x.is_ascii_hexdigit()));
    let oklch = c
        .strip_prefix("oklch(")
        .and_then(|r| r.strip_suffix(')'))
        .is_some_and(|inner| {
            !inner.trim().is_empty()
                && inner.len() <= 40
                && inner.chars().all(|x| x.is_ascii_digit() || matches!(x, '.' | ' ' | '%' | '/'))
        });
    if hex || oklch {
        Ok(c.to_string())
    } else {
        Err(format!("Invalid color \"{c}\": use #rrggbb or oklch(...)."))
    }
}

pub fn plan_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("The plan is empty.".into());
    }
    if text.len() as u64 > MAX_PLAN_BYTES {
        return Err(format!("The plan is too large (max {} KB).", MAX_PLAN_BYTES / 1024));
    }
    Ok(())
}

/// Plan file: `.md`, exists, is a file and stays inside `repo` after resolving symlinks and
/// `..` (canonicalize). `path` can be absolute or relative to the repo.
pub fn plan_file(repo: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err("Pick a plan file.".into());
    }
    let repo = repo.canonicalize().map_err(|_| format!("The repository folder doesn't exist: {}", repo.display()))?;
    let p = Path::new(raw);
    let joined = if p.is_absolute() { p.to_path_buf() } else { repo.join(p) };
    let canon = joined.canonicalize().map_err(|_| format!("The plan file doesn't exist: {raw}"))?;
    if !canon.starts_with(&repo) {
        return Err(format!("The plan file must be inside the repository ({}).", repo.display()));
    }
    if canon.strip_prefix(&repo).is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == ".git")) {
        return Err("The plan file can't be inside .git.".into());
    }
    let is_md = canon.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if !is_md {
        return Err(format!("The plan file must be a Markdown (.md) file: {raw}"));
    }
    let meta = std::fs::metadata(&canon).map_err(|e| format!("Couldn't read {raw}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("The plan path is not a file: {raw}"));
    }
    if meta.len() > MAX_PLAN_BYTES {
        return Err(format!("The plan file is too large (max {} KB).", MAX_PLAN_BYTES / 1024));
    }
    Ok(canon)
}

/// No empty or repeated ones (case-insensitive), each on a single line.
pub fn labels(labels: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for l in labels {
        let l = one_line(l);
        if l.is_empty() || out.iter().any(|x| x.eq_ignore_ascii_case(&l)) {
            continue;
        }
        if l.chars().count() > MAX_LABEL_CHARS {
            return Err(format!("The label \"{l}\" is too long (max {MAX_LABEL_CHARS} characters)."));
        }
        out.push(l);
    }
    if out.len() > MAX_LABELS {
        return Err(format!("Too many labels (max {MAX_LABELS})."));
    }
    Ok(out)
}

/// Acceptance criteria: no empty ones, with line breaks collapsed.
pub fn acceptance(items: &[String]) -> Result<Vec<String>, String> {
    let out: Vec<String> = items.iter().map(|s| one_line(s)).filter(|s| !s.is_empty()).collect();
    if out.len() > MAX_CRITERIA {
        return Err(format!("Too many acceptance criteria (max {MAX_CRITERIA})."));
    }
    if out.iter().any(|s| s.chars().count() > MAX_CRITERION_CHARS) {
        return Err(format!("An acceptance criterion is too long (max {MAX_CRITERION_CHARS} characters)."));
    }
    Ok(out)
}

pub fn executor(e: &Executor) -> Result<(), String> {
    match e {
        Executor::Agent { name, .. } if !is_valid_agent_name(name) => Err(format!("Invalid agent name \"{name}\".")),
        Executor::Workflow { name } if !is_valid_workflow_name(name) => {
            Err(format!("Invalid workflow name \"{name}\"."))
        }
        _ => Ok(()),
    }
}

pub fn reviewer(name: &str) -> Result<String, String> {
    let n = name.trim();
    if is_valid_agent_name(n) {
        Ok(n.to_string())
    } else {
        Err(format!("Invalid reviewer agent name \"{n}\"."))
    }
}

pub fn editor(e: &str) -> Result<String, String> {
    let e = e.trim();
    if EDITORS.contains(&e) {
        Ok(e.to_string())
    } else {
        Err(format!("Unknown editor \"{e}\": use one of {}.", EDITORS.join(", ")))
    }
}

pub fn concurrency(n: u32) -> Result<u32, String> {
    if (1..=MAX_CONCURRENCY).contains(&n) {
        Ok(n)
    } else {
        Err(format!("Concurrency must be between 1 and {MAX_CONCURRENCY}."))
    }
}

const MAX_DESCRIPTION_CHARS: usize = 2000;

/// Project description: trimmed; empty → `None`.
pub fn description(s: Option<&str>) -> Result<Option<String>, String> {
    let Some(t) = s.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    if t.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is too long (max {MAX_DESCRIPTION_CHARS} characters)."));
    }
    Ok(Some(t.to_string()))
}

pub fn extra_instructions(s: Option<&str>) -> Result<Option<String>, String> {
    let Some(t) = s.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    if t.chars().count() > MAX_EXTRA_CHARS {
        return Err(format!("The extra instructions are too long (max {MAX_EXTRA_CHARS} characters)."));
    }
    Ok(Some(t.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::paths::tests::TempDir;

    /// Two sibling folders: `repo` and `outside`.
    fn setup(name: &str) -> (TempDir, PathBuf, PathBuf) {
        let t = TempDir::new(name);
        let repo = t.0.join("repo");
        let outside = t.0.join("outside");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(repo.join("docs/plan.md"), "# Plan").unwrap();
        std::fs::write(repo.join("docs/notes.txt"), "x").unwrap();
        std::fs::write(outside.join("evil.md"), "# Evil").unwrap();
        (t, repo, outside)
    }

    #[test]
    fn plan_file_rules() {
        let (_g, repo, outside) = setup("plan");
        let ok = repo.join("docs/plan.md");
        assert_eq!(plan_file(&repo, ok.to_str().unwrap()).unwrap(), ok);
        assert_eq!(plan_file(&repo, "docs/plan.md").unwrap(), ok);
        assert_eq!(plan_file(&repo, "./docs/../docs/plan.md").unwrap(), ok);
        assert!(plan_file(&repo, "../outside/evil.md").unwrap_err().contains("inside"));
        assert!(plan_file(&repo, outside.join("evil.md").to_str().unwrap()).unwrap_err().contains("inside"));
        assert!(plan_file(&repo, "docs/missing.md").unwrap_err().contains("doesn't exist"));
        assert!(plan_file(&repo, "docs/notes.txt").unwrap_err().contains(".md"));
        assert!(plan_file(&repo, "").is_err());
        std::fs::write(repo.join(".git/x.md"), "#").unwrap();
        assert!(plan_file(&repo, ".git/x.md").unwrap_err().contains(".git"));
    }

    #[cfg(unix)]
    #[test]
    fn plan_file_symlink_escaping_repo_is_rejected() {
        let (_g, repo, outside) = setup("symlink");
        std::os::unix::fs::symlink(outside.join("evil.md"), repo.join("docs/link.md")).unwrap();
        assert!(plan_file(&repo, "docs/link.md").unwrap_err().contains("inside"));
        std::fs::create_dir_all(repo.join("dir.md")).unwrap();
        assert!(plan_file(&repo, "dir.md").unwrap_err().contains("not a file"));
    }

    #[test]
    fn simple_fields() {
        assert_eq!(title("  Fix   the\n bug ").unwrap(), "Fix the bug");
        assert!(title(" \n ").is_err());
        assert!(title(&"x".repeat(MAX_TITLE_CHARS + 1)).is_err());
        assert!(plan_text("  ").is_err());
        assert!(plan_text("# ok").is_ok());
        assert_eq!(project_key(" pay ").unwrap(), "PAY");
        assert_eq!(project_key("web2").unwrap(), "WEB2");
        for bad in ["P", "PAYMENTS", "2PAY", "PA-Y", ""] {
            assert!(project_key(bad).is_err(), "{bad}");
        }
        assert!(color("#d98c3f").is_ok() && color("#abc").is_ok() && color(PALETTE[0]).is_ok());
        assert!(color("red").is_err() && color("oklch(1;x)").is_err() && color("#12345").is_err());
        assert_eq!(labels(&[" ui ".into(), "UI".into(), "".into(), "api".into()]).unwrap(), ["ui", "api"]);
        assert!(labels(&["x".repeat(41)]).is_err());
        assert_eq!(acceptance(&["a\nb".into(), "  ".into()]).unwrap(), ["a b"]);
        assert!(executor(&Executor::Agent { name: "a b".into(), source: crate::domain::AgentSource::User }).is_err());
        assert!(executor(&Executor::Workflow { name: "-x".into() }).is_err());
        assert!(executor(&Executor::Claude).is_ok());
        assert_eq!(editor(" cursor ").unwrap(), "cursor");
        assert!(editor("rm").is_err());
        assert!(concurrency(0).is_err() && concurrency(17).is_err() && concurrency(3).is_ok());
        assert_eq!(extra_instructions(Some("  ")).unwrap(), None);
        assert_eq!(reviewer(" code-reviewer ").unwrap(), "code-reviewer");
    }
}
