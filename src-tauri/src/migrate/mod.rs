//! "Import data from a previous version…": imports the previous version's data folder,
//! chosen by the user (the app doesn't know any old path).
//!
//! 1. Copies the old version's data files (the JSON files below and
//!    `tasks/<id>/plan.md`, nothing else and with a size cap) to
//!    `<app_data_dir>/legacy-backup-<ts>/`, or reuses the latest backup if identical. Having
//!    none of those JSON files is an error and nothing is copied. The source is only read.
//! 2. Imports from the copy, in a single transaction, `config.json`, `tasks.json`,
//!    `task-runs.json`, `issue-runs.json` and `tasks/<id>/plan.md`.
//! 3. After the commit, writes any missing plans to `<app_data_dir>/tasks/<id>/plan.md`.
//!
//! Idempotency: each imported object leaves a row in `legacy_imports` (a stable key
//! derived from the old record) and new ids are deterministic. Re-importing the same
//! folder creates nothing new and counts each record in `already_imported`.
//!
//! Rules:
//! - each repo in `config.json` → a Repo; if its path already belongs to a project, that
//!   project is used, otherwise one is created (name = scope if known, else the folder). Each
//!   scope in the mapping → a SourceLink of that project (state mapping pending);
//! - local tasks → the project of the repo with that path; if there's none, the "Local"
//!   project (with a new repo for that path). They're assigned to `plan-task`, which is how
//!   they used to run;
//! - runs → `runs` table with `legacy_label` (issue identifier or task title) and a NULL
//!   `task_id` if there's no task. `launching` → Failed("migrated"); `launched` → Finished
//!   with no outcome (not re-evaluated: triggers no transitions); `queued` → stays **Queued**:
//!   with `legacy_label` the queue doesn't launch it on its own
//!   (`work::queue::awaiting_confirmation`) until it's confirmed (`confirm_run`) or canceled;
//! - `concurrency` → settings (only the first time, and never overwrites one already set);
//! - a corrupt JSON (file or record) is skipped with a warning in `skipped`.

pub mod legacy;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::db::{self, rows, Db, DbError};
use crate::domain::*;

pub const BACKUP_PREFIX: &str = "legacy-backup-";
pub const LOCAL_PROJECT: &str = "Local";
pub const LAUNCHING_ERROR: &str = "migrated";
/// Name old runs used to launch local tasks.
const LEGACY_TASK_WORKFLOW: &str = "plan-task";

const COLORS: &[&str] = &[
    "oklch(0.74 0.15 55)",
    "oklch(0.72 0.12 150)",
    "oklch(0.70 0.13 250)",
    "oklch(0.72 0.14 320)",
    "oklch(0.78 0.12 95)",
    "oklch(0.68 0.14 25)",
    "oklch(0.72 0.10 200)",
];

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportReport {
    /// Untouched copy of the chosen folder.
    pub backup_dir: String,
    pub projects: u32,
    pub repos: u32,
    pub tasks: u32,
    pub runs: u32,
    /// Already imported before (idempotency).
    pub already_imported: u32,
    /// Skipped files or records, with the reason.
    pub skipped: Vec<String>,
}

#[tauri::command]
pub async fn import_legacy_data(
    app: AppHandle,
    db: State<'_, Db>,
    folder: String,
) -> Result<LegacyImportReport, String> {
    let data_dir = crate::util::paths::data_dir(&app)?;
    let now = crate::util::now_ms();
    let src = PathBuf::from(folder.trim());
    let dd = data_dir.clone();
    // The copy doesn't take the database lock.
    let (backup_dir, skipped) = tauri::async_runtime::spawn_blocking(move || backup(&src, &dd, now))
        .await
        .map_err(|e| format!("Backup task failed: {e}"))??;
    let db = db.inner().clone();
    let mut report = db::with_db(&db, move |conn| {
        import_backup(conn, &backup_dir, &data_dir, now).map_err(DbError::Invalid)
    })
    .await?;
    report.skipped.splice(0..0, skipped);
    Ok(report)
}

/// Copy + import (what the command does, on a single thread). For tests.
#[cfg(test)]
pub fn import_folder(conn: &mut Connection, src: &Path, data_dir: &Path, now: i64) -> Result<LegacyImportReport, String> {
    let (backup_dir, skipped) = backup(src, data_dir, now)?;
    let mut report = import_backup(conn, &backup_dir, data_dir, now)?;
    report.skipped.splice(0..0, skipped);
    Ok(report)
}

