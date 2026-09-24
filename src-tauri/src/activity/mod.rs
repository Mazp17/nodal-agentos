//! Actividad de Claude Code en un repo: sesiones (interactivas, en background, y
//! headless que `claude agents` no lista) y subagentes de CUALQUIER sesión cuyo
//! cwd/worktree cae dentro del repo. Solo lectura; el parseo del formato interno de
//! Claude Code vive en `claude_sessions.rs`.

mod claude_sessions;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::{with_db, Db};
use crate::runs::{claude_bin, claude_fs};
use claude_sessions::{AgentSession, SubagentFile};

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// Cola leída de cada transcript.
const TAIL_BYTES: u64 = 64 * 1024;
/// Un subagente sin terminar y sin escribir en este tiempo se da por colgado/inactivo.
/// Largo a propósito: una tool (p. ej. un build) puede tardar varios minutos sin escribir.
const STALE_MS: i64 = 10 * 60_000;
/// Subagentes inactivos que se siguen mostrando.
const RECENT_SUBAGENT_MS: i64 = 60 * 60_000;
/// Sesiones terminadas (background done/stopped) que se siguen mostrando.
const RECENT_SESSION_MS: i64 = 12 * 60 * 60_000;
/// Una sesión que `claude agents` no lista cuenta como activa si escribió hace menos de esto.
const UNLISTED_ACTIVE_MS: i64 = 2 * 60_000;
const MAX_SUBAGENTS: usize = 80;
/// Un `.meta.json` más grande que esto no es el formato conocido: se ignora.
const META_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActivity {
    pub session_id: String,
    /// Id corto (solo background).
    pub id: Option<String>,
    /// "interactive" | "background" | "unlisted" (transcript activo que `claude agents`
    /// no lista: `claude -p`, SDK, …).
    pub kind: String,
    pub name: Option<String>,
    pub cwd: Option<String>,
    /// "busy" | "idle" | "waiting" (procesos vivos).
    pub status: Option<String>,
    /// "working" | "blocked" | "done" | "stopped" (background).
    pub state: Option<String>,
    pub waiting_for: Option<String>,
    pub started_at: Option<i64>,
    pub pid: Option<u32>,
    pub alive: bool,
    /// Lanzada desde la app (issue o tarea).
    pub is_app_run: bool,
    /// mtime del transcript.
    pub last_activity_at: Option<i64>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    /// "cli", "sdk-cli", … (del transcript).
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentActivity {
    pub session_id: String,
    pub agent_id: String,
    pub description: Option<String>,
    pub agent_type: Option<String>,
    /// Worktree propio o heredado; si no, el último cwd del transcript.
    pub cwd: Option<String>,
    pub worktree: Option<String>,
    pub parent_agent_id: Option<String>,
    pub workflow_id: Option<String>,
    pub workflow_phase: Option<String>,
    pub model: Option<String>,
    pub active: bool,
    /// Terminó (último mensaje con `end_turn`). `active = false && !finished` = sin
    /// actividad hace más de 10 min, o su sesión ya no corre.
    pub finished: bool,
    pub last_activity_at: Option<i64>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    /// Sesión dueña.
    pub session_name: Option<String>,
    pub session_kind: String,
    pub session_cwd: Option<String>,
    pub session_is_app_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoActivity {
    pub repo_path: String,
    pub sessions: Vec<SessionActivity>,
    pub subagents: Vec<SubagentActivity>,
    /// Epoch ms del cálculo.
    pub generated_at: i64,
}

/// Runs lanzados por la app: ids cortos y sessionIds.
#[derive(Debug, Default, Clone)]
pub struct AppRuns {
    pub run_ids: HashSet<String>,
    pub session_ids: HashSet<String>,
}

impl AppRuns {
    fn contains(&self, id: Option<&str>, session_id: &str) -> bool {
        self.session_ids.contains(session_id) || id.is_some_and(|i| self.run_ids.contains(i))
    }
    fn add(&mut self, run_id: Option<String>, session_id: Option<String>) {
        if let Some(r) = run_id {
            self.run_ids.insert(r);
        }
        if let Some(s) = session_id {
            self.session_ids.insert(s);
        }
    }
}

/// Rutas equivalentes del repo (tal cual y canonicalizada) para comparar contra lo que
/// escribe Claude Code, que no siempre canonicaliza.
struct RepoRoots(Vec<PathBuf>);

impl RepoRoots {
    /// Por componentes: `/x/nodal-sandbox` NO está dentro de `/x/agent-desk`.
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
    let mtime = claude_sessions::mtime_ms(&f.transcript)?;
    // Sin actividad reciente: ni activo ni "reciente"; no vale la pena leerlo.
    if now - mtime > RECENT_SUBAGENT_MS {
        return None;
    }
    let meta = f
        .meta
        .as_deref()
        .and_then(|m| claude_sessions::read_tail(m, META_MAX_BYTES).filter(|_| std::fs::metadata(m).is_ok_and(|x| x.len() <= META_MAX_BYTES)))
        .map(|t| claude_sessions::parse_meta(&t))
        .unwrap_or_default();
    let tail = claude_sessions::read_tail(&f.transcript, TAIL_BYTES)
        .map(|t| claude_sessions::parse_tail(&t))
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

/// Arma la actividad del repo. Pura salvo lecturas de `projects` (testeable con fixtures).
fn assemble(repo_paths: &[PathBuf], agents: &[AgentSession], projects: &Path, app: &AppRuns, now: i64) -> RepoActivity {
    let roots = RepoRoots(repo_paths.to_vec());
    let mut sessions = Vec::new();
    let mut owners = Vec::new();
    let listed: HashSet<&str> = agents.iter().filter_map(|a| a.session_id.as_deref()).collect();

    for a in agents {
        let Some(sid) = a.session_id.clone() else { continue };
        let in_repo = a.cwd.as_deref().is_some_and(|c| roots.contains(c));
        let alive = a.alive();
        let started_at = a.started_at.map(|n| n as i64);
        // Subagentes: se miran las sesiones vivas (de cualquier cwd) y las del repo.
        if !alive && !in_repo {
            continue;
        }
        // Un solo lookup por sesión: el transcript en `<slug(cwd)>/<sid>.jsonl`; solo si
        // no está ahí se recorre `projects` (y la carpeta de subagentes es su hermana).
        let jsonl = claude_sessions::find_session_jsonl(projects, a.cwd.as_deref(), &sid);
        let last_activity = jsonl.as_deref().and_then(claude_sessions::mtime_ms);
        // "Reciente" por la última escritura (una sesión larga puede haber terminado recién).
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
            jsonl.as_deref().and_then(|p| claude_sessions::read_tail(p, TAIL_BYTES)).map(|t| claude_sessions::parse_tail(&t))
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

    // Sesiones que `claude agents` no lista (headless `claude -p`, SDK, CLIs viejos):
    // transcripts recién escritos en los proyectos del repo y sus worktrees.
    for root in &roots.0 {
        let Some(root) = root.to_str() else { continue };
        let prefix = claude_fs::project_slug(root);
        for (sid, path, mtime) in claude_sessions::recent_session_transcripts(projects, &prefix, now - UNLISTED_ACTIVE_MS) {
            if listed.contains(sid.as_str()) || sessions.iter().any(|s| s.session_id == sid) {
                continue;
            }
            let tail = claude_sessions::read_tail(&path, TAIL_BYTES).map(|t| claude_sessions::parse_tail(&t)).unwrap_or_default();
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
        .filter_map(|o| o.dir.as_deref().map(|d| (o, claude_sessions::subagent_files(d))))
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

/// Cuántas sesiones y subagentes están trabajando ahora en un conjunto de repos.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySummary {
    /// Sesiones vivas trabajando o esperando al usuario.
    pub sessions: u32,
    /// Subagentes activos.
    pub agents: u32,
    pub generated_at: i64,
}

/// Mismo criterio que el panel (`sessionState(...).live` en RepoActivityPanel).
fn summarize(act: &RepoActivity) -> ActivitySummary {
    let live = |s: &SessionActivity| {
        s.alive
            && (matches!(s.state.as_deref(), Some("working" | "blocked"))
                || matches!(s.status.as_deref(), Some("busy" | "waiting")))
    };
    ActivitySummary {
        sessions: act.sessions.iter().filter(|s| live(s)).count() as u32,
        agents: act.subagents.iter().filter(|a| a.active).count() as u32,
        generated_at: act.generated_at,
    }
}

/// Runs lanzados por la app (tabla `runs`). Si la base no está disponible, no se marca nada.
async fn app_run_refs(app: &AppHandle) -> AppRuns {
    let mut refs = AppRuns::default();
    let Some(db) = app.try_state::<Db>() else { return refs };
    match with_db(&db, |c| crate::db::queries::runs::launched_refs(c)).await {
        Ok(list) => {
            for (r, s) in list {
                refs.add(r, s);
            }
        }
        Err(e) => eprintln!("activity: {e}"),
    }
    refs
}

async fn list_agents() -> Result<Vec<AgentSession>, String> {
    let mut cmd = claude_bin::claude_command()?;
    cmd.args(["agents", "--json", "--all"]);
    let out = claude_bin::output_with_timeout(cmd, LIST_TIMEOUT, "`claude agents`").await?;
    if !out.status.success() {
        return Err(format!("`claude agents` failed: {}", claude_bin::error_text(&out)));
    }
    claude_sessions::parse_agents(&String::from_utf8_lossy(&out.stdout))
}

/// Sesiones y subagentes de Claude Code trabajando en `repo_path` (o sus worktrees).
#[tauri::command]
pub async fn repo_activity(app: AppHandle, repo_path: String) -> Result<RepoActivity, String> {
    let raw = PathBuf::from(repo_path.trim());
    if raw.as_os_str().is_empty() || !raw.is_absolute() {
        return Err(format!("The repository path must be absolute: {repo_path}"));
    }
    let agents = list_agents().await?;
    let refs = app_run_refs(&app).await;
    tauri::async_runtime::spawn_blocking(move || {
        let mut roots = vec![raw.clone()];
        if let Ok(c) = raw.canonicalize() {
            if c != raw {
                roots.push(c);
            }
        }
        if !roots.iter().any(|r| r.is_dir()) {
            return Err(format!("The repository folder doesn't exist: {}", raw.display()));
        }
        let projects = claude_fs::claude_config_dir()
            .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
            .join("projects");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Ok(assemble(&roots, &agents, &projects, &refs, now))
    })
    .await
    .map_err(|e| format!("Internal error reading activity: {e}"))?
}

/// Resumen de actividad de varios repos en una sola pasada (un `claude agents` y un
/// recorrido de `projects`), para el indicador del sidebar. Las rutas que no existen se
/// ignoran; sin rutas válidas devuelve ceros.
#[tauri::command]
pub async fn activity_summary(repo_paths: Vec<String>) -> Result<ActivitySummary, String> {
    let raw: Vec<PathBuf> = repo_paths
        .iter()
        .map(|p| PathBuf::from(p.trim()))
        .filter(|p| p.is_absolute())
        .collect();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    };
    if raw.is_empty() {
        return Ok(ActivitySummary { sessions: 0, agents: 0, generated_at: now() });
    }
    let agents = list_agents().await?;
    tauri::async_runtime::spawn_blocking(move || {
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
        if roots.is_empty() {
            return Ok(ActivitySummary { sessions: 0, agents: 0, generated_at: now() });
        }
        let projects = claude_fs::claude_config_dir()
            .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
            .join("projects");
        Ok(summarize(&assemble(&roots, &agents, &projects, &AppRuns::default(), now())))
    })
    .await
    .map_err(|e| format!("Internal error reading activity: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/activity/fixtures")
    }

    const COORD: &str = "11111111-2222-3333-4444-555555555555";
    const BG: &str = "99999999-0000-0000-0000-000000000001";

    /// Las ventanas de tiempo dependen del mtime: se copian los fixtures a un temporal
    /// (mtime = ahora) y se ajusta el de los que tienen que verse viejos.
    fn setup(name: &str) -> (PathBuf, i64) {
        let dst = std::env::temp_dir().join(format!("agent-desk-activity-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        copy_dir(&fixtures().join("projects"), &dst);
        let old = std::time::SystemTime::now() - Duration::from_secs(3 * 3600);
        let rel = "-Users-me-Code/11111111-2222-3333-4444-555555555555/subagents/agent-aold00000000001.jsonl";
        let f = std::fs::File::options().append(true).open(dst.join(rel)).unwrap();
        f.set_modified(old).unwrap();
        let now = claude_sessions::mtime_ms(&dst.join("-Users-me-Code-repo").join(format!("{BG}.jsonl"))).unwrap() + 1_000;
        (dst, now)
    }

    fn copy_dir(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            let to = dst.join(e.file_name());
            if e.path().is_dir() {
                copy_dir(&e.path(), &to);
            } else {
                std::fs::copy(e.path(), &to).unwrap();
                // En macOS `fs::copy` conserva el mtime del origen (el del checkout): se fija a
                // ahora para que las ventanas de tiempo no dependan de cuándo se clonó el repo.
                let f = std::fs::File::options().append(true).open(&to).unwrap();
                f.set_modified(std::time::SystemTime::now()).unwrap();
            }
        }
    }

    /// `startedAt` relativo a `now`: la terminada del repo es de hace 1 h (reciente).
    fn agents(now: i64) -> Vec<AgentSession> {
        let mut a =
            claude_sessions::parse_agents(&std::fs::read_to_string(fixtures().join("agents.json")).unwrap()).unwrap();
        for s in a.iter_mut() {
            s.started_at = Some((now - 3_600_000) as f64);
        }
        a
    }

    #[test]
    fn repo_activity_from_fixtures() {
        let (projects, now) = setup("main");
        let mut app = AppRuns::default();
        app.add(Some("99999999".into()), None);
        let act = assemble(&[PathBuf::from("/Users/me/Code/repo")], &agents(now), &projects, &app, now);

        let ids: Vec<(&str, &str, bool)> =
            act.sessions.iter().map(|s| (s.session_id.as_str(), s.kind.as_str(), s.alive)).collect();
        // Background working del repo, interactiva en un worktree del repo, headless no
        // listada, background terminada reciente. Nada de la sandbox ni del coordinador.
        assert!(ids.contains(&(BG, "background", true)), "{ids:?}");
        assert!(ids.contains(&("77777777-0000-0000-0000-000000000003", "interactive", true)), "{ids:?}");
        assert!(ids.contains(&("88888888-aaaa-bbbb-cccc-000000000001", "unlisted", true)), "{ids:?}");
        assert!(ids.contains(&("43495c2e-056b-4f96-8cab-4dd89a61e005", "background", false)), "{ids:?}");
        assert_eq!(ids.len(), 4, "{ids:?}");
        assert!(!ids[3].2, "vivas primero");

        let bg = act.sessions.iter().find(|s| s.session_id == BG).unwrap();
        assert!(bg.is_app_run);
        assert_eq!(bg.last_tool.as_deref(), Some("Read"));
        let unlisted = act.sessions.iter().find(|s| s.kind == "unlisted").unwrap();
        assert_eq!(unlisted.entrypoint.as_deref(), Some("sdk-cli"));
        assert!(!unlisted.is_app_run);

        let subs: Vec<(&str, bool)> = act.subagents.iter().map(|s| (s.agent_id.as_str(), s.active)).collect();
        // Del coordinador (cwd fuera del repo): el del worktree, su hijo con worktree
        // heredado y el que hizo cd al repo. No el terminado fuera del repo ni el viejo.
        assert!(subs.contains(&("alive0000000001", true)), "{subs:?}");
        assert!(subs.contains(&("achild000000001", false)), "{subs:?}");
        assert!(subs.contains(&("acdrepo00000001", true)), "{subs:?}");
        // Workflow agent de la sesión background del repo.
        assert!(subs.contains(&("aworkflow000000001", true)), "{subs:?}");
        assert_eq!(subs.len(), 4, "{subs:?}");
        assert!(act.subagents[..3].iter().all(|s| s.active), "activos primero");

        let alive = act.subagents.iter().find(|s| s.agent_id == "alive0000000001").unwrap();
        assert_eq!(alive.session_id, COORD);
        assert_eq!(alive.session_kind, "interactive");
        assert_eq!(alive.worktree.as_deref(), Some("/Users/me/Code/repo/.claude/worktrees/agent-alive0000000001"));
        assert_eq!(alive.description.as_deref(), Some("Local tasks backend"));
        assert_eq!(alive.last_tool.as_deref(), Some("Bash"));
        let child = act.subagents.iter().find(|s| s.agent_id == "achild000000001").unwrap();
        assert!(child.finished);
        assert_eq!(child.parent_agent_id.as_deref(), Some("alive0000000001"));
        let wf = act.subagents.iter().find(|s| s.agent_id == "aworkflow000000001").unwrap();
        assert_eq!(wf.workflow_id.as_deref(), Some("wf_1234"));
        assert!(wf.session_is_app_run);

        std::fs::remove_dir_all(&projects).unwrap();
    }

    #[test]
    fn summary_counts_live_sessions_and_active_agents_across_repos() {
        let (projects, now) = setup("summary");
        let repo = PathBuf::from("/Users/me/Code/repo");
        let one = summarize(&assemble(std::slice::from_ref(&repo), &agents(now), &projects, &AppRuns::default(), now));
        // alive0000000001, acdrepo00000001 y aworkflow000000001.
        assert_eq!(one.agents, 3, "{one:?}");
        assert_eq!(one.sessions, 2, "background working + headless: {one:?}");
        // Sumar la sandbox agrega (al menos) su sesión bloqueada esperando permiso.
        let sandbox = PathBuf::from("/Users/me/Code/repo-sandbox");
        let both = summarize(&assemble(&[repo, sandbox], &agents(now), &projects, &AppRuns::default(), now));
        assert!(both.sessions > one.sessions, "{one:?} {both:?}");
        std::fs::remove_dir_all(&projects).unwrap();
    }

    #[test]
    fn dead_session_has_no_active_subagents_and_stale_ones_expire() {
        let (projects, now) = setup("dead");
        let mut agents = agents(now);
        // El coordinador murió: sus subagentes quedan, pero inactivos.
        let coord = agents.iter_mut().find(|a| a.session_id.as_deref() == Some(COORD)).unwrap();
        coord.pid = None;
        coord.status = None;
        let act = assemble(&[PathBuf::from("/Users/me/Code/repo")], &agents, &projects, &AppRuns::default(), now);
        assert!(act.subagents.iter().all(|s| s.session_id != COORD), "sesión muerta fuera del repo: no se escanea");

        // Pasados 10 min sin escribir, un subagente sin `end_turn` deja de contar como activo.
        let act = assemble(&[PathBuf::from("/Users/me/Code/repo")], &self::agents(now), &projects, &AppRuns::default(), now + STALE_MS + 1);
        assert!(act.subagents.iter().all(|s| !s.active), "{:?}", act.subagents);
        std::fs::remove_dir_all(&projects).unwrap();
    }

    #[test]
    fn repo_prefix_is_not_containment() {
        let roots = RepoRoots(vec![PathBuf::from("/Users/me/Code/repo")]);
        assert!(roots.contains("/Users/me/Code/repo"));
        assert!(roots.contains("/Users/me/Code/repo/.claude/worktrees/x"));
        assert!(!roots.contains("/Users/me/Code/repo-sandbox"));
        assert!(!roots.contains("/Users/me/Code"));
        assert!(!roots.contains("repo"));
    }

    /// Contra los datos reales de esta máquina (solo lectura): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_repo_activity() {
        let agents = tauri::async_runtime::block_on(list_agents()).expect("claude agents");
        let projects = claude_fs::claude_config_dir().unwrap().join("projects");
        let repo = std::env::var("ACTIVITY_REPO").unwrap_or_else(|_| "/Users/me/Code/agent-desk".into());
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let act = assemble(&[PathBuf::from(&repo)], &agents, &projects, &AppRuns::default(), now);
        for s in &act.sessions {
            eprintln!("session {} {} {:?} alive={} {:?} {:?}", s.session_id, s.kind, s.name, s.alive, s.status, s.last_tool);
        }
        for s in &act.subagents {
            eprintln!(
                "subagent {} {:?} {:?} active={} finished={} session={} cwd={:?} tool={:?}",
                s.agent_id, s.agent_type, s.description, s.active, s.finished, s.session_id, s.cwd, s.last_tool
            );
        }
    }
}
