//! Does Claude Code trust a folder? Reads the global config READ-ONLY
//! (`$CLAUDE_CONFIG_DIR/.claude.json` or `~/.claude.json`), `projects[<path>].hasTrustDialogAccepted`.
//!
//! Rule verified against the Claude Code 2.1.281 bundle:
//! 1. the exact key of the repo's canonical root (for a worktree, the main repo;
//!    outside git, the folder itself) with `hasTrustDialogAccepted === true`;
//! 2. otherwise, walk up from the folder to the git root containing it (inclusive): the
//!    first one with `hasTrustDialogAccepted` true. Outside git it walks up to `/`.
//!
//! Trust for the home folder is session-only and not persisted: there's nothing to read.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::util::git;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustSource {
    /// Entry for the repo's canonical root.
    Repo,
    /// Entry for the folder or a parent folder inside the repo.
    Parent,
    /// No trusted entry: Claude Code will show the dialog.
    NotTrusted,
    /// There's no Claude Code config (never opened) or it couldn't be read.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoTrust {
    /// `null`: unknown (no readable config).
    pub trusted: Option<bool>,
    pub source: TrustSource,
    /// The `projects` key that granted the trust.
    pub matched_path: Option<String>,
}

pub fn global_config_file() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join(".claude.json"));
    }
    crate::util::paths::home().map(|h| h.join(".claude.json"))
}

fn accepted(config: &Value, key: &Path) -> Option<bool> {
    config.get("projects")?.get(key.to_str()?)?.get("hasTrustDialogAccepted")?.as_bool()
}

/// Pure rule. `canonical_root`: root of the main repo (or the folder, outside git);
/// `bound`: the folder's git root (`None` outside git).
pub fn trust_of(config: &Value, path: &Path, canonical_root: &Path, bound: Option<&Path>) -> RepoTrust {
    if accepted(config, canonical_root) == Some(true) {
        return RepoTrust {
            trusted: Some(true),
            source: TrustSource::Repo,
            matched_path: Some(canonical_root.to_string_lossy().into_owned()),
        };
    }
    let mut cur = Some(path);
    while let Some(dir) = cur {
        if bound.is_some_and(|b| !dir.starts_with(b)) {
            break;
        }
        if accepted(config, dir) == Some(true) {
            return RepoTrust {
                trusted: Some(true),
                source: TrustSource::Parent,
                matched_path: Some(dir.to_string_lossy().into_owned()),
            };
        }
        if bound == Some(dir) {
            break;
        }
        cur = dir.parent();
    }
    RepoTrust { trusted: Some(false), source: TrustSource::NotTrusted, matched_path: None }
}

