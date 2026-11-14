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

/// `pr` (a number or a GitHub PR URL) as `gh pr view` reports it.
pub fn pr_view(dir: &Path, pr: &str) -> Result<TaskPr, String> {
    let url_ok = pr.strip_prefix("https://github.com/").is_some_and(|r| r.contains("/pull/") && !r.contains(char::is_whitespace));
    if !(url_ok || !pr.is_empty() && pr.bytes().all(|b| b.is_ascii_digit())) {
        return Err("Enter a PR number or a github.com pull request URL.".into());
    }
    let out = gh(dir, &["pr", "view", pr, "--json", "number,url,state,isDraft,headRefOid"])?;
    if !out.ok {
        return Err(clip(&out.stderr));
    }
    parse_pr_item(&out.stdout)
}

/// The PR's comments, reviews and checks as `gh` reports them.
pub fn pr_details(dir: &Path, number: i64) -> Result<serde_json::Value, String> {
    let n = number.to_string();
    let fields = "number,title,url,state,isDraft,headRefName,reviewDecision,mergeable,author,comments,reviews,statusCheckRollup";
    let out = gh(dir, &["pr", "view", &n, "--json", fields])?;
    if !out.ok {
        return Err(clip(&out.stderr));
    }
    let mut v: serde_json::Value = serde_json::from_str(&out.stdout).map_err(|e| format!("Unexpected gh output: {e}"))?;
    // Inline (file/line) review comments aren't in `gh pr view`; best effort, `[]` when it fails.
    let inline = v["url"].as_str().and_then(repo_of_pr_url).and_then(|repo| {
        let path = format!("repos/{repo}/pulls/{n}/comments?per_page=100");
        let jq = "map({author: {login: .user.login}, body, path, line: (.line // .original_line), createdAt: .created_at, url: .html_url})";
        let out = gh(dir, &["api", &path, "--jq", jq]).ok().filter(|o| o.ok)?;
        serde_json::from_str::<serde_json::Value>(&out.stdout).ok()
    });
    v["reviewComments"] = inline.unwrap_or_else(|| serde_json::json!([]));
    Ok(v)
}

/// `gh pr merge` with `method` (squash, merge or rebase). The branch stays: a worktree may use it.
pub fn merge_pr(dir: &Path, number: i64, method: &str) -> Result<(), String> {
    let flag = match method {
        "squash" => "--squash",
        "merge" => "--merge",
        "rebase" => "--rebase",
        _ => return Err(format!("Unknown merge method {method}.")),
    };
    let out = gh(dir, &["pr", "merge", &number.to_string(), flag])?;
    if out.ok {
        return Ok(());
    }
    let msg = if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr };
    Err(clip(msg))
}

/// `https://github.com/o/r/pull/12` → `o/r`; `None` for anything else.
fn repo_of_pr_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    (ok(owner) && ok(repo) && parts.next() == Some("pull")).then(|| format!("{owner}/{repo}"))
}

/// The last 200 lines of a run's failed-step log.
pub fn check_log(dir: &Path, run_id: &str) -> Result<String, String> {
    if run_id.is_empty() || !run_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid run id.".into());
    }
    let out = gh(dir, &["run", "view", run_id, "--log-failed"])?;
    if !out.ok {
        return Err(clip(&out.stderr));
    }
    let lines: Vec<&str> = out.stdout.lines().collect();
    Ok(lines[lines.len().saturating_sub(200)..].join("\n"))
}

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

fn parse_pr_list(json: &str) -> Result<Option<TaskPr>, String> {
    let items: Vec<Item> = serde_json::from_str(json).map_err(|e| format!("Unexpected gh output: {e}"))?;
    items.into_iter().next().map(to_pr).transpose()
}

fn parse_pr_item(json: &str) -> Result<TaskPr, String> {
    to_pr(serde_json::from_str(json).map_err(|e| format!("Unexpected gh output: {e}"))?)
}

