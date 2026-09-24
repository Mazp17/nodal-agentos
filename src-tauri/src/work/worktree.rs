//! Worktrees de las tareas: `~/.nodal/worktrees/<repo>/<task-slug>` con la rama
//! `nodal/<task-slug>`, creados desde la rama actual del repo antes del primer run y
//! reusados en los siguientes (incluidos los traspasos). Todo bloqueante.

use std::path::{Path, PathBuf};

use crate::domain::WorktreeRef;
use crate::util::git;

pub const BRANCH_PREFIX: &str = "nodal/";
const SLUG_MAX: usize = 48;

/// Minúsculas ASCII, dígitos y `-` (sin repetir ni en los bordes).
pub fn slugify(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            if out.len() >= max {
                break;
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            if out.len() + 1 >= max {
                break;
            }
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `pay-1-logo-nuevo-en-el-header`: la key de la tarea (única en la app) más el título.
pub fn task_slug(project_key: &str, number: i64, title: &str) -> String {
    let key = slugify(&format!("{project_key}-{number}"), 20);
    let rest = slugify(title, SLUG_MAX.saturating_sub(key.len() + 1));
    if rest.is_empty() {
        key
    } else {
        format!("{key}-{rest}")
    }
}

pub fn branch_for(slug: &str) -> String {
    format!("{BRANCH_PREFIX}{slug}")
}

/// Carpeta del worktree: `<root>/<repo>/<slug>` (`root` = `~/.nodal/worktrees`).
pub fn dir_for(root: &Path, repo_name: &str, slug: &str) -> PathBuf {
    let repo = slugify(repo_name, 60);
    root.join(if repo.is_empty() { "repo".to_string() } else { repo }).join(slug)
}

/// Rama actual del repo (o el commit si está en detached HEAD): la base del worktree.
pub fn current_base(repo: &Path) -> Result<String, String> {
    let out = git::run(repo, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let branch = out.stdout.trim();
    if out.ok && !branch.is_empty() {
        return Ok(branch.to_string());
    }
    Ok(git::ok(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool, String> {
    Ok(git::run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])?.ok)
}

/// ¿`dir` es un worktree vivo (su toplevel es él mismo)?
fn is_live_worktree(dir: &Path) -> bool {
    dir.is_dir()
        && git::toplevel(dir)
            .ok()
            .flatten()
            .and_then(|t| t.canonicalize().ok())
            .zip(dir.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

/// Crea (o reusa) el worktree de la tarea. `existing`: el que ya tiene guardado.
pub fn ensure(repo: &Path, dir: &Path, branch: &str, existing: Option<&WorktreeRef>) -> Result<WorktreeRef, String> {
    if let Some(wt) = existing {
        let p = Path::new(&wt.path);
        if is_live_worktree(p) {
            return Ok(wt.clone());
        }
    }
    let (dir, branch) = match existing {
        Some(wt) => (PathBuf::from(&wt.path), wt.branch.clone()),
        None => (dir.to_path_buf(), branch.to_string()),
    };
    let base = match existing {
        Some(wt) => wt.base.clone(),
        None => current_base(repo)?,
    };
    // Registros de worktrees borrados a mano.
    let _ = git::run(repo, &["worktree", "prune"]);
    if dir.exists() {
        if is_live_worktree(&dir) {
            return Ok(WorktreeRef { path: dir.to_string_lossy().into_owned(), branch, base });
        }
        let empty = std::fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !empty {
            return Err(format!("{} already exists and is not a worktree of this repo.", dir.display()));
        }
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
    }
    let dir_s = dir.to_string_lossy().into_owned();
    if branch_exists(repo, &branch)? {
        git::ok(repo, &["worktree", "add", &dir_s, &branch])?;
    } else {
        git::ok(repo, &["worktree", "add", "-b", &branch, &dir_s, &base])?;
    }
    let path = dir.canonicalize().map(|p| p.to_string_lossy().into_owned()).unwrap_or(dir_s);
    Ok(WorktreeRef { path, branch, base })
}

/// Borra el worktree (aunque tenga cambios) y su rama. Lo que no existe se ignora.
pub fn cleanup(repo: &Path, wt: &WorktreeRef) -> Result<(), String> {
    let p = Path::new(&wt.path);
    if p.exists() {
        git::ok(repo, &["worktree", "remove", "--force", &wt.path])?;
    }
    let _ = git::run(repo, &["worktree", "prune"]);
    if branch_exists(repo, &wt.branch)? {
        git::ok(repo, &["branch", "-D", &wt.branch])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::paths::tests::{git_available, init_repo, TempDir};

    #[test]
    fn slugs() {
        assert_eq!(task_slug("PAY", 1, "Logo nuevo en el header!"), "pay-1-logo-nuevo-en-el-header");
        assert_eq!(task_slug("WEB", 12, "Migración de ñandúes"), "web-12-migracion-de-nandues");
        assert_eq!(task_slug("A1", 3, "  ¿¿??  "), "a1-3");
        let long = task_slug("PAY", 1, &"palabra ".repeat(20));
        assert!(long.len() <= SLUG_MAX, "{long}");
        assert!(!long.ends_with('-'));
        assert_eq!(branch_for("pay-1-x"), "nodal/pay-1-x");
        assert_eq!(dir_for(Path::new("/w"), "My Repo", "pay-1"), PathBuf::from("/w/my-repo/pay-1"));
    }

    #[test]
    fn create_reuse_and_cleanup() {
        if !git_available() {
            eprintln!("git no disponible: se saltea");
            return;
        }
        let t = TempDir::new("worktree");
        let repo = t.0.join("repo");
        init_repo(&repo);
        let dir = dir_for(&t.0.join("worktrees"), "repo", "pay-1-logo");
        let wt = ensure(&repo, &dir, "nodal/pay-1-logo", None).unwrap();
        assert_eq!(wt.branch, "nodal/pay-1-logo");
        assert_eq!(wt.base, "main");
        assert!(Path::new(&wt.path).join("README.md").is_file());
        assert_eq!(current_base(Path::new(&wt.path)).unwrap(), "nodal/pay-1-logo");

        // Reusar: mismo worktree, sin tocar lo que haya adentro.
        std::fs::write(Path::new(&wt.path).join("wip.txt"), "x").unwrap();
        let again = ensure(&repo, &dir, "nodal/pay-1-logo", Some(&wt)).unwrap();
        assert_eq!(again, wt);
        assert!(Path::new(&wt.path).join("wip.txt").is_file());

        // Si lo borran a mano, se recrea sobre la misma rama.
        std::fs::remove_dir_all(&wt.path).unwrap();
        let recreated = ensure(&repo, &dir, "nodal/pay-1-logo", Some(&wt)).unwrap();
        assert_eq!(recreated.branch, wt.branch);
        assert!(Path::new(&recreated.path).is_dir());

        // Una carpeta ajena con contenido no se pisa.
        let foreign = t.0.join("worktrees/repo/other");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("f"), "x").unwrap();
        assert!(ensure(&repo, &foreign, "nodal/other", None).unwrap_err().contains("not a worktree"));

        cleanup(&repo, &recreated).unwrap();
        assert!(!Path::new(&recreated.path).exists());
        assert!(!branch_exists(&repo, "nodal/pay-1-logo").unwrap());
        // Idempotente.
        cleanup(&repo, &recreated).unwrap();
    }
}
