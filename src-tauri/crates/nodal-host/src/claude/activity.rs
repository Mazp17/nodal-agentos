//! Claude Code activity in a repo: sessions (interactive, background, and headless ones
//! that `claude agents` doesn't list) and subagents of ANY session whose cwd/worktree
//! falls inside the repo. Read-only; parsing of Claude Code's internal format lives in
//! `activity_files`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions, RepoActivity, RepoSessions, SessionActivity, SubagentActivity};

use super::activity_files::{self, SubagentFile};
use super::bin as claude_bin;

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// Tail read from each transcript.
const TAIL_BYTES: u64 = 64 * 1024;
/// An unfinished subagent that hasn't written in this long is considered hung/inactive.
/// Long on purpose: a tool (e.g. a build) can take several minutes without writing.
const STALE_MS: i64 = 10 * 60_000;
/// Inactive subagents that are still shown.
const RECENT_SUBAGENT_MS: i64 = 60 * 60_000;
/// Finished sessions (background done/stopped) that are still shown.
const RECENT_SESSION_MS: i64 = 12 * 60 * 60_000;
/// A session `claude agents` doesn't list counts as active if it wrote less than this ago.
const UNLISTED_ACTIVE_MS: i64 = 2 * 60_000;
const MAX_SUBAGENTS: usize = 80;
/// A `.meta.json` larger than this isn't the known format: ignored.
const META_MAX_BYTES: u64 = 64 * 1024;

/// Equivalent repo paths (as is and canonicalized) to compare against what Claude Code
/// writes, which doesn't always canonicalize.
struct RepoRoots(Vec<PathBuf>);

impl RepoRoots {
    /// By components: `/x/nodal-sandbox` is NOT inside `/x/nodal`.
    fn contains(&self, path: &str) -> bool {
        let p = Path::new(path);
        p.is_absolute() && self.0.iter().any(|r| p.starts_with(r))
    }
}

/// Session scanned for subagents.
struct Owner {
    session_id: String,
    name: Option<String>,
    kind: String,
    cwd: Option<String>,
    alive: bool,
    in_repo: bool,
    is_app_run: bool,
    dir: Option<PathBuf>,
}

fn subagent_of(owner: &Owner, f: &SubagentFile, roots: &RepoRoots, now: i64) -> Option<SubagentActivity> {
    let mtime = activity_files::mtime_ms(&f.transcript)?;
    // No recent activity: neither active nor "recent"; not worth reading.
    if now - mtime > RECENT_SUBAGENT_MS {
        return None;
    }
    let meta = f
        .meta
        .as_deref()
        .and_then(|m| activity_files::read_tail(m, META_MAX_BYTES).filter(|_| std::fs::metadata(m).is_ok_and(|x| x.len() <= META_MAX_BYTES)))
        .map(|t| activity_files::parse_meta(&t))
        .unwrap_or_default();
    let tail = activity_files::read_tail(&f.transcript, TAIL_BYTES)
        .map(|t| activity_files::parse_tail(&t))
        .unwrap_or_default();
    let worktree = meta.worktree_path.clone().or(meta.inherited_worktree_path.clone());
    let in_repo = owner.in_repo
        || [worktree.as_deref(), tail.cwd.as_deref()].into_iter().flatten().any(|p| roots.contains(p))
        || tail.touched_paths.iter().any(|p| roots.contains(p));
    if !in_repo {
        return None;
    }
    let active = owner.alive && !tail.finished && now - mtime < STALE_MS;
    Some(SubagentActivity {
        session_id: owner.session_id.clone(),
        agent_id: f.agent_id.clone(),
        description: meta.description,
        agent_type: meta.agent_type,
        cwd: worktree.clone().or(tail.cwd).or(owner.cwd.clone()),
        worktree,
        parent_agent_id: meta.parent_agent_id,
        workflow_id: f.workflow_id.clone(),
        workflow_phase: meta.workflow_phase,
        model: meta.model,
        active,
        finished: tail.finished,
        last_activity_at: Some(mtime),
        last_tool: tail.last_tool,
        last_tool_summary: tail.last_tool_summary,
        session_name: owner.name.clone(),
        session_kind: owner.kind.clone(),
        session_cwd: owner.cwd.clone(),
        session_is_app_run: owner.is_app_run,
    })
}

