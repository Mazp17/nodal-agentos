//! Commit everything and open a pull request with the GitHub CLI. `git` goes through
//! `super::run`; `gh` is spawned here with an argument array (title and body are user input).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use nodal_domain::diff::CommitInfo;

use super::{diff, ok, run, wait_for_output, GitOutput, GIT_TIMEOUT};
use crate::claude::bin as claude_bin;

const NO_GH: &str = "Install the GitHub CLI (gh) and run `gh auth login`.";

fn clip(s: &str) -> String {
    nodal_domain::util::clip_chars(s.trim(), 600)
}

/// `git add -A` plus `git commit` in `dir`; returns the new commit.
/// Commits every change in `dir`, or only `paths` (relative) when some are given.
pub fn commit_all(dir: &Path, message: &str, paths: &[String]) -> Result<CommitInfo, String> {
    if let Some(bad) = paths.iter().find(|p| p.is_empty() || p.starts_with('/') || p.starts_with('-') || p.split('/').any(|c| c == "..")) {
        return Err(format!("Invalid path: {bad}"));
    }
    let only: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut add = vec!["add", "-A", "--"];
    add.extend(&only);
    ok(dir, &add)?;
    let mut staged = vec!["diff", "--cached", "--name-only", "--"];
    staged.extend(&only);
    if run(dir, &staged)?.stdout.trim().is_empty() {
        return Err("Nothing to commit.".into());
    }
    // `--only` with paths leaves whatever else is staged out of this commit.
    let mut commit = vec!["commit", "-q", "-m", message];
    if !only.is_empty() {
        commit.extend(["--only", "--"]);
        commit.extend(&only);
    }
    let out = run(dir, &commit)?;
    if !out.ok {
        let msg = if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr };
        return Err(format!("Commit failed: {}", clip(msg)));
    }
    diff::log(dir, None)?.into_iter().next().ok_or_else(|| "Committed, but couldn't read the commit.".into())
}

/// The remote's default branch (`origin/HEAD`), when it is known locally.
pub fn default_branch(dir: &Path) -> Option<String> {
    let out = ok(dir, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).ok()?;
    out.trim().strip_prefix("origin/").map(str::to_string).filter(|b| !b.is_empty())
}

fn gh(dir: &Path, args: &[&str]) -> Result<GitOutput, String> {
    let bin = claude_bin::resolve_bin("gh").ok_or(NO_GH)?;
    let child = Command::new(bin)
        .current_dir(dir)
        .args(args)
        .env("PATH", claude_bin::augmented_path())
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run gh: {e}"))?;
    wait_for_output(child, Instant::now() + GIT_TIMEOUT, "gh")
}

/// Pushes `branch` to `origin` and opens a PR against `base`; if one already exists for the
/// branch, returns its URL. Returns the PR URL.
pub fn open_pr(dir: &Path, base: &str, branch: &str, title: &str, body: &str) -> Result<String, String> {
    if base.starts_with('-') || branch.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    claude_bin::resolve_bin("gh").ok_or(NO_GH)?;
    if !run(dir, &["remote", "get-url", "origin"])?.ok {
        return Err("This repo has no `origin` remote to push to.".into());
    }
    ok(dir, &["push", "-q", "-u", "--", "origin", branch])?;
    let out = gh(dir, &["pr", "create", "--base", base, "--head", branch, "--title", title, "--body", body])?;
    if out.ok {
        return last_line(&out.stdout);
    }
    if out.stderr.contains("already exists") {
        let view = gh(dir, &["pr", "view", "--json", "url", "-q", ".url"])?;
        if view.ok {
            return last_line(&view.stdout);
        }
    }
    let msg = if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr };
    Err(clip(msg))
}

fn last_line(s: &str) -> Result<String, String> {
    s.lines().rev().map(str::trim).find(|l| !l.is_empty()).map(str::to_string).ok_or_else(|| "gh returned no PR URL.".into())
}