fn to_pr(i: Item) -> Result<TaskPr, String> {
    let state = match i.state.as_str() {
        "OPEN" if i.is_draft => PrState::Draft,
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        other => return Err(format!("Unknown PR state {other}")),
    };
    let oid = i.head_ref_oid;
    let head = (oid.len() == 40 && oid.bytes().all(|b| b.is_ascii_hexdigit())).then_some(oid);
    Ok(TaskPr { url: i.url, number: i.number, state, head })
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
    fn rejects_unknown_merge_methods() {
        assert!(merge_pr(Path::new("."), 1, "--admin").unwrap_err().contains("Unknown merge method"));
    }

    #[test]
    fn reads_the_repo_of_a_pr_url() {
        assert_eq!(repo_of_pr_url("https://github.com/o/r-1/pull/12").as_deref(), Some("o/r-1"));
        assert_eq!(repo_of_pr_url("https://github.com/o/r/issues/12"), None);
        assert_eq!(repo_of_pr_url("https://github.com/o/r?x/pull/1"), None);
        assert_eq!(repo_of_pr_url("https://evil.com/o/r/pull/1"), None);
    }

    #[test]
    fn rejects_option_like_branches() {
        assert!(pr_for_branch(Path::new("."), "--web").is_err());
    }

    #[test]
    fn parses_gh_pr_view() {
        let json = format!(r#"{{"headRefOid":"{OID}","isDraft":true,"number":7,"state":"OPEN","url":"https://github.com/o/r/pull/7"}}"#);
        let pr = parse_pr_item(&json).unwrap();
        assert_eq!((pr.number, pr.state, pr.head.as_deref()), (7, PrState::Draft, Some(OID)));
        assert!(parse_pr_item("[]").is_err());
    }

    #[test]
    fn rejects_option_like_pr_and_run_args() {
        let d = Path::new(".");
        assert!(pr_view(d, "--web").is_err());
        assert!(pr_view(d, "-1").is_err());
        assert!(pr_view(d, "").is_err());
        assert!(check_log(d, "1;x").is_err());
        assert!(check_log(d, "--web").is_err());
    }

    #[cfg(unix)]
    mod fake_gh {
        use super::*;
        use crate::testutil::{fake_gh_calls, fake_gh_fail, fake_gh_reply, install_fake_gh, TempDir};

        const VIEW: &str = r#"{"headRefOid":"","isDraft":false,"number":7,"state":"OPEN","url":"https://github.com/o/r/pull/7"}"#;

        #[test]
        fn pr_view_accepts_numbers_and_urls_and_reports_gh_errors() {
            install_fake_gh();
            let t = TempDir::new("pr-view");
            fake_gh_reply(&t.0, "pr_view", VIEW);
            for arg in ["7", "https://github.com/o/r/pull/7"] {
                let pr = pr_view(&t.0, arg).unwrap();
                assert_eq!((pr.number, pr.state, pr.head), (7, PrState::Open, None));
            }
            fake_gh_fail(&t.0, "pr_view", "no pull requests found\n");
            assert_eq!(pr_view(&t.0, "7").unwrap_err(), "no pull requests found");
        }

        #[test]
        fn pr_for_branch_lists_the_newest_pr() {
            install_fake_gh();
            let t = TempDir::new("pr-list");
            fake_gh_reply(&t.0, "pr_list", &format!("[{VIEW}]"));
            assert_eq!(pr_for_branch(&t.0, "feat").unwrap().unwrap().number, 7);
            fake_gh_reply(&t.0, "pr_list", "[]");
            assert_eq!(pr_for_branch(&t.0, "feat").unwrap(), None);
            fake_gh_fail(&t.0, "pr_list", "not logged in");
            assert_eq!(pr_for_branch(&t.0, "feat").unwrap_err(), "not logged in");
        }

        #[test]
        fn pr_details_adds_inline_comments_when_the_api_answers() {
            install_fake_gh();
            let t = TempDir::new("pr-details");
            fake_gh_reply(&t.0, "pr_view", r#"{"number":7,"url":"https://github.com/o/r/pull/7","comments":[]}"#);
            // `gh api` failing is best effort: an empty list.
            let v = pr_details(&t.0, 7).unwrap();
            assert_eq!((v["number"].as_i64(), &v["reviewComments"]), (Some(7), &serde_json::json!([])));
            fake_gh_reply(&t.0, "api", r#"[{"body":"nit"}]"#);
            assert_eq!(pr_details(&t.0, 7).unwrap()["reviewComments"][0]["body"], "nit");
            fake_gh_reply(&t.0, "pr_view", "not json");
            assert!(pr_details(&t.0, 7).unwrap_err().contains("Unexpected gh output"));
            fake_gh_fail(&t.0, "pr_view", "boom");
            assert_eq!(pr_details(&t.0, 7).unwrap_err(), "boom");
        }

        #[test]
        fn merge_pr_passes_the_method_flag_and_surfaces_failures() {
            install_fake_gh();
            let t = TempDir::new("pr-merge");
            fake_gh_reply(&t.0, "pr_merge", "");
            merge_pr(&t.0, 7, "squash").unwrap();
            let calls = std::fs::read_to_string(t.0.join(".git/fake-gh/calls")).unwrap();
            assert_eq!(calls.trim(), "pr merge 7 --squash");
            fake_gh_fail(&t.0, "pr_merge", "not mergeable\n");
            assert_eq!(merge_pr(&t.0, 7, "rebase").unwrap_err(), "not mergeable");
            assert_eq!(fake_gh_calls(&t.0), 2);
        }

        #[test]
        fn check_log_keeps_the_last_200_lines() {
            install_fake_gh();
            let t = TempDir::new("check-log");
            let log: String = (1..=250).map(|i| format!("line {i}\n")).collect();
            fake_gh_reply(&t.0, "run_view", &log);
            let out = check_log(&t.0, "123").unwrap();
            assert_eq!((out.lines().count(), out.lines().next(), out.lines().last()), (200, Some("line 51"), Some("line 250")));
            fake_gh_fail(&t.0, "run_view", "run not found");
            assert_eq!(check_log(&t.0, "123").unwrap_err(), "run not found");
        }
    }
}
