//! A run's diff: `git diff <base>...HEAD` in its folder, plus anything uncommitted
//! (including new untracked files), parsed into files and hunks.
//!
//! Types and parsing live in `nodal_domain::diff`.
//!
//! `collect` is cached per `(cwd, base)`: it recomputes only when the worktree changed
//! (current HEAD oid + a hash of `git status`'s output differ from what's cached). New
//! untracked files no longer cost one `git diff --no-index` process each: their patch is
//! synthesized in Rust from the file's own bytes (see `synth_new_file`).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use nodal_domain::diff::{parse_log, CommitInfo, COMMITS_MAX, PATCH_MAX};

use super::{ok, run};

const UNTRACKED_MAX: usize = 200;
const UNTRACKED_FILE_MAX: u64 = 1024 * 1024;
/// Bytes sniffed from the start of a new file to decide whether it's binary (git's own
/// heuristic looks for a NUL in roughly the same range).
const BINARY_SNIFF_LEN: usize = 8000;
/// Cached `collect` results kept around, oldest evicted first. Small on purpose: a patch can
/// be up to `PATCH_MAX` (8 MiB), so this bounds worst-case cache memory, not just entry count.
const DIFF_CACHE_CAP: usize = 16;

/// Commits in `head` that aren't in `base` (`git log base..head`).
pub fn commits(dir: &Path, base: &str, head: &str) -> Result<Vec<CommitInfo>, String> {
    if base.starts_with('-') || head.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    let range = format!("{base}..{head}");
    let max = COMMITS_MAX.to_string();
    let out = ok(
        dir,
        &[
            "log",
            "--no-color",
            "--format=%H%x1f%h%x1f%an%x1f%at%x1f%s",
            "-n",
            &max,
            &range,
            "--",
        ],
    )?;
    Ok(parse_log(&out))
}

/// Branch checked out in `dir`; `None` with a detached HEAD.
pub fn current_branch(dir: &Path) -> Option<String> {
    let b = ok(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let b = b.trim();
    (!b.is_empty() && b != "HEAD").then(|| b.to_string())
}

/// Hard cap on git processes a single `collect()` call can spend, enforced (not just
/// documented) by [`ProcessBudget`]: `status` + `merge-base` + the rare `rev-parse` fallback +
/// `diff` = 4, with one process of headroom. Exceeding it fails the diff instead of spawning
/// more processes, so a future change can't silently reintroduce an unbounded per-file loop.
const MAX_GIT_PROCESSES: usize = 5;

/// Per-call budget of git processes, threaded through `collect()`'s helpers. Local state (not
/// the process-wide spawn counter in `crate::metrics`): counting it doesn't race with other
/// tests or other `run_diff` calls.
struct ProcessBudget(usize);

impl ProcessBudget {
    fn spend(&mut self, what: &str) -> Result<(), String> {
        self.0 = self
            .0
            .checked_sub(1)
            .ok_or_else(|| format!("Too many git processes for one diff (budget exhausted at `{what}`)."))?;
        Ok(())
    }
}

/// A worktree's `git status`, parsed once: the HEAD oid (from `# branch.oid`, works even
/// detached), the untracked files (each its own `? path` line, unquoted because `-z`
/// disables quoting entirely), whether anything changed, and a hash covering every
/// non-header line (so any tracked or untracked change flips it).
struct StatusSnapshot {
    head: String,
    dirty: bool,
    untracked: Vec<String>,
    hash: u64,
}

/// `git status --porcelain=v2 --branch --untracked-files=all -z` in `cwd`: HEAD plus every
/// tracked and untracked change, individual files (not collapsed directories), in one process.
fn status_snapshot(cwd: &Path, budget: &mut ProcessBudget) -> Result<StatusSnapshot, String> {
    budget.spend("status")?;
    let out = ok(
        cwd,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ],
    )?;
    Ok(parse_status(&out))
}