/// Builds the repo activity. Pure except for reads of `projects` (testable with fixtures).
pub fn assemble(repo_paths: &[PathBuf], agents: &[AgentSession], projects: &Path, app: &AppRuns, now: i64) -> RepoActivity {
    let roots = RepoRoots(repo_paths.to_vec());
    let mut sessions = Vec::new();
    let mut owners = Vec::new();
    let listed: HashSet<&str> = agents.iter().filter_map(|a| a.session_id.as_deref()).collect();

    for a in agents {
        let Some(sid) = a.session_id.clone() else { continue };
        let in_repo = a.cwd.as_deref().is_some_and(|c| roots.contains(c));
        let alive = a.alive();
        let started_at = a.started_at.map(|n| n as i64);
        // Subagents: look at live sessions (from any cwd) and the repo's own.
        if !alive && !in_repo {
            continue;
        }
        // A single lookup per session: the transcript at `<slug(cwd)>/<sid>.jsonl`; only if
        // it's not there is `projects` scanned (and the subagents folder is its sibling).
        let jsonl = activity_files::find_session_jsonl(projects, a.cwd.as_deref(), &sid);
        let last_activity = jsonl.as_deref().and_then(activity_files::mtime_ms);
        // "Recent" by the last write (a long session may have just finished).
        let recent = last_activity.or(started_at).is_some_and(|t| now - t < RECENT_SESSION_MS);
        if !alive && !recent {
            continue;
        }
        let is_app_run = app.contains(a.id.as_deref(), &sid);
        owners.push(Owner {
            session_id: sid.clone(),
            name: a.name.clone(),
            kind: a.kind.clone().unwrap_or_else(|| "unknown".into()),
            cwd: a.cwd.clone(),
            alive,
            in_repo,
            is_app_run,
            dir: jsonl.as_deref().map(|p| p.with_extension("")).filter(|d| d.is_dir()),
        });
        if !in_repo {
            continue;
        }
        let tail = if alive {
            jsonl.as_deref().and_then(|p| activity_files::read_tail(p, TAIL_BYTES)).map(|t| activity_files::parse_tail(&t))
        } else {
            None
        };
        let tail = tail.unwrap_or_default();
        sessions.push(SessionActivity {
            session_id: sid,
            id: a.id.clone(),
            kind: a.kind.clone().unwrap_or_else(|| "unknown".into()),
            name: a.name.clone(),
            cwd: a.cwd.clone(),
            status: a.status.clone(),
            state: a.state.clone(),
            waiting_for: a.waiting_for.clone(),
            started_at,
            pid: a.pid,
            alive,
            is_app_run,
            last_activity_at: last_activity,
            last_tool: tail.last_tool,
            last_tool_summary: tail.last_tool_summary,
            entrypoint: tail.entrypoint,
        });
    }

    // Sessions `claude agents` doesn't list (headless `claude -p`, SDK, old CLIs):
    // freshly written transcripts in the projects of the repo and its worktrees.
    for root in &roots.0 {
        let Some(root) = root.to_str() else { continue };
        let prefix = super::fs::paths::project_slug(root);
        for (sid, path, mtime) in activity_files::recent_session_transcripts(projects, &prefix, now - UNLISTED_ACTIVE_MS) {
            if listed.contains(sid.as_str()) || sessions.iter().any(|s| s.session_id == sid) {
                continue;
            }
            let tail = activity_files::read_tail(&path, TAIL_BYTES).map(|t| activity_files::parse_tail(&t)).unwrap_or_default();
            if !tail.cwd.as_deref().is_some_and(|c| roots.contains(c)) {
                continue;
            }
            let is_app_run = app.contains(None, &sid);
            owners.push(Owner {
                session_id: sid.clone(),
                name: None,
                kind: "unlisted".into(),
                cwd: tail.cwd.clone(),
                alive: true,
                in_repo: true,
                is_app_run,
                dir: Some(path.with_extension("")).filter(|d| d.is_dir()),
            });
            sessions.push(SessionActivity {
                session_id: sid,
                id: None,
                kind: "unlisted".into(),
                name: None,
                cwd: tail.cwd,
                status: Some(if tail.finished { "idle" } else { "busy" }.into()),
                state: None,
                waiting_for: None,
                started_at: None,
                pid: None,
                alive: true,
                is_app_run,
                last_activity_at: Some(mtime),
                last_tool: tail.last_tool,
                last_tool_summary: tail.last_tool_summary,
                entrypoint: tail.entrypoint,
            });
        }
    }

    let mut subagents: Vec<SubagentActivity> = owners
        .iter()
        .filter_map(|o| o.dir.as_deref().map(|d| (o, activity_files::subagent_files(d))))
        .flat_map(|(o, files)| files.into_iter().filter_map(|f| subagent_of(o, &f, &roots, now)).collect::<Vec<_>>())
        .collect();

    sessions.sort_by_key(|s| (!s.alive, std::cmp::Reverse(s.last_activity_at.or(s.started_at))));
    subagents.sort_by_key(|s| (!s.active, std::cmp::Reverse(s.last_activity_at)));
    subagents.truncate(MAX_SUBAGENTS);
    RepoActivity {
        repo_path: repo_paths.first().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        sessions,
        subagents,
        generated_at: now,
    }
}