// ---------- Backup ----------

/// Cap on what's copied to the backup: the old version's JSON files and plans weigh KBs, so
/// anything larger is almost certainly the wrong folder.
pub const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;

/// The old version's data files; the folder must have at least one.
pub const LEGACY_JSON: [&str; 4] =
    [legacy::CONFIG_FILE, legacy::TASKS_FILE, legacy::TASK_RUNS_FILE, legacy::ISSUE_RUNS_FILE];

fn is_regular_file(p: &Path) -> Option<bool> {
    fs::symlink_metadata(p).ok().map(|m| m.is_file())
}

/// (relative path in the backup, path in the source).
type LegacyFile = (PathBuf, PathBuf);

/// What gets copied: the `LEGACY_JSON` files present and `tasks/<id>/plan.md`. Nothing else
/// from the folder (so choosing `~` doesn't copy the disk). No known JSON is an error. Doesn't
/// follow symlinks.
fn legacy_files(src: &Path) -> Result<(Vec<LegacyFile>, Vec<String>), String> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for name in LEGACY_JSON {
        let p = src.join(name);
        match is_regular_file(&p) {
            Some(true) => files.push((PathBuf::from(name), p)),
            Some(false) => skipped.push(format!("{}: not a regular file, not copied", p.display())),
            None => {}
        }
    }
    if files.is_empty() {
        return Err(format!(
            "{} doesn't look like a data folder from the previous version (it has none of {}). Nothing was copied.",
            src.display(),
            LEGACY_JSON.join(", ")
        ));
    }
    let plans = src.join(legacy::PLANS_DIR);
    if fs::symlink_metadata(&plans).is_ok_and(|m| m.is_dir()) {
        let entries = fs::read_dir(&plans).map_err(|e| format!("Couldn't read {}: {e}", plans.display()))?;
        let mut entries: Vec<_> = entries.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if !fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let plan = entry.path().join(legacy::PLAN_FILE);
            match is_regular_file(&plan) {
                Some(true) => {
                    let rel = PathBuf::from(legacy::PLANS_DIR).join(entry.file_name()).join(legacy::PLAN_FILE);
                    files.push((rel, plan));
                }
                Some(false) => skipped.push(format!("{}: not a regular file, not copied", plan.display())),
                None => {}
            }
        }
    }
    Ok((files, skipped))
}

/// `legacy-backup-<ts>[-n]` → `(ts, n)` for sorting.
fn backup_order(name: &str) -> Option<(i64, u32)> {
    let rest = name.strip_prefix(BACKUP_PREFIX)?;
    let (ts, n) = rest.split_once('-').unwrap_or((rest, "1"));
    Some((ts.parse().ok()?, n.parse().ok()?))
}

/// The most recent backup in the data folder.
fn latest_backup(data_dir: &Path) -> Option<PathBuf> {
    fs::read_dir(data_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir()))
        .filter_map(|e| backup_order(&e.file_name().to_string_lossy()).map(|k| (k, e.path())))
        .max_by_key(|(k, _)| *k)
        .map(|(_, p)| p)
}