/// Root of the main repo containing `dir` (the same for its worktrees).
fn main_root(dir: &Path) -> Option<PathBuf> {
    let out = git::run(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()?;
    let common = PathBuf::from(out.stdout.trim());
    (out.ok && common.file_name().is_some_and(|n| n == ".git")).then(|| common.parent().map(Path::to_path_buf)).flatten()
}

/// Blocking: reads the config and queries git.
pub fn repo_trust_blocking(path: &Path, config_file: Option<&Path>) -> Result<RepoTrust, String> {
    if !path.is_absolute() {
        return Err(format!("The path must be absolute: {}", path.display()));
    }
    let unknown = RepoTrust { trusted: None, source: TrustSource::Unknown, matched_path: None };
    let Some(file) = config_file else { return Ok(unknown) };
    let Ok(text) = std::fs::read_to_string(file) else { return Ok(unknown) };
    let Ok(config) = serde_json::from_str::<Value>(&text) else { return Ok(unknown) };
    // git returns resolved paths (`/private/tmp`, not `/tmp`): compare with the canonicalized
    // path, and if that's not enough, with the path as is (no git bound).
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let bound = if real.is_dir() { git::toplevel(&real)?.map(|t| t.canonicalize().unwrap_or(t)) } else { None };
    let canonical = match &bound {
        Some(top) => main_root(top).map(|m| m.canonicalize().unwrap_or(m)).unwrap_or_else(|| top.clone()),
        None => real.clone(),
    };
    let t = trust_of(&config, &real, &canonical, bound.as_deref());
    if t.trusted == Some(true) || real == path {
        return Ok(t);
    }
    let raw_bound = bound.as_ref().and_then(|b| {
        // The same bound expressed with the unresolved prefix.
        let rel = real.strip_prefix(b).ok()?;
        let n = rel.components().count();
        let mut p = path;
        for _ in 0..n {
            p = p.parent()?;
        }
        Some(p.to_path_buf())
    });
    let raw = trust_of(&config, path, raw_bound.as_deref().unwrap_or(path), raw_bound.as_deref());
    Ok(if raw.trusted == Some(true) { raw } else { t })
}

/// Whether Claude Code already trusts the folder (no trust dialog on launch).
#[tauri::command]
pub async fn repo_trust(path: String) -> Result<RepoTrust, String> {
    let path = PathBuf::from(path.trim());
    crate::util::blocking(move || repo_trust_blocking(&path, global_config_file().as_deref())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::paths::tests::{git_available, init_repo, TempDir};
    use serde_json::json;

    fn cfg(entries: &[(&str, bool)]) -> Value {
        let projects: serde_json::Map<String, Value> =
            entries.iter().map(|(k, v)| (k.to_string(), json!({"hasTrustDialogAccepted": v}))).collect();
        json!({ "projects": projects })
    }

    #[test]
    fn rule_exact_parent_and_bound() {
        let repo = Path::new("/w/acme");
        let sub = Path::new("/w/acme/packages/web");
        // Canonical root.
        let t = trust_of(&cfg(&[("/w/acme", true)]), sub, repo, Some(repo));
        assert_eq!((t.trusted, t.source, t.matched_path.as_deref()), (Some(true), TrustSource::Repo, Some("/w/acme")));
        // Parent folder inside the repo.
        let t = trust_of(&cfg(&[("/w/acme/packages", true)]), sub, repo, Some(repo));
        assert_eq!((t.trusted, t.source), (Some(true), TrustSource::Parent));
        // A parent OUTSIDE the repo doesn't count; outside git, it does.
        let t = trust_of(&cfg(&[("/w", true)]), sub, repo, Some(repo));
        assert_eq!((t.trusted, t.source), (Some(false), TrustSource::NotTrusted));
        let t = trust_of(&cfg(&[("/w", true)]), sub, sub, None);
        assert_eq!(t.matched_path.as_deref(), Some("/w"));
        // Explicit `false` or missing: not trusted.
        let t = trust_of(&cfg(&[("/w/acme", false)]), repo, repo, Some(repo));
        assert_eq!(t.trusted, Some(false));
        assert_eq!(trust_of(&json!({}), repo, repo, Some(repo)).trusted, Some(false));
        // Worktree: the canonical root is the main repo even though it's outside the bound.
        let wt = Path::new("/wt/acme/pay-1");
        let t = trust_of(&cfg(&[("/w/acme", true)]), wt, repo, Some(wt));
        assert_eq!(t.source, TrustSource::Repo);
    }

    #[test]
    fn reads_config_and_resolves_worktrees() {
        if !git_available() {
            eprintln!("git not available: skipping");
            return;
        }
        let t = TempDir::new("trust");
        let repo = t.0.join("repo");
        init_repo(&repo);
        let wt = t.0.join("wt");
        git::ok(&repo, &["worktree", "add", "-q", "-b", "nodal/x", wt.to_str().unwrap()]).unwrap();
        let file = t.0.join(".claude.json");
        std::fs::write(&file, cfg(&[(repo.to_str().unwrap(), true)]).to_string()).unwrap();

        let r = repo_trust_blocking(&wt, Some(&file)).unwrap();
        assert_eq!((r.trusted, r.source), (Some(true), TrustSource::Repo));
        let sub = repo.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(repo_trust_blocking(&sub, Some(&file)).unwrap().trusted, Some(true));

        std::fs::write(&file, "{}").unwrap();
        assert_eq!(repo_trust_blocking(&repo, Some(&file)).unwrap().trusted, Some(false));
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(repo_trust_blocking(&repo, Some(&file)).unwrap().source, TrustSource::Unknown);
        assert_eq!(repo_trust_blocking(&repo, Some(&t.0.join("missing.json"))).unwrap().trusted, None);
        assert!(repo_trust_blocking(Path::new("relative"), Some(&file)).is_err());

        // Symlink to the folder: resolved to compare with what git returns.
        let link = t.0.join("link");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        std::fs::write(&file, cfg(&[(repo.join("src").to_str().unwrap(), true)]).to_string()).unwrap();
        let r = repo_trust_blocking(&link.join("src"), Some(&file)).unwrap();
        assert_eq!((r.trusted, r.source), (Some(true), TrustSource::Parent));
        // And an entry saved with the unresolved path counts too.
        std::fs::write(&file, cfg(&[(link.to_str().unwrap(), true)]).to_string()).unwrap();
        assert_eq!(repo_trust_blocking(&link.join("src"), Some(&file)).unwrap().trusted, Some(true));
    }
}
