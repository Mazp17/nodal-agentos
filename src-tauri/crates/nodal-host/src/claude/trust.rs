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

use crate::git;
use crate::paths::home;

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
    home().map(|h| h.join(".claude.json"))
}

fn accepted(config: &Value, key: &Path) -> Option<bool> {
    config
        .get("projects")?
        .get(key.to_str()?)?
        .get("hasTrustDialogAccepted")?
        .as_bool()
}

/// Pure rule. `canonical_root`: root of the main repo (or the folder, outside git);
/// `bound`: the folder's git root (`None` outside git).
pub fn trust_of(
    config: &Value,
    path: &Path,
    canonical_root: &Path,
    bound: Option<&Path>,
) -> RepoTrust {
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
    RepoTrust {
        trusted: Some(false),
        source: TrustSource::NotTrusted,
        matched_path: None,
    }
}

/// Root of the main repo containing `dir` (the same for its worktrees).
fn main_root(dir: &Path) -> Option<PathBuf> {
    let out = git::run(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let common = PathBuf::from(out.stdout.trim());
    (out.ok && common.file_name().is_some_and(|n| n == ".git"))
        .then(|| common.parent().map(Path::to_path_buf))
        .flatten()
}

/// Blocking: reads the config and queries git.
pub fn repo_trust_blocking(path: &Path, config_file: Option<&Path>) -> Result<RepoTrust, String> {
    if !path.is_absolute() {
        return Err(format!("The path must be absolute: {}", path.display()));
    }
    let unknown = RepoTrust {
        trusted: None,
        source: TrustSource::Unknown,
        matched_path: None,
    };
    let Some(file) = config_file else {
        return Ok(unknown);
    };
    let Ok(text) = std::fs::read_to_string(file) else {
        return Ok(unknown);
    };
    let Ok(config) = serde_json::from_str::<Value>(&text) else {
        return Ok(unknown);
    };
    // git returns resolved paths (`/private/tmp`, not `/tmp`): compare with the canonicalized
    // path, and if that's not enough, with the path as is (no git bound).
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let bound = if real.is_dir() {
        git::toplevel(&real)?.map(|t| t.canonicalize().unwrap_or(t))
    } else {
        None
    };
    let canonical = match &bound {
        Some(top) => main_root(top)
            .map(|m| m.canonicalize().unwrap_or(m))
            .unwrap_or_else(|| top.clone()),
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
    let raw = trust_of(
        &config,
        path,
        raw_bound.as_deref().unwrap_or(path),
        raw_bound.as_deref(),
    );
    Ok(if raw.trusted == Some(true) { raw } else { t })
}

#[cfg(test)]
mod tests;