/// Files in a tree (without following symlinks), relative to `root`.
fn tree_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if fs::symlink_metadata(&p)?.is_dir() {
            tree_files(root, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

/// `backup` has exactly these files, with the same content.
fn same_content(backup: &Path, files: &[LegacyFile]) -> bool {
    let mut have = Vec::new();
    if tree_files(backup, backup, &mut have).is_err() {
        return false;
    }
    let mut want: Vec<&PathBuf> = files.iter().map(|(rel, _)| rel).collect();
    have.sort();
    want.sort();
    have.iter().eq(want.iter().copied())
        && files.iter().all(|(rel, from)| match (fs::read(backup.join(rel)), fs::read(from)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        })
}

/// Copies the old version's files from `src` (see `legacy_files`) to
/// `<data_dir>/legacy-backup-<now>[-n]/` and returns that path plus the warnings. If the most
/// recent backup already has exactly that content, it's reused instead of copying again.
pub fn backup(src: &Path, data_dir: &Path, now: i64) -> Result<(PathBuf, Vec<String>), String> {
    let src = src
        .canonicalize()
        .map_err(|e| format!("Couldn't open the folder {}: {e}", src.display()))?;
    if !src.is_dir() {
        return Err(format!("{} is not a folder.", src.display()));
    }
    let (files, skipped) = legacy_files(&src)?;
    let mut total: u64 = 0;
    for (_, from) in &files {
        total += fs::symlink_metadata(from).map_err(|e| format!("Couldn't read {}: {e}", from.display()))?.len();
    }
    if total > MAX_BACKUP_BYTES {
        return Err(format!(
            "The data in {} takes {} MB, more than the {} MB an import can copy. Nothing was copied.",
            src.display(),
            total.div_ceil(1024 * 1024),
            MAX_BACKUP_BYTES / (1024 * 1024)
        ));
    }
    fs::create_dir_all(data_dir).map_err(|e| format!("Couldn't create {}: {e}", data_dir.display()))?;
    if let Some(prev) = latest_backup(data_dir).filter(|p| same_content(p, &files)) {
        return Ok((prev, skipped));
    }
    let mut dest = data_dir.join(format!("{BACKUP_PREFIX}{now}"));
    let mut n = 2;
    while dest.exists() {
        dest = data_dir.join(format!("{BACKUP_PREFIX}{now}-{n}"));
        n += 1;
    }
    fs::create_dir(&dest).map_err(|e| format!("Couldn't create {}: {e}", dest.display()))?;
    for (rel, from) in &files {
        let to = dest.join(rel);
        let res = to.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::copy(from, &to).map(|_| ()));
        if let Err(e) = res {
            let _ = fs::remove_dir_all(&dest);
            return Err(format!("Couldn't copy {}: {e}. Nothing was imported.", from.display()));
        }
    }
    Ok((dest, skipped))
}

// ---------- Import ----------

/// Imports from an already copied folder. All in one transaction; plans are written after
/// the commit (only missing ones, so a retry completes those that failed).
pub fn import_backup(conn: &mut Connection, dir: &Path, data_dir: &Path, now: i64) -> Result<LegacyImportReport, String> {
    let tx = conn.transaction().map_err(sql)?;
    let mut ctx = Ctx {
        conn: &tx,
        now,
        data_dir,
        report: LegacyImportReport { backup_dir: dir.display().to_string(), ..Default::default() },
        finish_by_path: HashMap::new(),
        plans: Vec::new(),
        fresh: HashSet::new(),
    };
    ctx.import_config(dir)?;
    ctx.import_tasks(dir)?;
    ctx.import_task_runs(dir)?;
    ctx.import_issue_runs(dir)?;
    let Ctx { mut report, plans, .. } = ctx;
    tx.commit().map_err(sql)?;

    for (from, to) in plans {
        if to.exists() {
            continue;
        }
        let res = to
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::copy(&from, &to).map(|_| ()));
        if let Err(e) = res {
            report.skipped.push(format!("{}: couldn't copy the plan ({e})", to.display()));
        }
    }
    Ok(report)
}

fn sql(e: impl std::fmt::Display) -> String {
    format!("Import failed, nothing was imported: {e}")
}

struct Ctx<'c> {
    conn: &'c Connection,
    now: i64,
    data_dir: &'c Path,
    report: LegacyImportReport,
    /// Repo finish by path (for issue runs, which didn't store it).
    finish_by_path: HashMap<String, Finish>,
    /// Plans to copy after the commit: (source in the backup, destination).
    plans: Vec<(PathBuf, PathBuf)>,
    /// Keys marked in this same import (a duplicate within the file doesn't count as
    /// "already imported").
    fresh: HashSet<String>,
}

/// 64-bit FNV-1a hash: stable across Rust versions (unlike `DefaultHasher`).
fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn det_id(prefix: &str, key: &str) -> String {
    format!("{prefix}{}", fnv(key))
}

fn expand(raw: &str) -> PathBuf {
    let raw = raw.trim();
    match (raw.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(raw),
    }
}

fn trim_slash(p: &Path) -> String {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.into() }
}

/// `~`, whitespace and trailing `/`; doesn't touch the disk (for idempotency keys).
pub fn norm_path(raw: &str) -> String {
    trim_slash(&expand(raw))
}

/// Like `norm_path`, plus symlinks resolved if the path exists (to compare with `repos.path`).
pub fn canon_path(raw: &str) -> String {
    let p = expand(raw);
    trim_slash(&p.canonicalize().unwrap_or(p))
}

fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// `my-app` → `MA`, `payments` → `PAYM`. Always `[A-Z][A-Z0-9]*`.
pub fn key_base(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let mut k: String = if words.len() >= 2 {
        words.iter().take(4).filter_map(|w| w.chars().next()).collect()
    } else {
        words.first().map(|w| w.chars().take(4).collect()).unwrap_or_default()
    };
    k = k.to_ascii_uppercase();
    if !k.starts_with(|c: char| c.is_ascii_alphabetic()) {
        k.insert(0, 'P');
    }
    k
}

