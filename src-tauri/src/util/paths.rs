//! Rutas: `~`, raíz git canónica y la carpeta de Nodal en el home.

use std::path::PathBuf;

use super::{blocking, git};

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `~` y `~/x` → home. El resto queda igual.
pub fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), home()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ if path == "~" => home().unwrap_or_default(),
        _ => PathBuf::from(path),
    }
}

/// `~/.nodal`: worktrees de las tareas.
pub fn nodal_home() -> Result<PathBuf, String> {
    home().map(|h| h.join(".nodal")).ok_or_else(|| "$HOME is not set.".to_string())
}

/// Carpeta absoluta y existente (con `~` expandido).
fn existing_dir(path: &str) -> Result<PathBuf, String> {
    let dir = expand_home(path.trim());
    if !dir.is_absolute() {
        return Err(format!("The folder must be an absolute path: {path}"));
    }
    if !dir.is_dir() {
        return Err(format!("The folder doesn't exist: {path}"));
    }
    Ok(dir)
}

/// Raíz git canónica (symlinks resueltos) del repo que contiene `path`. `Ok(None)` si no
/// está dentro de un repo git.
pub fn git_root_of(path: &str) -> Result<Option<PathBuf>, String> {
    let dir = existing_dir(path)?;
    let Some(root) = git::toplevel(&dir)? else { return Ok(None) };
    Ok(Some(root.canonicalize().map_err(|e| format!("Couldn't resolve {}: {e}", root.display()))?))
}

/// Raíz git canónica o error listo para mostrar.
pub fn require_git_root(path: &str) -> Result<PathBuf, String> {
    git_root_of(path)?.ok_or_else(|| format!("{} is not inside a git repository.", path.trim()))
}

/// Raíz del repo git que contiene `path`, o `None` si no está dentro de un repo. Para
/// ofrecer la raíz cuando eligen una subcarpeta.
#[tauri::command]
pub async fn resolve_git_root(path: String) -> Result<Option<String>, String> {
    blocking(move || Ok(git_root_of(&path)?.map(|p| p.to_string_lossy().into_owned()))).await
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;

    /// Carpeta temporal propia que se borra al soltarla.
    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("nodal-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            TempDir(d.canonicalize().unwrap())
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn git_available() -> bool {
        std::process::Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    /// `git init` con un commit inicial en `main` y una identidad local de prueba.
    pub fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        std::fs::create_dir_all(dir).unwrap();
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
    }

    #[test]
    fn git_root_of_subfolder() {
        if !git_available() {
            eprintln!("git no disponible: se saltea");
            return;
        }
        let t = TempDir::new("git-root");
        let repo = t.0.join("repo");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let sub = repo.join("a/b").to_string_lossy().into_owned();
        let root = tauri::async_runtime::block_on(resolve_git_root(sub)).unwrap();
        assert_eq!(root.as_deref(), Some(repo.to_string_lossy().as_ref()));
        let outside = t.0.join("not-git");
        std::fs::create_dir_all(&outside).unwrap();
        assert_eq!(git_root_of(outside.to_str().unwrap()).unwrap(), None);
        assert!(require_git_root(outside.to_str().unwrap()).unwrap_err().contains("not inside a git repository"));
        assert!(tauri::async_runtime::block_on(resolve_git_root("relativa".into())).is_err());
        assert!(git_root_of("/no/such/dir").unwrap_err().contains("doesn't exist"));
    }

    #[test]
    fn expands_home() {
        let h = home().unwrap();
        assert_eq!(expand_home("~/x"), h.join("x"));
        assert_eq!(expand_home("~"), h);
        assert_eq!(expand_home("/a/~"), PathBuf::from("/a/~"));
    }
}
