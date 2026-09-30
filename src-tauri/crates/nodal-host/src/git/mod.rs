//! Synchronous `git` with a timeout (called from blocking threads). No shell: each
//! argument is passed separately.
//!
//! `worktree`, `diff` and `merge` build on this with the higher-level operations behind
//! the `Git` port.

pub mod diff;
pub mod merge;
pub mod worktree;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::claude::bin as claude_bin;

pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct GitOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

fn git_bin() -> Result<PathBuf, String> {
    claude_bin::resolve_git()
}

/// Runs `git -C <dir> <args>` and returns the output even if it fails. Errors only if it
/// couldn't run or the timeout passed. The chokepoint every `git` spawn goes through.
#[tracing::instrument(skip(dir, args), fields(git.subcommand = args.first().copied().unwrap_or("")), level = "info")]
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
    crate::metrics::record_git_spawn();
    let child = cmd.spawn().map_err(|e| format!("Couldn't run {what}: {e}"))?;
    let deadline = Instant::now() + GIT_TIMEOUT;
    wait_for_output(child, deadline, &what)
}

/// Drains `child`'s stdout/stderr and waits for it to exit, killing it past `deadline`.
///
/// Unix (P06): a single `poll(2)` call multiplexes both pipes with the exact remaining
/// timeout as its argument, so the calling thread blocks in the kernel until there's data,
/// a pipe closes, or the deadline passes — no extra threads, no periodic wake-ups. Once both
/// pipes are drained the child has already exited in virtually every case, so the final
/// `wait()` returns immediately.
#[cfg(unix)]
fn wait_for_output(mut child: std::process::Child, deadline: Instant, what: &str) -> Result<GitOutput, String> {
    use std::os::unix::io::AsRawFd;

    let (Some(mut out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(format!("{what}: missing stdout/stderr pipe"));
    };
    let (out_fd, err_fd) = (out.as_raw_fd(), err.as_raw_fd());
    set_nonblocking(out_fd);
    set_nonblocking(err_fd);

    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let (mut out_open, mut err_open) = (true, true);

    while out_open || err_open {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{what} didn't finish within {} s.", GIT_TIMEOUT.as_secs()));
        }
        let mut fds = Vec::with_capacity(2);
        if out_open {
            fds.push(libc::pollfd { fd: out_fd, events: libc::POLLIN, revents: 0 });
        }
        if err_open {
            fds.push(libc::pollfd { fd: err_fd, events: libc::POLLIN, revents: 0 });
        }
        let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `fds` is a valid, appropriately-sized buffer of `pollfd` for the duration
        // of this call; both fds stay open (owned by `out`/`err`) until we mark them closed.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("{what}: poll failed: {e}"));
        }
        let mut i = 0;
        if out_open {
            if fds[i].revents != 0 && !drain_nonblocking(&mut out, &mut out_buf) {
                out_open = false;
            }
            i += 1;
        }
        if err_open && fds[i].revents != 0 && !drain_nonblocking(&mut err, &mut err_buf) {
            err_open = false;
        }
    }
    let status = child.wait().map_err(|e| format!("{what} failed: {e}"))?;
    Ok(GitOutput {
        ok: status.success(),
        stdout: String::from_utf8_lossy(&out_buf).into_owned(),
        stderr: String::from_utf8_lossy(&err_buf).into_owned(),
    })
}

#[cfg(unix)]
fn set_nonblocking(fd: std::os::unix::io::RawFd) {
    // SAFETY: `fd` is a valid, open pipe fd owned by the caller for at least as long as this
    // call; `fcntl(F_GETFL/F_SETFL)` on it is the standard way to flip `O_NONBLOCK`.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// Reads whatever is available into `buf` without blocking. `false` once the pipe has hit
/// EOF (the writer closed it); `true` if there may be more to come.
#[cfg(unix)]
fn drain_nonblocking(f: &mut impl Read, buf: &mut Vec<u8>) -> bool {
    let mut chunk = [0u8; 8192];
    loop {
        match f.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if n < chunk.len() {
                    return true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
}

/// Non-Unix fallback: the original 2-reader-threads-plus-poll implementation. Not on the
/// measured/optimized path (the app targets macOS; CI is macOS-only), kept only so the crate
/// still builds and behaves correctly elsewhere.
#[cfg(not(unix))]
fn wait_for_output(mut child: std::process::Child, deadline: Instant, what: &str) -> Result<GitOutput, String> {
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

/// Like `run`, but a non-zero exit code is an error (with the stderr).
pub fn ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = run(dir, args)?;
    if out.ok {
        Ok(out.stdout)
    } else {
        let msg = out.stderr.trim();
        let msg = if msg.is_empty() { out.stdout.trim() } else { msg };
        Err(format!("`git {}` failed: {}", args.join(" "), nodal_domain::util::clip_chars(msg, 600)))
    }
}

/// Root of the repo containing `dir`, or `None` if it's not in a git repo.
pub fn toplevel(dir: &Path) -> Result<Option<PathBuf>, String> {
    let out = run(dir, &["rev-parse", "--show-toplevel"])?;
    let root = out.stdout.trim();
    Ok((out.ok && !root.is_empty()).then(|| PathBuf::from(root)))
}

/// `git` version (`git version 2.x`), using the same resolver as the rest of the app.
/// Shell-only free function: the shell wraps this in its own blocking helper, as
/// `claude::cli::version` does for `claude --version`.
pub async fn version() -> Result<String, String> {
    let out = tokio::task::spawn_blocking(|| ok(Path::new("/"), &["--version"]))
        .await
        .map_err(|e| format!("Internal error: {e}"))??;
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests;