fn parse_finish(s: Option<&str>) -> Finish {
    match s.map(str::trim) {
        Some("branch") => Finish::Commit,
        _ => Finish::Pr,
    }
}

/// `/plan-task {...}` → workflow `plan-task`; anything else, a plain session.
fn executor_from_prompt(prompt: &str) -> Executor {
    match prompt.trim().strip_prefix('/').and_then(|r| r.split_whitespace().next()) {
        Some(name) if !name.is_empty() => Executor::Workflow { name: name.into() },
        _ => Executor::Claude,
    }
}

/// Old status → (status, error, outcome). See the rules in the module doc.
fn map_run_status(status: &str, error: Option<String>) -> Result<(RunStatus, Option<String>), String> {
    Ok(match status {
        "queued" => (RunStatus::Queued, None),
        "launching" => (RunStatus::Failed, Some(LAUNCHING_ERROR.into())),
        "launched" => (RunStatus::Finished, error),
        "failed" => (RunStatus::Failed, Some(error.unwrap_or_else(|| LAUNCHING_ERROR.into()))),
        other => return Err(format!("unknown status \"{other}\"")),
    })
}

/// A list of records: `{<field>: [...]}` or just `[...]`.
fn records(v: Value, field: &str) -> Option<Vec<Value>> {
    match v {
        Value::Array(a) => Some(a),
        Value::Object(mut o) => match o.remove(field) {
            Some(Value::Array(a)) => Some(a),
            None => Some(Vec::new()),
            _ => None,
        },
        _ => None,
    }
}