fn parse_status(raw: &str) -> StatusSnapshot {
    let mut head = String::new();
    let mut untracked = Vec::new();
    let mut dirty = false;
    let mut hasher = DefaultHasher::new();
    let mut entries = raw.split('\0').filter(|s| !s.is_empty());
    while let Some(e) = entries.next() {
        if let Some(oid) = e.strip_prefix("# branch.oid ") {
            head = oid.trim().to_string();
            continue;
        }
        if e.starts_with('#') {
            // Other branch headers (name, upstream, ahead/behind): don't affect the diff.
            continue;
        }
        e.hash(&mut hasher);
        if let Some(path) = e.strip_prefix("? ") {
            untracked.push(path.to_string());
            dirty = true;
        } else if e.starts_with("2 ") {
            dirty = true;
            entries.next(); // Rename/copy entries append the orig path as its own NUL field.
        } else if e.starts_with("1 ") || e.starts_with("u ") {
            dirty = true;
        }
    }
    StatusSnapshot { head, dirty, untracked, hash: hasher.finish() }
}

type CacheKey = (PathBuf, Option<String>);

struct CacheEntry {
    head: String,
    status_hash: u64,
    patch: String,
    dirty: bool,
}

#[derive(Default)]
struct DiffCache {
    entries: HashMap<CacheKey, CacheEntry>,
    order: Vec<CacheKey>,
}

impl DiffCache {
    fn hit(&self, key: &CacheKey, snapshot: &StatusSnapshot) -> Option<(String, bool)> {
        let e = self.entries.get(key)?;
        (e.head == snapshot.head && e.status_hash == snapshot.hash).then(|| (e.patch.clone(), e.dirty))
    }

    fn put(&mut self, key: CacheKey, entry: CacheEntry) {
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= DIFF_CACHE_CAP && !self.order.is_empty() {
                let oldest = self.order.remove(0);
                self.entries.remove(&oldest);
            }
            self.order.push(key.clone());
        }
        self.entries.insert(key, entry);
    }
}

fn cache() -> &'static Mutex<DiffCache> {
    static CACHE: OnceLock<Mutex<DiffCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(DiffCache::default()))
}

/// Raw patch of `cwd` against `base` (with `None`, against HEAD: only uncommitted work).
/// Returns `(patch, has uncommitted changes)`.
///
/// Recomputes only when the worktree changed since the last call for this `(cwd, base)`
/// (HEAD oid + a hash of `git status`); otherwise reuses the cached patch. On a miss this
/// spawns at most `MAX_GIT_PROCESSES` git processes (`status`, `merge-base`, `diff`; a
/// `rev-parse` fallback only in the rare case `merge-base` fails) instead of one more per new
/// untracked file.
pub fn collect(cwd: &Path, base: Option<&str>) -> Result<(String, bool), String> {
    let mut budget = ProcessBudget(MAX_GIT_PROCESSES);
    let snapshot = status_snapshot(cwd, &mut budget)?;
    let key: CacheKey = (cwd.to_path_buf(), base.map(str::to_string));
    if let Some(hit) = cache().lock().unwrap_or_else(|p| p.into_inner()).hit(&key, &snapshot) {
        return Ok(hit);
    }
    let (patch, dirty) = collect_uncached(cwd, base, &snapshot, &mut budget)?;
    cache().lock().unwrap_or_else(|p| p.into_inner()).put(
        key,
        CacheEntry { head: snapshot.head, status_hash: snapshot.hash, patch: patch.clone(), dirty },
    );
    Ok((patch, dirty))
}

