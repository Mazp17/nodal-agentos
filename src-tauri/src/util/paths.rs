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

/// Debug builds (`pnpm tauri dev`) keep their own database, worktrees and keychain entries,
/// so they never touch the data of an installed Nodal.
pub const DEV: bool = cfg!(debug_assertions);

/// `~/.nodal` (`~/.nodal-dev` in debug builds): worktrees de las tareas.
pub fn nodal_home() -> Result<PathBuf, String> {
    let name = if DEV { ".nodal-dev" } else { ".nodal" };
    home().map(|h| h.join(name)).ok_or_else(|| "$HOME is not set.".to_string())
}

/// App data folder (database, plans). Debug builds use `<identifier>.dev` next to it.
pub fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    Ok(if DEV { dev_sibling(&dir) } else { dir })
}

/// `identifier` of `tauri.conf.json`: names the app data folder.
pub const APP_IDENTIFIER: &str = "io.github.mazp17.nodal";

/// `data_dir` without an `AppHandle`, for `nodal-mcp`. Same folder Tauri resolves on macOS.
pub fn data_dir_standalone() -> Result<PathBuf, String> {
    let home = home().ok_or_else(|| "$HOME is not set.".to_string())?;
    let dir = home.join("Library/Application Support").join(APP_IDENTIFIER);
    Ok(if DEV { dev_sibling(&dir) } else { dir })
}

/// Unix socket of the MCP server, inside the app data folder.
pub fn mcp_socket(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("mcp.sock")
}

/// `/x/io.github.mazp17.nodal` → `/x/io.github.mazp17.nodal.dev`.
fn dev_sibling(dir: &std::path::Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".dev");
    dir.with_file_name(name)
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
    fn dev_data_lives_next_to_the_release_data() {
        let dir = Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal");
        assert_eq!(dev_sibling(dir), Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal.dev"));
        // `cargo test` is a debug build unless run with --release.
        assert_eq!(nodal_home().unwrap().ends_with(".nodal-dev"), DEV);
    }

    #[test]
    fn standalone_data_dir_matches_the_app_and_keeps_debug_apart() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], APP_IDENTIFIER);
        let dir = data_dir_standalone().unwrap();
        assert_eq!(dir.parent().unwrap(), home().unwrap().join("Library/Application Support"));
        assert_eq!(dir.to_string_lossy().ends_with(".nodal.dev"), DEV);
        assert_eq!(mcp_socket(&dir), dir.join("mcp.sock"));
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
