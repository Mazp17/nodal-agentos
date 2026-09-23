//! Validación de entradas de tareas (rutas, título, plan) y armado del prompt.
//! Todo lo que toca disco es bloqueante: se llama desde `spawn_blocking`.

use std::path::{Path, PathBuf};

use serde::Serialize;

pub const WORKFLOW: &str = "plan-task";
pub const MAX_TITLE_CHARS: usize = 200;
/// Tope para planes en texto y para leer un plan (archivo o texto).
pub const MAX_PLAN_BYTES: u64 = 512 * 1024;

/// Repo git válido: absoluto, existe, es carpeta y tiene `.git` (carpeta, o archivo en
/// worktrees/submódulos). Devuelve la ruta canonicalizada.
pub fn repo_root(repo_path: &str) -> Result<PathBuf, String> {
    let raw = repo_path.trim();
    if raw.is_empty() {
        return Err("Pick a repository.".into());
    }
    let p = Path::new(raw);
    if !p.is_absolute() {
        return Err(format!("The repository path must be absolute: {raw}"));
    }
    let canon = p.canonicalize().map_err(|_| format!("The repository folder doesn't exist: {raw}"))?;
    if !canon.is_dir() {
        return Err(format!("The repository path is not a folder: {raw}"));
    }
    if !canon.join(".git").exists() {
        return Err(format!("{raw} is not the root of a git repository (no .git)."));
    }
    Ok(canon)
}

/// Archivo de plan: `.md`, existe, es archivo y queda dentro de `repo` después de resolver
/// symlinks y `..` (canonicalize). `path` puede ser absoluta o relativa al repo.
pub fn plan_file(repo: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err("Pick a plan file.".into());
    }
    let p = Path::new(raw);
    let joined = if p.is_absolute() { p.to_path_buf() } else { repo.join(p) };
    let canon = joined.canonicalize().map_err(|_| format!("The plan file doesn't exist: {raw}"))?;
    if !canon.starts_with(repo) {
        return Err(format!("The plan file must be inside the repository ({}).", repo.display()));
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

pub fn title(title: &str) -> Result<String, String> {
    // Una sola línea: el título va al prompt (dentro de JSON, pero igual).
    let t = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return Err("The title is empty.".into());
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("The title is too long (max {MAX_TITLE_CHARS} characters)."));
    }
    Ok(t)
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

/// "pr" (default) | "branch".
pub fn finish(finish: Option<&str>) -> Result<String, String> {
    match finish.map(str::trim).filter(|f| !f.is_empty()).unwrap_or("pr") {
        f @ ("pr" | "branch") => Ok(f.to_string()),
        other => Err(format!("Unknown finish mode «{other}»: use \"pr\" or \"branch\".")),
    }
}

#[derive(Serialize)]
struct PlanTaskArgs<'a> {
    plan: &'a str,
    title: &'a str,
    finish: &'a str,
}

/// `/plan-task {"plan":"…","title":"…","finish":"…"}`. El JSON lo arma serde (escapa
/// comillas, barras y saltos de línea); nunca se concatena a mano.
pub fn prompt(plan_path: &Path, title: &str, finish: &str) -> Result<String, String> {
    let plan = plan_path.to_str().ok_or("The plan path is not valid UTF-8.")?;
    let json = serde_json::to_string(&PlanTaskArgs { plan, title, finish }).map_err(|e| e.to_string())?;
    Ok(format!("/{WORKFLOW} {json}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRepo(PathBuf);
    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Dos carpetas hermanas: `repo` (con .git) y `outside`.
    fn setup(name: &str) -> (TempRepo, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("agent-desk-tasks-validate-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        let outside = base.join("outside");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(repo.join("docs/plan.md"), "# Plan").unwrap();
        std::fs::write(repo.join("docs/notes.txt"), "x").unwrap();
        std::fs::write(outside.join("evil.md"), "# Evil").unwrap();
        let repo = repo.canonicalize().unwrap();
        let outside = outside.canonicalize().unwrap();
        (TempRepo(base), repo, outside)
    }

    #[test]
    fn repo_must_be_absolute_existing_git_root() {
        let (_g, repo, outside) = setup("repo");
        assert_eq!(repo_root(repo.to_str().unwrap()).unwrap(), repo);
        assert!(repo_root("relative/repo").unwrap_err().contains("absolute"));
        assert!(repo_root("/no/such/repo").unwrap_err().contains("doesn't exist"));
        assert!(repo_root(outside.to_str().unwrap()).unwrap_err().contains("git"));
        assert!(repo_root("  ").is_err());
    }

    #[test]
    fn plan_file_rules() {
        let (_g, repo, outside) = setup("plan");
        let ok = repo.join("docs/plan.md");
        assert_eq!(plan_file(&repo, ok.to_str().unwrap()).unwrap(), ok);
        assert_eq!(plan_file(&repo, "docs/plan.md").unwrap(), ok);
        assert_eq!(plan_file(&repo, "./docs/../docs/plan.md").unwrap(), ok);
        // Traversal hacia afuera, absoluta afuera, inexistente, no .md, carpeta.
        assert!(plan_file(&repo, "../outside/evil.md").unwrap_err().contains("inside"));
        assert!(plan_file(&repo, outside.join("evil.md").to_str().unwrap()).unwrap_err().contains("inside"));
        assert!(plan_file(&repo, "docs/missing.md").unwrap_err().contains("doesn't exist"));
        assert!(plan_file(&repo, "docs/notes.txt").unwrap_err().contains(".md"));
        assert!(plan_file(&repo, "").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn plan_file_symlink_escaping_repo_is_rejected() {
        let (_g, repo, outside) = setup("symlink");
        std::os::unix::fs::symlink(outside.join("evil.md"), repo.join("docs/link.md")).unwrap();
        assert!(plan_file(&repo, "docs/link.md").unwrap_err().contains("inside"));
        // Una carpeta con nombre .md tampoco sirve.
        std::fs::create_dir_all(repo.join("dir.md")).unwrap();
        assert!(plan_file(&repo, "dir.md").unwrap_err().contains("not a file"));
    }

    #[test]
    fn prompt_is_serde_json() {
        let p = prompt(Path::new("/r/docs/plan \"x\".md"), "Fix \"quotes\" \\ and\nnewline", "pr").unwrap();
        let json = p.strip_prefix("/plan-task ").unwrap();
        assert!(!json.contains('\n'), "{p}");
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["plan"], "/r/docs/plan \"x\".md");
        assert_eq!(v["title"], "Fix \"quotes\" \\ and\nnewline");
        assert_eq!(v["finish"], "pr");
        assert_eq!(v.as_object().unwrap().len(), 3);
    }

    #[test]
    fn title_finish_and_text() {
        assert_eq!(title("  Fix   the\n bug ").unwrap(), "Fix the bug");
        assert!(title(" \n ").is_err());
        assert!(title(&"x".repeat(MAX_TITLE_CHARS + 1)).is_err());
        assert_eq!(finish(None).unwrap(), "pr");
        assert_eq!(finish(Some("branch")).unwrap(), "branch");
        assert!(finish(Some("merge")).is_err());
        assert!(plan_text("  ").is_err());
        assert!(plan_text("# ok").is_ok());
    }
}