impl Ctx<'_> {
    fn skip(&mut self, msg: String) {
        self.report.skipped.push(msg);
    }

    /// Reads a JSON file from the backup. Missing → `None` without warning; corrupt → `None` with a warning.
    fn read_json(&mut self, dir: &Path, file: &str) -> Option<Value> {
        let path = dir.join(file);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                self.skip(format!("{file}: couldn't read it ({e})"));
                return None;
            }
        };
        match serde_json::from_str(&text) {
            Ok(v) => Some(v),
            Err(e) => {
                self.skip(format!("{file}: corrupt JSON, skipped ({e})"));
                None
            }
        }
    }

    fn read_records(&mut self, dir: &Path, file: &str, field: &str) -> Vec<Value> {
        let Some(v) = self.read_json(dir, file) else { return Vec::new() };
        records(v, field).unwrap_or_else(|| {
            self.skip(format!("{file}: unexpected format, skipped"));
            Vec::new()
        })
    }

    fn parse<T: serde::de::DeserializeOwned>(&mut self, file: &str, i: usize, v: Value) -> Option<T> {
        match serde_json::from_value(v) {
            Ok(t) => Some(t),
            Err(e) => {
                self.skip(format!("{file}: record #{} is invalid, skipped ({e})", i + 1));
                None
            }
        }
    }

    // --- legacy_imports ---

    fn seen(&self, key: &str) -> Result<Option<Option<String>>, String> {
        self.conn
            .query_row("SELECT target_id FROM legacy_imports WHERE source_key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(sql)
    }

    fn already(&mut self, key: &str) {
        if !self.fresh.contains(key) {
            self.report.already_imported += 1;
        }
    }

    fn mark(&mut self, key: &str, kind: &str, target: Option<&str>) -> Result<(), String> {
        self.fresh.insert(key.to_string());
        self.conn
            .execute(
                "INSERT OR REPLACE INTO legacy_imports (source_key, kind, target_id, imported_at) VALUES (?1, ?2, ?3, ?4)",
                params![key, kind, target, self.now],
            )
            .map(|_| ())
            .map_err(sql)
    }

    fn exists(&self, table: &str, id: &str) -> Result<bool, String> {
        self.conn
            .query_row(&format!("SELECT 1 FROM {table} WHERE id = ?1"), [id], |_| Ok(()))
            .optional()
            .map(|o| o.is_some())
            .map_err(sql)
    }

    // --- projects and repos ---

    fn repo_by_id(&self, id: &str) -> Result<Option<(String, String)>, String> {
        self.conn
            .query_row("SELECT id, project_id FROM repos WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(sql)
    }

    fn repo_by_path(&self, path: &str) -> Result<Option<(String, String)>, String> {
        self.conn
            .query_row("SELECT id, project_id FROM repos WHERE path = ?1", [path], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(sql)
    }

    fn unique_key(&self, name: &str) -> Result<String, String> {
        let base = key_base(name);
        for n in 1.. {
            let k = if n == 1 { base.clone() } else { format!("{base}{n}") };
            let taken = self
                .conn
                .query_row("SELECT 1 FROM projects WHERE key = ?1", [&k], |_| Ok(()))
                .optional()
                .map_err(sql)?
                .is_some();
            if !taken {
                return Ok(k);
            }
        }
        unreachable!()
    }

    fn create_project(&mut self, id: &str, name: &str) -> Result<(), String> {
        let color = COLORS[(u64::from_str_radix(&fnv(id), 16).unwrap_or(0) % COLORS.len() as u64) as usize];
        let p = Project {
            id: id.into(),
            name: name.into(),
            key: self.unique_key(name)?,
            next_task_number: 1,
            color: color.into(),
            default_executor: None,
            reviewer: None,
            created_at: self.now,
            archived_at: None,
            description: None,
        };
        rows::insert_project(self.conn, &p).map_err(sql)?;
        self.report.projects += 1;
        Ok(())
    }

    /// Deterministic id of the repo the import creates for `path`.
    fn repo_id_for(path: &str) -> String {
        det_id("lr_", path)
    }

    /// Creates the repo for `path` in the project, or returns the one a previous import
    /// already created (even if its path was changed later). A clash with another repo doesn't
    /// abort the import: it's skipped with a warning (`None`).
    fn ensure_repo(
        &mut self,
        project_id: &str,
        path: &str,
        launch: LaunchOptions,
        finish: Finish,
    ) -> Result<Option<(String, String)>, String> {
        let id = Self::repo_id_for(path);
        if let Some(found) = self.repo_by_id(&id)? {
            return Ok(Some(found));
        }
        let position: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM repos WHERE project_id = ?1", [project_id], |r| r.get(0))
            .map_err(sql)?;
        let r = Repo {
            id: id.clone(),
            project_id: project_id.into(),
            path: path.into(),
            name: folder_name(path),
            launch,
            default_executor: None,
            default_isolation: Isolation::Worktree,
            default_finish: finish,
            default_review: true,
            reviewer: None,
            position,
            created_at: self.now,
        };
        match rows::insert_repo(self.conn, &r) {
            Ok(()) => {}
            // Only uniqueness clashes (path or id taken); any other constraint is a bug.
            Err(DbError::Sqlite(rusqlite::Error::SqliteFailure(f, m)))
                if matches!(
                    f.extended_code,
                    rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                ) =>
            {
                let why = m.unwrap_or_else(|| f.to_string());
                self.skip(format!("repo {path}: conflicts with an existing repo ({why}), skipped"));
                return Ok(None);
            }
            Err(e) => return Err(sql(e)),
        }
        self.report.repos += 1;
        Ok(Some((id, project_id.to_string())))
    }

    /// The import's "Local" project (created the first time it's needed).
    fn local_project(&mut self) -> Result<String, String> {
        const KEY: &str = "project:local";
        if let Some(Some(id)) = self.seen(KEY)? {
            if self.exists("projects", &id)? {
                return Ok(id);
            }
        }
        let id = det_id("lp_", KEY);
        if !self.exists("projects", &id)? {
            self.create_project(&id, LOCAL_PROJECT)?;
        }
        self.mark(KEY, "project", Some(&id))?;
        Ok(id)
    }

    // --- config.json ---

    fn import_config(&mut self, dir: &Path) -> Result<(), String> {
        let file = legacy::CONFIG_FILE;
        let Some(v) = self.read_json(dir, file) else { return Ok(()) };
        let cfg: legacy::Config = match serde_json::from_value(v) {
            Ok(c) => c,
            Err(e) => {
                self.skip(format!("{file}: unexpected format, skipped ({e})"));
                return Ok(());
            }
        };

        if let Some(c) = cfg.concurrency {
            const KEY: &str = "settings:concurrency";
            if self.seen(KEY)?.is_some() {
                self.already(KEY);
            } else {
                let c = c.clamp(1, MAX_CONCURRENCY);
                self.conn
                    .execute(
                        "INSERT INTO settings (key, value_json) VALUES ('concurrency', ?1)
                         ON CONFLICT(key) DO NOTHING",
                        [c.to_string()],
                    )
                    .map_err(sql)?;
                self.mark(KEY, "setting", None)?;
            }
        }

        for (i, raw) in cfg.repos.into_iter().enumerate() {
            let Some(entry) = self.parse::<legacy::RepoEntry>(file, i, raw) else { continue };
            if entry.path.trim().is_empty() {
                self.skip(format!("{file}: record #{} has an empty path, skipped", i + 1));
                continue;
            }
            let path = canon_path(&entry.path);
            let finish = parse_finish(entry.finish.as_deref());
            self.finish_by_path.entry(path.clone()).or_insert(finish);
            let scopes = entry.scopes();

            // The key uses the path without resolving symlinks: it doesn't change if the folder
            // appears, disappears or a symlink changes between two imports.
            let stable = norm_path(&entry.path);
            let repo_key = format!("config:repo:{stable}");
            let previous = match self.seen(&repo_key)? {
                Some(Some(id)) => self.repo_by_id(&id)?,
                _ => None,
            };
            let (repo_id, project_id) = match previous {
                Some(found) => {
                    self.already(&repo_key);
                    found
                }
                None => {
                    // The user already had it, or another entry or a previous import created it
                    // (even if its path was changed later): it's reused untouched.
                    let existing = match self.repo_by_path(&path)? {
                        Some(found) => Some(found),
                        None => self.repo_by_id(&Self::repo_id_for(&path))?,
                    };
                    match existing {
                        Some(found) => {
                            self.mark(&repo_key, "repo", Some(&found.0))?;
                            found
                        }
                        None => {
                            let name = scopes
                                .iter()
                                .find_map(|s| s.name.clone().filter(|n| !n.trim().is_empty()))
                                .or_else(|| entry.name.clone().filter(|n| !n.trim().is_empty()))
                                .unwrap_or_else(|| folder_name(&path));
                            let project_id = det_id("lp_", &repo_key);
                            let new_project = !self.exists("projects", &project_id)?;
                            if new_project {
                                self.create_project(&project_id, name.trim())?;
                            }
                            let Some(found) = self.ensure_repo(&project_id, &path, entry.launch(), finish)? else {
                                // Don't leave an empty project behind because of a skipped repo.
                                if new_project {
                                    self.conn.execute("DELETE FROM projects WHERE id = ?1", [&project_id]).map_err(sql)?;
                                    self.report.projects -= 1;
                                }
                                continue;
                            };
                            self.mark(&repo_key, "repo", Some(&found.0))?;
                            found
                        }
                    }
                }
            };

            for s in scopes {
                let link_key = format!("config:link:{stable}:{}:{}:{}", s.provider, s.kind, s.id);
                if self.seen(&link_key)?.is_some() {
                    self.already(&link_key);
                    continue;
                }
                let dup = self
                    .conn
                    .query_row(
                        "SELECT id FROM source_links WHERE project_id = ?1 AND provider = ?2 AND scope_id = ?3",
                        params![project_id, s.provider, s.id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(sql)?;
                let link_id = match dup {
                    Some(id) => id,
                    None => {
                        let id = det_id("ll_", &link_key);
                        let link = SourceLink {
                            id: id.clone(),
                            project_id: project_id.clone(),
                            provider: s.provider.clone(),
                            scope: ScopeRef { kind: s.kind.clone(), id: s.id.clone(), name: s.name.clone().unwrap_or_default() },
                            default_repo_id: Some(repo_id.clone()),
                            repo_rules: Vec::new(),
                            state_map: StateMap::default(),
                            auto_import: false,
                            created_at: self.now,
                            last_synced_at: None,
                            last_sync_error: None,
                            pending_state_changes: None,
                        };
                        rows::insert_source_link(self.conn, &link).map_err(sql)?;
                        id
                    }
                };
                self.mark(&link_key, "source_link", Some(&link_id))?;
            }
        }
        Ok(())
    }

    // --- tasks.json ---

    fn import_tasks(&mut self, dir: &Path) -> Result<(), String> {
        let file = legacy::TASKS_FILE;
        let raws = self.read_records(dir, file, "tasks");
        let mut tasks: Vec<legacy::Task> =
            raws.into_iter().enumerate().filter_map(|(i, v)| self.parse(file, i, v)).collect();
        // Numbers are assigned by creation order: the same file yields the same numbers.
        tasks.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));

        for t in tasks {
            let key = format!("task:{}", t.id);
            let new_id = if is_safe_id(&t.id) { t.id.clone() } else { det_id("lt_", &key) };
            let is_text = matches!(t.plan, legacy::PlanRef::Text);
            if let Some(target) = self.seen(&key)? {
                self.already(&key);
                // If the user deleted it later, don't leave an orphaned plan.
                let alive = match target {
                    Some(id) => self.exists("tasks", &id)?,
                    None => false,
                };
                if is_text && alive {
                    self.queue_plan(dir, &t.id, &new_id);
                }
                continue;
            }
            if self.exists("tasks", &new_id)? {
                self.skip(format!("{file}: task {} already exists with another origin, skipped", t.id));
                continue;
            }
            let status = match t.status.as_str() {
                "todo" => TaskStatus::Todo,
                "done" => TaskStatus::Done,
                other => {
                    self.skip(format!("{file}: task {} has an unknown status \"{other}\", skipped", t.id));
                    continue;
                }
            };
            if t.repo_path.trim().is_empty() {
                self.skip(format!("{file}: task {} has no repository, skipped", t.id));
                continue;
            }
            let path = canon_path(&t.repo_path);
            let existing = match self.repo_by_path(&path)? {
                Some(found) => Some(found),
                // The repo a previous import created, even if its path was changed.
                None => self.repo_by_id(&Self::repo_id_for(&path))?,
            };
            let (repo_id, project_id) = match existing {
                Some(found) => found,
                None => {
                    let project_id = self.local_project()?;
                    let finish = self.finish_by_path.get(&path).copied().unwrap_or(Finish::Pr);
                    match self.ensure_repo(&project_id, &path, LaunchOptions::default(), finish)? {
                        Some(found) => found,
                        None => {
                            self.skip(format!("{file}: task {} skipped, its repository could not be created", t.id));
                            continue;
                        }
                    }
                }
            };
            let number: i64 = self
                .conn
                .query_row("SELECT next_task_number FROM projects WHERE id = ?1", [&project_id], |r| r.get(0))
                .map_err(sql)?;
            self.conn
                .execute("UPDATE projects SET next_task_number = ?1 WHERE id = ?2", params![number + 1, project_id])
                .map_err(sql)?;
            let plan = match &t.plan {
                legacy::PlanRef::Text => PlanRef::Text,
                legacy::PlanRef::File { path } => PlanRef::File { path: path.clone() },
            };
            let done = status == TaskStatus::Done;
            let closed_at = done.then(|| t.done_at.unwrap_or(t.created_at));
            let task = Task {
                id: new_id.clone(),
                project_id,
                repo_id,
                number,
                title: t.title.clone(),
                status,
                priority: Priority::None,
                labels: Vec::new(),
                position: number as f64,
                plan,
                plan_overridden: false,
                acceptance: Vec::new(),
                assignee: Some(Executor::Workflow { name: LEGACY_TASK_WORKFLOW.into() }),
                isolation: None,
                finish: None,
                review: None,
                worktree: None,
                source: None,
                created_at: t.created_at,
                updated_at: closed_at.unwrap_or(t.created_at),
                closed_at,
            };
            rows::insert_task(self.conn, &task).map_err(sql)?;
            self.mark(&key, "task", Some(&new_id))?;
            self.report.tasks += 1;
            if is_text {
                self.queue_plan(dir, &t.id, &new_id);
            }
        }
        Ok(())
    }

    fn queue_plan(&mut self, dir: &Path, old_id: &str, new_id: &str) {
        if !is_safe_id(old_id) {
            self.skip(format!("{}: task {old_id} has an unsafe id, its plan was not copied", legacy::TASKS_FILE));
            return;
        }
        let from = dir.join(legacy::PLANS_DIR).join(old_id).join(legacy::PLAN_FILE);
        if !from.is_file() {
            self.skip(format!("{}/{old_id}/{}: missing, the task has an empty plan", legacy::PLANS_DIR, legacy::PLAN_FILE));
            return;
        }
        let to = self.data_dir.join(legacy::PLANS_DIR).join(new_id).join(legacy::PLAN_FILE);
        self.plans.push((from, to));
    }

    // --- runs ---

    fn task_of(&self, legacy_task_id: &str) -> Result<Option<(String, String, String)>, String> {
        let Some(Some(id)) = self.seen(&format!("task:{legacy_task_id}"))? else { return Ok(None) };
        self.conn
            .query_row("SELECT id, repo_id, title FROM tasks WHERE id = ?1", [&id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()
            .map_err(sql)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_run(
        &mut self,
        key: &str,
        task_id: Option<String>,
        repo_id: Option<String>,
        cwd: String,
        executor: Executor,
        prompt: String,
        options: LaunchOptions,
        finish: Finish,
        status: RunStatus,
        error: Option<String>,
        claude_run_id: Option<String>,
        session_id: Option<String>,
        queued_at: i64,
        launched_at: Option<i64>,
        legacy_label: String,
    ) -> Result<(), String> {
        let id = det_id("lrun_", key);
        let run = Run {
            id: id.clone(),
            task_id,
            repo_id,
            cwd,
            executor,
            kind: RunKind::Work,
            parent_run_id: None,
            prompt,
            extra_instructions: None,
            options,
            finish,
            isolation: None,
            review: false,
            verdict: None,
            status,
            queue_position: queued_at as f64,
            claude_run_id,
            session_id,
            queued_at,
            launched_at,
            finished_at: None,
            outcome: None,
            summary: None,
            pr_url: None,
            branch: None,
            error,
            legacy_label: Some(legacy_label),
            tokens: None,
        };
        rows::insert_run(self.conn, &run).map_err(sql)?;
        self.mark(key, "run", Some(&id))?;
        self.report.runs += 1;
        Ok(())
    }

    fn import_task_runs(&mut self, dir: &Path) -> Result<(), String> {
        let file = legacy::TASK_RUNS_FILE;
        let raws = self.read_records(dir, file, "runs");
        for (i, v) in raws.into_iter().enumerate() {
            let Some(r) = self.parse::<legacy::TaskRun>(file, i, v) else { continue };
            // Without `run_id`: a run that was queued and later launched is still the same one.
            let key = format!("task-run:{}:{}", r.task_id, r.queued_at);
            if self.seen(&key)?.is_some() {
                self.already(&key);
                continue;
            }
            let (status, error) = match map_run_status(&r.status, r.error.clone()) {
                Ok(s) => s,
                Err(e) => {
                    self.skip(format!("{file}: record #{} has an {e}, skipped", i + 1));
                    continue;
                }
            };
            let task = self.task_of(&r.task_id)?;
            let cwd_repo = self.repo_by_path(&canon_path(&r.cwd))?.map(|(id, _)| id);
            let (task_id, repo_id, label) = match task {
                Some((id, repo, title)) => (Some(id), cwd_repo.or(Some(repo)), title),
                None => (None, cwd_repo, r.task_id.clone()),
            };
            self.insert_run(
                &key,
                task_id,
                repo_id,
                r.cwd.clone(),
                executor_from_prompt(&r.prompt),
                r.prompt.clone(),
                LaunchOptions::default(),
                parse_finish(r.finish.as_deref()),
                status,
                error,
                r.run_id.clone(),
                r.session_id.clone(),
                r.queued_at,
                r.launched_at,
                label,
            )?;
        }
        Ok(())
    }

    fn import_issue_runs(&mut self, dir: &Path) -> Result<(), String> {
        let file = legacy::ISSUE_RUNS_FILE;
        let raws = self.read_records(dir, file, "runs");
        for (i, v) in raws.into_iter().enumerate() {
            let Some(r) = self.parse::<legacy::IssueRun>(file, i, v) else { continue };
            let key = format!("issue-run:{}:{}", r.issue_id, r.queued_at);
            if self.seen(&key)?.is_some() {
                self.already(&key);
                continue;
            }
            let (status, error) = match map_run_status(&r.status, r.error.clone()) {
                Ok(s) => s,
                Err(e) => {
                    self.skip(format!("{file}: record #{} has an {e}, skipped", i + 1));
                    continue;
                }
            };
            let path = canon_path(&r.cwd);
            let repo_id = self.repo_by_path(&path)?.map(|(id, _)| id);
            let finish = self.finish_by_path.get(&path).copied().unwrap_or(Finish::Pr);
            self.insert_run(
                &key,
                None,
                repo_id,
                r.cwd.clone(),
                Executor::Workflow { name: r.workflow.clone() },
                format!("/{} {}", r.workflow, r.identifier),
                r.options.clone(),
                finish,
                status,
                error,
                r.run_id.clone(),
                r.session_id.clone(),
                r.queued_at,
                r.launched_at,
                r.identifier.clone(),
            )?;
        }
        Ok(())
    }
}

/// Ids that end up in paths (`tasks/<id>/`): `[0-9a-z]`, as the old version generated them.
fn is_safe_id(id: &str) -> bool {
    (2..=40).contains(&id.len()) && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}
