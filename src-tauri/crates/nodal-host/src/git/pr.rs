//! Commit everything and open a pull request with the GitHub CLI. `git` goes through
//! `super::run`; `gh` is spawned here with an argument array (title and body are user input).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use nodal_domain::diff::CommitInfo;
use nodal_domain::model::{PrState, TaskPr};

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
    let mut cmd = Command::new(bin);
    cmd.current_dir(dir)
        .args(args)
        .env("PATH", claude_bin::augmented_path())
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    claude_bin::no_window_std(&mut cmd);
    let child = claude_bin::spawn_retrying(|| cmd.spawn()).map_err(|e| format!("Couldn't run gh: {e}"))?;
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

/// The newest PR (any state) whose head is `branch`, looked up with `gh pr list` in `dir`.
/// `Err` when `gh` is missing, not logged in or the repo isn't on GitHub: callers treat it as
/// "unknown", not as a failure to show.
pub fn pr_for_branch(dir: &Path, branch: &str) -> Result<Option<TaskPr>, String> {
    if branch.is_empty() || branch.starts_with('-') {
        return Err("Invalid branch name.".into());
    }
    let out = gh(dir, &["pr", "list", "--head", branch, "--state", "all", "--json", "number,url,state,isDraft,headRefOid", "--limit", "1"])?;
    if !out.ok {
        return Err(clip(&out.stderr));
    }
    let mut pr = parse_pr_list(&out.stdout)?;
    // A merged PR's head must be here to diff from it; fetch it if it was pushed from elsewhere.
    if let Some(p) = pr.as_mut().filter(|p| p.state == PrState::Merged) {
        let has = |oid: &str| run(dir, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok_and(|o| o.ok);
        if let Some(oid) = p.head.clone().filter(|o| !has(o)) {
            let _ = run(dir, &["fetch", "-q", "origin", &oid]);
            if !has(&oid) {
                p.head = None;
            }
        }
    }
    Ok(pr)
}

fn parse_pr_list(json: &str) -> Result<Option<TaskPr>, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        number: i64,
        url: String,
        state: String,
        #[serde(default)]
        is_draft: bool,
        #[serde(default)]
        head_ref_oid: String,
    }
    let items: Vec<Item> = serde_json::from_str(json).map_err(|e| format!("Unexpected gh output: {e}"))?;
    let Some(i) = items.into_iter().next() else { return Ok(None) };
    let state = match i.state.as_str() {
        "OPEN" if i.is_draft => PrState::Draft,
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        other => return Err(format!("Unknown PR state {other}")),
    };
    let oid = i.head_ref_oid;
    let head = (oid.len() == 40 && oid.bytes().all(|b| b.is_ascii_hexdigit())).then_some(oid);
    Ok(Some(TaskPr { url: i.url, number: i.number, state, head }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "5b756e7a1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f";

    fn one(state: &str, draft: bool) -> Option<TaskPr> {
        let json = format!(r#"[{{"headRefOid":"{OID}","isDraft":{draft},"number":12,"state":"{state}","url":"https://github.com/o/r/pull/12"}}]"#);
        parse_pr_list(&json).unwrap()
    }

    #[test]
    fn parses_gh_pr_list() {
        assert_eq!(one("OPEN", false).unwrap().state, PrState::Open);
        assert_eq!(one("OPEN", true).unwrap().state, PrState::Draft);
        assert_eq!(one("MERGED", false).unwrap().state, PrState::Merged);
        assert_eq!(one("CLOSED", false).unwrap().state, PrState::Closed);
        let pr = one("OPEN", false).unwrap();
        assert_eq!((pr.number, pr.url.as_str(), pr.head.as_deref()), (12, "https://github.com/o/r/pull/12", Some(OID)));
        let bad = r#"[{"headRefOid":"--upload-pack=x","number":1,"state":"MERGED","url":"u"}]"#;
        assert_eq!(parse_pr_list(bad).unwrap().unwrap().head, None);
        assert_eq!(parse_pr_list("[]\n").unwrap(), None);
        assert!(parse_pr_list("not json").is_err());
    }

    #[test]
    fn rejects_option_like_branches() {
        assert!(pr_for_branch(Path::new("."), "--web").is_err());
    }
}