/// Background and interactive sessions (`claude agents --json --all`, unfiltered).
pub async fn list_agents() -> Result<Vec<AgentSession>, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["agents", "--json", "--all"]);
    let out = claude_bin::output_with_timeout(cmd, LIST_TIMEOUT, "`claude agents`").await?;
    if !out.status.success() {
        return Err(format!("`claude agents` failed: {}", claude_bin::error_text(&out)));
    }
    activity_files::parse_agents(&String::from_utf8_lossy(&out.stdout))
}

/// Existing paths, each one as is and canonicalized, without duplicates.
pub fn existing_roots(raw: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for r in raw {
        if !r.is_dir() {
            continue;
        }
        if let Ok(c) = r.canonicalize() {
            if c != r && !roots.contains(&c) {
                roots.push(c);
            }
        }
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    roots
}

/// Components of the deepest root that contains one of `paths` (0: none).
fn match_depth<'a>(roots: &[PathBuf], paths: impl IntoIterator<Item = Option<&'a str>>) -> usize {
    paths
        .into_iter()
        .flatten()
        .map(Path::new)
        .filter(|p| p.is_absolute())
        .flat_map(|p| roots.iter().filter(move |r| p.starts_with(r)).map(|r| r.components().count()))
        .max()
        .unwrap_or(0)
}

/// Sessions (and their subagents) not launched by the app, from a single agent listing.
/// With nested repos, each one lands only in the deepest repo that contains it: the one
/// whose root matches its path with the most components (ties: the deepest repo, then
/// the first in `repos`). `roots` resolves the paths to compare, as in the shell's
/// `project_counts`.
pub fn external_sessions_of(
    repos: &[(String, PathBuf)],
    agents: &[AgentSession],
    projects: &Path,
    app: &AppRuns,
    now: i64,
    roots: impl Fn(Vec<PathBuf>) -> Vec<PathBuf>,
) -> ExternalSessions {
    let per_repo: Vec<(&String, Vec<PathBuf>, RepoActivity)> = repos
        .iter()
        .filter_map(|(id, path)| {
            let roots = roots(vec![path.clone()]);
            if roots.is_empty() {
                return None;
            }
            let mut act = assemble(&roots, agents, projects, app, now);
            act.sessions.retain(|s| !s.is_app_run);
            act.subagents.retain(|s| !s.session_is_app_run);
            Some((id, roots, act))
        })
        .collect();

    // Best repo for each session and subagent: (score, index in `per_repo`). Higher score
    // wins; on a tie the first repo keeps it.
    let mut best_session: HashMap<&str, ((usize, usize), usize)> = HashMap::new();
    type SubScore = ((usize, usize), usize);
    let mut best_subagent: HashMap<(&str, &str), (SubScore, usize)> = HashMap::new();
    for (i, (_, roots, act)) in per_repo.iter().enumerate() {
        let depth = roots.iter().map(|r| r.components().count()).max().unwrap_or(0);
        for s in &act.sessions {
            let score = (match_depth(roots, [s.cwd.as_deref()]), depth);
            let e = best_session.entry(&s.session_id).or_insert((score, i));
            if score > e.0 {
                *e = (score, i);
            }
        }
        for s in &act.subagents {
            // Where it works first; its session's cwd only breaks ties.
            let own = match_depth(roots, [s.cwd.as_deref(), s.worktree.as_deref()]);
            let score = ((own, match_depth(roots, [s.session_cwd.as_deref()])), depth);
            let e = best_subagent.entry((&s.session_id, &s.agent_id)).or_insert((score, i));
            if score > e.0 {
                *e = (score, i);
            }
        }
    }

    let repos = per_repo
        .iter()
        .enumerate()
        .map(|(i, (id, _, act))| RepoSessions {
            repo_id: (*id).clone(),
            sessions: act.sessions.iter().filter(|s| best_session[s.session_id.as_str()].1 == i).cloned().collect(),
            subagents: act
                .subagents
                .iter()
                .filter(|s| best_subagent[&(s.session_id.as_str(), s.agent_id.as_str())].1 == i)
                .cloned()
                .collect(),
        })
        .filter(|r| !r.sessions.is_empty() || !r.subagents.is_empty())
        .collect();
    ExternalSessions { repos, generated_at: now }
}

#[cfg(test)]
mod tests;