fn collect_uncached(
    cwd: &Path,
    base: Option<&str>,
    snapshot: &StatusSnapshot,
    budget: &mut ProcessBudget,
) -> Result<(String, bool), String> {
    let from = match base {
        Some(b) => {
            // `base...HEAD` = from the merge-base; if there's none (unrelated histories), the base.
            budget.spend("merge-base")?;
            let mb = run(cwd, &["merge-base", b, "HEAD"])?;
            if mb.ok && !mb.stdout.trim().is_empty() {
                mb.stdout.trim().to_string()
            } else {
                budget.spend("rev-parse")?;
                ok(cwd, &["rev-parse", "--verify", b])?.trim().to_string()
            }
        }
        None => "HEAD".to_string(),
    };
    // Without `..HEAD`: compares the working tree against `from`, which includes the branch's
    // commits and whatever wasn't committed.
    budget.spend("diff")?;
    let mut patch = ok(
        cwd,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "-M",
            "--no-color",
            "--no-ext-diff",
            &from,
            "--",
        ],
    )?;
    if snapshot.dirty {
        for file in snapshot.untracked.iter().take(UNTRACKED_MAX) {
            if patch.len() > PATCH_MAX {
                break;
            }
            patch.push_str(&synth_new_file(cwd, file));
        }
    }
    if patch.len() > PATCH_MAX {
        let mut cut = PATCH_MAX;
        while !patch.is_char_boundary(cut) {
            cut -= 1;
        }
        patch.truncate(cut);
    }
    Ok((patch, snapshot.dirty))
}

/// The unified-diff text for a single new untracked `file` (relative to `cwd`), equivalent
/// to `git diff --no-index /dev/null file` but built from the file's bytes: no process spawn.
fn synth_new_file(cwd: &Path, file: &str) -> String {
    let full = cwd.join(file);
    let Ok(data) = std::fs::read(&full) else {
        return String::new(); // Vanished between `status` and now.
    };
    let qp = quote_diff_path(file);
    if data.len() as u64 > UNTRACKED_FILE_MAX || looks_binary(&data) {
        return format!("diff --git a/{qp} b/{qp}\nnew file mode 100644\nBinary files /dev/null and b/{qp} differ\n");
    }
    let Ok(text) = std::str::from_utf8(&data) else {
        return format!("diff --git a/{qp} b/{qp}\nnew file mode 100644\nBinary files /dev/null and b/{qp} differ\n");
    };
    let mode = if is_executable(&full) { "100755" } else { "100644" };
    let mut out = format!("diff --git a/{qp} b/{qp}\nnew file mode {mode}\nindex 0000000..0000000\n");
    if text.is_empty() {
        return out; // Matches `git diff --no-index` on an empty file: header only, no hunk.
    }
    out.push_str("--- /dev/null\n");
    out.push_str(&format!("+++ b/{qp}\n"));
    let ends_with_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = body.split('\n').collect();
    out.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    for line in &lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    if !ends_with_newline {
        out.push_str("\\ No newline at end of file\n");
    }
    out
}

/// git's own heuristic: a NUL in the first few KB means binary.
fn looks_binary(data: &[u8]) -> bool {
    data.iter().take(BINARY_SNIFF_LEN).any(|&b| b == 0)
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        false
    }
}

/// Diff-header quoting for the rare path with a quote, backslash or tab: the exact inverse
/// of `nodal_domain::diff::unquote`.
fn quote_diff_path(p: &str) -> String {
    if !p.bytes().any(|b| matches!(b, b'"' | b'\\' | b'\t')) {
        return p.to_string();
    }
    let mut s = String::with_capacity(p.len() + 2);
    s.push('"');
    for c in p.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\t' => s.push_str("\\t"),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

/// Patch of a branch that isn't in its own checkout (a workflow's):
/// `git diff <base>...<branch>`, without a working tree.
pub fn collect_branch(repo: &Path, base: &str, branch: &str) -> Result<String, String> {
    if branch.starts_with('-') || base.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    let range = format!("{base}...{branch}");
    let mut patch = ok(
        repo,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "-M",
            "--no-color",
            "--no-ext-diff",
            &range,
            "--",
        ],
    )?;
    if patch.len() > PATCH_MAX {
        let mut cut = PATCH_MAX;
        while !patch.is_char_boundary(cut) {
            cut -= 1;
        }
        patch.truncate(cut);
    }
    Ok(patch)
}

#[cfg(test)]
mod tests;
