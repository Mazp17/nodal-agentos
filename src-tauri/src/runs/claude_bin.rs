//! Resolución del binario `claude` y ejecución con timeout.
//!
//! Una app abierta desde Finder no hereda el PATH del shell: por eso se busca en PATH y,
//! si no está, en las rutas de instalación conocidas; y al hijo se le pasa un PATH ampliado
//! para que `claude` encuentre a su vez git, node, etc.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::process::Command;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Directorios donde suele vivir `claude` (y herramientas que usa) fuera del PATH de Finder.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = home() {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Ruta de un ejecutable: primero el PATH, después los directorios conocidos.
pub fn resolve_bin(name: &str) -> Option<PathBuf> {
    let from_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    from_path.into_iter().chain(extra_dirs()).map(|d| d.join(name)).find(|c| is_executable(c))
}

/// Ruta del binario `claude`: primero el PATH, después `~/.local/bin/claude` y afines.
pub fn resolve_claude() -> Result<PathBuf, String> {
    resolve_bin("claude").ok_or_else(|| {
        "Couldn't find the `claude` executable in PATH or ~/.local/bin. Is Claude Code installed?".to_string()
    })
}

pub fn augmented_path() -> OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in extra_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// `claude` listo para configurar: sin stdin y con PATH ampliado.
pub fn claude_command() -> Result<Command, String> {
    let mut cmd = Command::new(resolve_claude()?);
    cmd.env("PATH", augmented_path()).stdin(Stdio::null());
    Ok(cmd)
}

/// Ejecuta y espera la salida completa; si pasa `limit`, mata el proceso.
pub async fn output_with_timeout(mut cmd: Command, limit: Duration, what: &str) -> Result<Output, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| format!("Couldn't run {what}: {e}"))?;
    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(format!("{what} failed: {e}")),
        // Al soltar el futuro se suelta el hijo y `kill_on_drop` lo mata.
        Err(_) => Err(format!("{what} didn't respond within {} s and was cancelled.", limit.as_secs())),
    }
}

/// stderr (o stdout si stderr está vacío), recortado, para mensajes de error.
pub fn error_text(out: &Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        err
    }
}
