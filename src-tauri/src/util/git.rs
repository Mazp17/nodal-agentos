//! `git` síncrono con timeout (se llama desde hilos bloqueantes). Sin shell: cada
//! argumento va por separado.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::runs::claude_bin;

pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct GitOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

fn git_bin() -> Result<PathBuf, String> {
    claude_bin::resolve_bin("git").ok_or_else(|| "Couldn't find `git` in PATH.".to_string())
}

/// Corre `git -C <dir> <args>` y devuelve la salida aunque falle. Error solo si no se pudo
/// ejecutar o pasó el timeout.
pub fn run(dir: &Path, args: &[&str]) -> Result<GitOutput, String> {
    let mut cmd = Command::new(git_bin()?);
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        .env("PATH", claude_bin::augmented_path())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let what = format!("git {}", args.first().copied().unwrap_or(""));
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run {what}: {e}"))?;
    // Se leen los dos streams en paralelo: un diff grande llenaría el pipe y colgaría a git.
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut buf);
        }
        buf
    });
    let err_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what} didn't finish within {} s.", GIT_TIMEOUT.as_secs()));
            }
            Err(e) => return Err(format!("{what} failed: {e}")),
        }
    };
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    Ok(GitOutput { ok: status.success(), stdout, stderr })
}

/// Como `run`, pero un código de salida distinto de 0 es error (con el stderr).
pub fn ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = run(dir, args)?;
    if out.ok {
        Ok(out.stdout)
    } else {
        let msg = out.stderr.trim();
        let msg = if msg.is_empty() { out.stdout.trim() } else { msg };
        Err(format!("`git {}` failed: {}", args.join(" "), crate::util::clip_chars(msg, 600)))
    }
}

/// Raíz del repo que contiene `dir`, o `None` si no está en un repo git.
pub fn toplevel(dir: &Path) -> Result<Option<PathBuf>, String> {
    let out = run(dir, &["rev-parse", "--show-toplevel"])?;
    let root = out.stdout.trim();
    Ok((out.ok && !root.is_empty()).then(|| PathBuf::from(root)))
}
