//! ¿Claude Code confía en una carpeta? Se lee en SOLO LECTURA el config global
//! (`$CLAUDE_CONFIG_DIR/.claude.json` o `~/.claude.json`), `projects[<ruta>].hasTrustDialogAccepted`.
//!
//! Regla verificada contra el bundle de Claude Code 2.1.281:
//! 1. la clave exacta de la raíz canónica del repo (para un worktree, el repo principal;
//!    fuera de git, la carpeta misma) con `hasTrustDialogAccepted === true`;
//! 2. si no, se sube desde la carpeta hasta la raíz git que la contiene (inclusive): la
//!    primera con `hasTrustDialogAccepted` verdadero. Fuera de git se sube hasta `/`.
//!
//! La confianza del home es solo de sesión y no se persiste: no hay nada que leer.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::util::git;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustSource {
    /// Entrada de la raíz canónica del repo.
    Repo,
    /// Entrada de la carpeta o de una carpeta padre dentro del repo.
    Parent,
    /// Ninguna entrada con confianza: Claude Code va a mostrar el diálogo.
    NotTrusted,
    /// No hay config de Claude Code (nunca se abrió) o no se pudo leer.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoTrust {
    /// `null`: no se sabe (sin config legible).
    pub trusted: Option<bool>,
    pub source: TrustSource,
    /// La clave de `projects` que dio la confianza.
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

/// Regla pura. `canonical_root`: raíz del repo principal (o la carpeta fuera de git);
/// `bound`: raíz git de la carpeta (`None` fuera de git).
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

/// Raíz del repo principal que contiene `dir` (la misma para sus worktrees).
fn main_root(dir: &Path) -> Option<PathBuf> {
    let out = git::run(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()?;
    let common = PathBuf::from(out.stdout.trim());
    (out.ok && common.file_name().is_some_and(|n| n == ".git")).then(|| common.parent().map(Path::to_path_buf)).flatten()
}

/// Bloqueante: lee el config y consulta git.
pub fn repo_trust_blocking(path: &Path, config_file: Option<&Path>) -> Result<RepoTrust, String> {
    if !path.is_absolute() {
        return Err(format!("The path must be absolute: {}", path.display()));
    }
    let unknown = RepoTrust { trusted: None, source: TrustSource::Unknown, matched_path: None };
    let Some(file) = config_file else { return Ok(unknown) };
    let Ok(text) = std::fs::read_to_string(file) else { return Ok(unknown) };
    let Ok(config) = serde_json::from_str::<Value>(&text) else { return Ok(unknown) };
    let bound = if path.is_dir() { git::toplevel(path)? } else { None };
    let canonical = match &bound {
        Some(top) => main_root(top).unwrap_or_else(|| top.clone()),
        None => path.to_path_buf(),
    };
    Ok(trust_of(&config, path, &canonical, bound.as_deref()))
}

/// Si Claude Code ya confía en la carpeta (sin diálogo de confianza al lanzar).
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
        // Raíz canónica.
        let t = trust_of(&cfg(&[("/w/acme", true)]), sub, repo, Some(repo));
        assert_eq!((t.trusted, t.source, t.matched_path.as_deref()), (Some(true), TrustSource::Repo, Some("/w/acme")));
        // Carpeta padre dentro del repo.
        let t = trust_of(&cfg(&[("/w/acme/packages", true)]), sub, repo, Some(repo));
        assert_eq!((t.trusted, t.source), (Some(true), TrustSource::Parent));
        // Un padre FUERA del repo no cuenta; fuera de git, sí.
        let t = trust_of(&cfg(&[("/w", true)]), sub, repo, Some(repo));
        assert_eq!((t.trusted, t.source), (Some(false), TrustSource::NotTrusted));
        let t = trust_of(&cfg(&[("/w", true)]), sub, sub, None);
        assert_eq!(t.matched_path.as_deref(), Some("/w"));
        // `false` explícito o ausente: no confía.
        let t = trust_of(&cfg(&[("/w/acme", false)]), repo, repo, Some(repo));
        assert_eq!(t.trusted, Some(false));
        assert_eq!(trust_of(&json!({}), repo, repo, Some(repo)).trusted, Some(false));
        // Worktree: la raíz canónica es el repo principal aunque esté fuera del bound.
        let wt = Path::new("/wt/acme/pay-1");
        let t = trust_of(&cfg(&[("/w/acme", true)]), wt, repo, Some(wt));
        assert_eq!(t.source, TrustSource::Repo);
    }

    #[test]
    fn reads_config_and_resolves_worktrees() {
        if !git_available() {
            eprintln!("git no disponible: se saltea");
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
        std::fs::write(&file, "no es json").unwrap();
        assert_eq!(repo_trust_blocking(&repo, Some(&file)).unwrap().source, TrustSource::Unknown);
        assert_eq!(repo_trust_blocking(&repo, Some(&t.0.join("missing.json"))).unwrap().trusted, None);
        assert!(repo_trust_blocking(Path::new("relativa"), Some(&file)).is_err());
    }
}
