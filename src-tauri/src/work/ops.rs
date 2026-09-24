//! CRUD síncrono de proyectos, repos, tareas, relaciones y settings. Cada función recibe la
//! conexión (los comandos la corren con `with_db`) y devuelve errores listos para mostrar.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::db::queries::{projects, relations, repos, runs as qruns, tasks};
use crate::db::rows;
use crate::domain::*;
use crate::providers;
use crate::runs::options;
use crate::util::{new_id, write_atomic};

use super::dto::*;
use super::transitions::{apply_status, outbox_ops, OutboxOp};
use super::{validate, Env};

// ---------- Proyectos ----------

pub fn create_project(conn: &Connection, input: &NewProject, now: i64) -> Result<Project, String> {
    let name = validate::name(&input.name, "project")?;
    let key = validate::project_key(&input.key)?;
    let color = match input.color.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => validate::color(c)?,
        None => validate::PALETTE[projects::count(conn)? as usize % validate::PALETTE.len()].to_string(),
    };
    let p = Project {
        id: new_id('p', now),
        name,
        key,
        next_task_number: 1,
        color,
        default_executor: None,
        reviewer: None,
        created_at: now,
        archived_at: None,
        description: None,
    };
    projects::insert(conn, &p)?;
    Ok(p)
}

pub fn update_project(conn: &Connection, id: &str, patch: &ProjectPatch, now: i64) -> Result<Project, String> {
    let mut p = projects::get(conn, id)?;
    if let Some(n) = &patch.name {
        p.name = validate::name(n, "project")?;
    }
    if let Some(k) = &patch.key {
        p.key = validate::project_key(k)?;
    }
    if let Some(c) = &patch.color {
        p.color = validate::color(c)?;
    }
    if let Some(e) = &patch.default_executor {
        if let Some(e) = e {
            validate::executor(e)?;
        }
        p.default_executor = e.clone();
    }
    if let Some(r) = &patch.reviewer {
        p.reviewer = r.as_deref().map(validate::reviewer).transpose()?;
    }
    if let Some(a) = patch.archived {
        p.archived_at = if a { p.archived_at.or(Some(now)) } else { None };
    }
    projects::update(conn, &p)?;
    Ok(p)
}

/// Borra en cascada repos, tareas y fuentes. Rechaza si hay runs en curso. Devuelve los
/// ids de las tareas borradas (para limpiar sus planes en disco).
pub fn delete_project(conn: &mut Connection, id: &str) -> Result<Vec<String>, String> {
    projects::get(conn, id)?;
    if qruns::pending_in_project(conn, id)? > 0 {
        return Err("The project has queued or running runs: cancel them first.".into());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let task_ids: Vec<String> = tasks::list(&tx, Some(id))?.into_iter().map(|t| t.id).collect();
    // Desvincular las tareas de sus fuentes antes del cascade (la FK de `src_link_id` no
    // deja borrar un link con tareas apuntándole, y el orden del cascade no está garantizado).
    tx.execute(
        "UPDATE tasks SET src_link_id = NULL WHERE project_id = ?1 AND src_link_id IS NOT NULL",
        [id],
    )
    .map_err(|e| e.to_string())?;
    projects::delete(&tx, id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(task_ids)
}

// ---------- Repos ----------

/// `root`: raíz git canónica ya resuelta (con `util::paths::require_git_root`).
pub fn add_repo(conn: &Connection, project_id: &str, input: &NewRepo, root: &Path, now: i64) -> Result<Repo, String> {
    projects::get(conn, project_id)?;
    let path = root.to_string_lossy().into_owned();
    let dir_name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
    let name = match input.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => validate::name(n, "repo")?,
        None => dir_name,
    };
    let launch = options::normalize(&input.launch).map_err(|e| e.join("\n"))?;
    if let Some(e) = &input.default_executor {
        validate::executor(e)?;
    }
    let r = Repo {
        id: new_id('r', now),
        project_id: project_id.to_string(),
        path,
        name,
        launch,
        default_executor: input.default_executor.clone(),
        default_isolation: input.default_isolation.unwrap_or(Isolation::Worktree),
        default_finish: input.default_finish.unwrap_or(Finish::Pr),
        default_review: input.default_review.unwrap_or(true),
        reviewer: input.reviewer.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(validate::reviewer).transpose()?,
        position: repos::next_position(conn, project_id)?,
        created_at: now,
    };
    repos::insert(conn, &r)?;
    Ok(r)
}

pub fn update_repo(conn: &Connection, id: &str, patch: &RepoPatch) -> Result<Repo, String> {
    let mut r = repos::get(conn, id)?;
    if let Some(n) = &patch.name {
        r.name = validate::name(n, "repo")?;
    }
    if let Some(e) = &patch.default_executor {
        if let Some(e) = e {
            validate::executor(e)?;
        }
        r.default_executor = e.clone();
    }
    if let Some(i) = patch.default_isolation {
        r.default_isolation = i;
    }
    if let Some(f) = patch.default_finish {
        r.default_finish = f;
    }
    if let Some(v) = patch.default_review {
        r.default_review = v;
    }
    if let Some(rv) = &patch.reviewer {
        r.reviewer = rv.as_deref().map(validate::reviewer).transpose()?;
    }
    let mut launch = r.launch.clone();
    if let Some(m) = &patch.model {
        launch.model = m.clone();
    }
    if let Some(e) = &patch.effort {
        launch.effort = e.clone();
    }
    if let Some(p) = &patch.permission_mode {
        launch.permission_mode = p.clone();
    }
    r.launch = options::normalize(&launch).map_err(|e| e.join("\n"))?;
    if let Some(p) = patch.position {
        r.position = p;
    }
    repos::update(conn, &r)?;
    Ok(r)
}

pub fn delete_repo(conn: &Connection, id: &str) -> Result<(), String> {
    Ok(repos::delete(conn, id)?)
}

// ---------- Tareas ----------

/// Ruta del plan para leerlo o pasárselo al ejecutor. Un archivo se vuelve a validar
/// (puede haberse movido o reemplazado por un symlink desde que se creó la tarea).
pub fn plan_path(env: &Env, task: &Task, repo: &Repo) -> Result<PathBuf, String> {
    match &task.plan {
        PlanRef::Text => {
            let p = env.text_plan_path(&task.id);
            if p.is_file() {
                Ok(p)
            } else {
                Err("The saved plan text is missing.".into())
            }
        }
        PlanRef::File { path } => validate::plan_file(Path::new(&repo.path), path),
    }
}

pub fn read_plan(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("Couldn't read the plan: {e}"))?;
    let mut bytes = Vec::new();
    file.take(validate::MAX_PLAN_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("Couldn't read the plan: {e}"))?;
    if bytes.len() as u64 > validate::MAX_PLAN_BYTES {
        return Err(format!("The plan is too large (max {} KB).", validate::MAX_PLAN_BYTES / 1024));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Valida el plan. Un texto se escribe en `staging` (se renombra al definitivo recién
/// cuando la base guardó la tarea).
fn stage_plan(repo: &Repo, plan: &PlanInput, staging: &Path) -> Result<PlanRef, String> {
    match plan {
        PlanInput::Text { text } => {
            validate::plan_text(text)?;
            write_atomic(staging, text.as_bytes())?;
            Ok(PlanRef::Text)
        }
        PlanInput::File { path } => {
            let canon = validate::plan_file(Path::new(&repo.path), path)?;
            Ok(PlanRef::File { path: canon.to_string_lossy().into_owned() })
        }
    }
}

/// Filas del outbox por un cambio de estado hecho en Nodal (a mano, o por la cola), en la
/// transacción de quien llama.
pub fn push_status(
    conn: &Connection,
    task: &Task,
    status: Option<TaskStatus>,
    comment: Option<String>,
    manages_source: Option<&str>,
    now: i64,
) -> Result<(), String> {
    for op in outbox_ops(task, status, comment, manages_source) {
        let queued = match op {
            OutboxOp::Status(s) => providers::enqueue_status(conn, &task.id, s, now),
            OutboxOp::Comment(body) => providers::enqueue_comment(conn, &task.id, &body, now),
        };
        queued.map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn create_task(conn: &mut Connection, env: &Env, input: &NewTask, now: i64) -> Result<Task, String> {
    let title = validate::title(&input.title)?;
    let repo = repos::get(conn, &input.repo_id)?;
    if repo.project_id != input.project_id {
        return Err("The repo belongs to another project.".into());
    }
    if let Some(e) = &input.assignee {
        validate::executor(e)?;
    }
    let labels = validate::labels(input.labels.as_deref().unwrap_or_default())?;
    let acceptance = validate::acceptance(input.acceptance.as_deref().unwrap_or_default())?;
    let id = new_id('t', now);
    let plan_file = env.text_plan_path(&id);
    let plan = stage_plan(&repo, &input.plan, &plan_file)?;
    let result = (|| -> Result<Task, String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let number = projects::take_task_number(&tx, &input.project_id)?;
        let status = input.status.unwrap_or(TaskStatus::Todo);
        let t = Task {
            id: id.clone(),
            project_id: input.project_id.clone(),
            repo_id: repo.id.clone(),
            number,
            title,
            status,
            priority: input.priority.unwrap_or_default(),
            labels,
            position: tasks::next_position(&tx, &input.project_id, status)?,
            plan,
            plan_overridden: false,
            acceptance,
            assignee: input.assignee.clone(),
            isolation: input.isolation,
            finish: input.finish,
            review: input.review,
            worktree: None,
            source: None,
            created_at: now,
            updated_at: now,
            closed_at: matches!(status, TaskStatus::Done | TaskStatus::Canceled).then_some(now),
        };
        tasks::insert(&tx, &t)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(t)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(env.plan_dir(&id));
    }
    result
}

/// Aplica un patch. En las importadas solo se acepta plan (queda `planOverridden`), estado,
/// repo, criterios y opciones de ejecución.
pub fn update_task(conn: &mut Connection, env: &Env, id: &str, patch: &TaskPatch, now: i64) -> Result<Task, String> {
    let old = tasks::get(conn, id)?;
    let mut t = old.clone();
    if t.source.is_some() && (patch.title.is_some() || patch.priority.is_some() || patch.labels.is_some()) {
        return Err("Imported tasks are read-only: only the plan, status, acceptance criteria and run options can change.".into());
    }
    let mut repo = repos::get(conn, &t.repo_id)?;
    if let Some(new_repo) = patch.repo_id.as_deref().filter(|r| *r != t.repo_id) {
        let target = repos::get(conn, new_repo)?;
        if target.project_id != t.project_id {
            return Err("A task can only move to a repo of its own project.".into());
        }
        if !qruns::pending_for_task(conn, id)?.is_empty() {
            return Err("The task has a queued or running run: wait for it or cancel it first.".into());
        }
        if t.worktree.is_some() {
            return Err("Clean up the task's worktree before moving it to another repo.".into());
        }
        if let (PlanRef::File { path }, None) = (&t.plan, &patch.plan) {
            validate::plan_file(Path::new(&target.path), path)
                .map_err(|_| "The plan file is in the old repo: pick a new plan for this task.".to_string())?;
        }
        t.repo_id = target.id.clone();
        repo = target;
    }
    if let Some(title) = &patch.title {
        t.title = validate::title(title)?;
    }
    if let Some(p) = patch.priority {
        t.priority = p;
    }
    if let Some(l) = &patch.labels {
        t.labels = validate::labels(l)?;
    }
    if let Some(a) = &patch.acceptance {
        t.acceptance = validate::acceptance(a)?;
    }
    if let Some(a) = &patch.assignee {
        if let Some(e) = a {
            validate::executor(e)?;
        }
        t.assignee = a.clone();
    }
    if let Some(i) = patch.isolation {
        t.isolation = i;
    }
    if let Some(f) = patch.finish {
        t.finish = f;
    }
    if let Some(r) = patch.review {
        t.review = r;
    }
    let text_path = env.text_plan_path(id);
    let staged = text_path.with_extension("md.new");
    if let Some(plan) = &patch.plan {
        t.plan = stage_plan(&repo, plan, &staged)?;
        if t.source.is_some() {
            t.plan_overridden = true;
        }
    }
    let new_status = patch.status.filter(|s| *s != old.status);
    if let Some(s) = new_status {
        t.status = s;
        t.closed_at = matches!(s, TaskStatus::Done | TaskStatus::Canceled).then(|| old.closed_at.unwrap_or(now));
        t.position = tasks::next_position(conn, &t.project_id, s)?;
    }
    t.updated_at = now;
    let saved = (|| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tasks::update(&tx, &t)?;
        push_status(&tx, &t, new_status, None, None, now)?;
        tx.commit().map_err(|e| e.to_string())
    })();
    // Recién ahora se tocan los archivos del plan: si el guardado falló, quedan como estaban.
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    if staged.is_file() {
        std::fs::rename(&staged, &text_path).map_err(|e| format!("Couldn't save the plan: {e}"))?;
    } else if old.plan == PlanRef::Text && t.plan != PlanRef::Text {
        let _ = std::fs::remove_file(&text_path);
    }
    Ok(t)
}

/// Arrastre en el board: columna y/o posición.
pub fn move_task(conn: &mut Connection, id: &str, status: TaskStatus, position: f64, now: i64) -> Result<Task, String> {
    if !position.is_finite() {
        return Err("Invalid position.".into());
    }
    let mut t = tasks::get(conn, id)?;
    let changed = (t.status != status).then_some(status);
    if let Some(s) = changed {
        t.closed_at = matches!(s, TaskStatus::Done | TaskStatus::Canceled).then(|| t.closed_at.unwrap_or(now));
    }
    t.status = status;
    t.position = position;
    t.updated_at = now;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tasks::update(&tx, &t)?;
    push_status(&tx, &t, changed, None, None, now)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(t)
}

/// Cambia el estado de la tarea por una transición de la cola (respeta Done/Canceled) y
/// deja las filas del outbox en la misma transacción. Devuelve el estado nuevo si cambió.
pub fn apply_task_transition(
    conn: &Connection,
    task: &Task,
    next: Option<TaskStatus>,
    comment: Option<String>,
    manages_source: Option<&str>,
    now: i64,
) -> Result<Option<TaskStatus>, String> {
    let status = apply_status(task.status, next);
    if let Some(s) = status {
        tasks::set_status(conn, &task.id, s, now)?;
    }
    if status.is_some() || comment.is_some() {
        push_status(conn, task, status, comment, manages_source, now)?;
    }
    Ok(status)
}

/// Borra la tarea y su plan en texto. Rechaza si tiene runs en cola o en curso. El worktree
/// (si hay) queda: se limpia aparte.
pub fn delete_task(conn: &Connection, env: &Env, id: &str) -> Result<(), String> {
    tasks::get(conn, id)?;
    if !qruns::pending_for_task(conn, id)?.is_empty() {
        return Err("The task has a queued or running run: cancel it first.".into());
    }
    tasks::delete(conn, id)?;
    match std::fs::remove_dir_all(env.plan_dir(id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("work: couldn't remove the plan of {id}: {e}");
        }
        _ => {}
    }
    Ok(())
}

pub fn read_task_plan(conn: &Connection, env: &Env, id: &str) -> Result<String, String> {
    let t = tasks::get(conn, id)?;
    let repo = repos::get(conn, &t.repo_id)?;
    read_plan(&plan_path(env, &t, &repo)?)
}

// ---------- Relaciones ----------

pub fn add_relation(conn: &Connection, task_id: &str, other_id: &str, kind: RelationKind) -> Result<(), String> {
    Ok(relations::add(conn, &TaskRelation { task_id: task_id.into(), other_id: other_id.into(), kind })?)
}

pub fn remove_relation(conn: &Connection, task_id: &str, other_id: &str, kind: RelationKind) -> Result<(), String> {
    Ok(relations::remove(conn, &TaskRelation { task_id: task_id.into(), other_id: other_id.into(), kind })?)
}

// ---------- Settings ----------

pub fn set_settings(conn: &mut Connection, s: &Settings) -> Result<Settings, String> {
    let clean = Settings {
        concurrency: validate::concurrency(s.concurrency)?,
        editor: s.editor.as_deref().map(str::trim).filter(|e| !e.is_empty()).map(validate::editor).transpose()?,
        reviewer: validate::reviewer(&s.reviewer)?,
        default_executor: match &s.default_executor {
            Some(e) => {
                validate::executor(e)?;
                Some(e.clone())
            }
            None => None,
        },
    };
    rows::save_settings(conn, &clean)?;
    Ok(rows::load_settings(conn)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::util::paths::tests::TempDir;
    use serde_json::json;

    struct Fx {
        _t: TempDir,
        env: Env,
        repo_dir: PathBuf,
    }

    fn fx(name: &str) -> Fx {
        let t = TempDir::new(name);
        let repo_dir = t.0.join("web");
        std::fs::create_dir_all(repo_dir.join("docs")).unwrap();
        std::fs::write(repo_dir.join("docs/plan.md"), "# Plan del repo").unwrap();
        let env = Env { data_dir: t.0.join("data"), worktrees_root: t.0.join("wt"), claude_dir: None };
        Fx { _t: t, env, repo_dir }
    }

    fn new_task(project: &Project, repo: &Repo, title: &str) -> NewTask {
        serde_json::from_value(json!({
            "projectId": project.id, "repoId": repo.id, "title": title,
            "plan": {"kind": "text", "text": "# Plan\nhacer algo"},
            "acceptance": ["El logo aparece", "  "], "labels": ["ui"], "priority": "high"
        }))
        .unwrap()
    }

    #[test]
    fn projects_crud_and_unique_keys() {
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let p = create_project(&c, &NewProject { name: "Payments".into(), key: "pay".into(), color: None }, 10).unwrap();
        assert_eq!(p.key, "PAY");
        assert_eq!(p.color, validate::PALETTE[0]);
        let err = create_project(&c, &NewProject { name: "Otro".into(), key: "PAY".into(), color: None }, 11).unwrap_err();
        assert!(err.contains("already used"), "{err}");
        let q = create_project(&c, &NewProject { name: "Web".into(), key: "WEB".into(), color: Some("#123456".into()) }, 12).unwrap();
        let err = update_project(&c, &q.id, &ProjectPatch { key: Some("pay".into()), ..Default::default() }, 13).unwrap_err();
        assert!(err.contains("already used"));
        let patch: ProjectPatch = serde_json::from_value(json!({"name": "Web 2", "reviewer": "code-reviewer", "archived": true})).unwrap();
        let q = update_project(&c, &q.id, &patch, 14).unwrap();
        assert_eq!((q.name.as_str(), q.reviewer.as_deref(), q.archived_at), ("Web 2", Some("code-reviewer"), Some(14)));
        let patch: ProjectPatch = serde_json::from_value(json!({"reviewer": null, "archived": false})).unwrap();
        let q = update_project(&c, &q.id, &patch, 15).unwrap();
        assert_eq!((q.reviewer, q.archived_at), (None, None));
        assert_eq!(projects::list(&c, false).unwrap().len(), 2);
        assert!(delete_project(&mut c, &q.id).unwrap().is_empty());
        assert!(update_project(&c, &q.id, &ProjectPatch::default(), 16).unwrap_err().contains("no longer exists"));
    }

    #[test]
    fn repos_unique_path_options_and_delete_guard() {
        let f = fx("ops-repos");
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let p = create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None }, 1).unwrap();
        let q = create_project(&c, &NewProject { name: "Web".into(), key: "WEB".into(), color: None }, 1).unwrap();
        let input: NewRepo = serde_json::from_value(json!({"path": "x", "model": "opus", "effort": "turbo"})).unwrap();
        assert!(add_repo(&c, &p.id, &input, &f.repo_dir, 2).unwrap_err().contains("Invalid effort"));
        let input: NewRepo = serde_json::from_value(json!({"path": "x", "model": " opus ", "defaultIsolation": "in_place"})).unwrap();
        let r = add_repo(&c, &p.id, &input, &f.repo_dir, 2).unwrap();
        assert_eq!((r.name.as_str(), r.launch.model.as_deref(), r.default_isolation), ("web", Some("opus"), Isolation::InPlace));
        assert!(r.default_review);
        let err = add_repo(&c, &q.id, &input, &f.repo_dir, 3).unwrap_err();
        assert!(err.contains("already added to Pay"), "{err}");
        let patch: RepoPatch = serde_json::from_value(json!({"model": null, "reviewer": "sec-reviewer", "defaultFinish": "commit"})).unwrap();
        let r = update_repo(&c, &r.id, &patch).unwrap();
        assert_eq!((r.launch.model.as_deref(), r.reviewer.as_deref(), r.default_finish), (None, Some("sec-reviewer"), Finish::Commit));
        let t = create_task(&mut c, &f.env, &new_task(&p, &r, "Una"), 4).unwrap();
        assert!(delete_repo(&c, &r.id).unwrap_err().contains("has 1 task"));
        delete_task(&c, &f.env, &t.id).unwrap();
        delete_repo(&c, &r.id).unwrap();
    }

    #[test]
    fn tasks_numbering_plan_move_and_patch() {
        let f = fx("ops-tasks");
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let p = create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None }, 1).unwrap();
        let r = add_repo(&c, &p.id, &NewRepo::default(), &f.repo_dir, 2).unwrap();
        let other_dir = f.repo_dir.parent().unwrap().join("api");
        std::fs::create_dir_all(&other_dir).unwrap();
        let r2 = add_repo(&c, &p.id, &NewRepo::default(), &other_dir, 2).unwrap();
        let t1 = create_task(&mut c, &f.env, &new_task(&p, &r, "Primera"), 3).unwrap();
        let t2 = create_task(&mut c, &f.env, &new_task(&p, &r, "Segunda"), 4).unwrap();
        assert_eq!((t1.number, t2.number), (1, 2));
        assert_eq!(t1.acceptance, ["El logo aparece"]);
        assert_eq!((t1.status, t1.priority), (TaskStatus::Todo, Priority::High));
        assert!(t2.position > t1.position);
        assert_eq!(read_task_plan(&c, &f.env, &t1.id).unwrap(), "# Plan\nhacer algo");
        assert_eq!(projects::get(&c, &p.id).unwrap().next_task_number, 3);

        // Plan como archivo del repo (relativo), y de vuelta a texto.
        let patch: TaskPatch = serde_json::from_value(json!({"plan": {"kind": "file", "path": "docs/plan.md"}})).unwrap();
        let t = update_task(&mut c, &f.env, &t1.id, &patch, 5).unwrap();
        assert!(matches!(&t.plan, PlanRef::File { path } if path.ends_with("docs/plan.md")));
        assert!(!f.env.text_plan_path(&t1.id).exists());
        assert_eq!(read_task_plan(&c, &f.env, &t1.id).unwrap(), "# Plan del repo");
        let bad: TaskPatch = serde_json::from_value(json!({"plan": {"kind": "file", "path": "../api/x.md"}})).unwrap();
        assert!(update_task(&mut c, &f.env, &t1.id, &bad, 6).is_err());

        // Mover de repo: con plan del repo viejo, hay que dar otro plan.
        let mv: TaskPatch = serde_json::from_value(json!({"repoId": r2.id})).unwrap();
        assert!(update_task(&mut c, &f.env, &t1.id, &mv, 7).unwrap_err().contains("old repo"));
        let mv: TaskPatch = serde_json::from_value(json!({"repoId": r2.id, "plan": {"kind": "text", "text": "nuevo"}})).unwrap();
        let t = update_task(&mut c, &f.env, &t1.id, &mv, 8).unwrap();
        assert_eq!((t.repo_id.as_str(), &t.plan), (r2.id.as_str(), &PlanRef::Text));
        assert_eq!(read_task_plan(&c, &f.env, &t1.id).unwrap(), "nuevo");

        // Opciones con null = volver al default del repo.
        let opts: TaskPatch = serde_json::from_value(json!({"isolation": "in_place", "review": false, "assignee": {"kind": "workflow", "name": "plan-task"}})).unwrap();
        let t = update_task(&mut c, &f.env, &t1.id, &opts, 9).unwrap();
        assert_eq!((t.isolation, t.review), (Some(Isolation::InPlace), Some(false)));
        let clear: TaskPatch = serde_json::from_value(json!({"isolation": null, "assignee": null})).unwrap();
        let t = update_task(&mut c, &f.env, &t1.id, &clear, 10).unwrap();
        assert_eq!((t.isolation, t.assignee, t.review), (None, None, Some(false)));

        // Estado manual y cierre.
        let t = move_task(&mut c, &t1.id, TaskStatus::Done, 0.5, 11).unwrap();
        assert_eq!((t.status, t.position, t.closed_at), (TaskStatus::Done, 0.5, Some(11)));
        let t = move_task(&mut c, &t1.id, TaskStatus::Todo, 2.0, 12).unwrap();
        assert_eq!(t.closed_at, None);
        assert!(move_task(&mut c, &t1.id, TaskStatus::Todo, f64::NAN, 13).is_err());

        // Relaciones.
        add_relation(&c, &t1.id, &t2.id, RelationKind::Related).unwrap();
        add_relation(&c, &t2.id, &t1.id, RelationKind::Related).unwrap();
        add_relation(&c, &t1.id, &t2.id, RelationKind::Blocks).unwrap();
        assert_eq!(relations::list(&c, &t2.id).unwrap().len(), 2);
        assert!(add_relation(&c, &t1.id, &t1.id, RelationKind::Blocks).is_err());
        assert!(add_relation(&c, &t1.id, "t-no", RelationKind::Blocks).unwrap_err().contains("no longer exists"));
        remove_relation(&c, &t2.id, &t1.id, RelationKind::Related).unwrap();
        assert_eq!(relations::list(&c, &t1.id).unwrap().len(), 1);

        // Borrar el proyecto borra en cascada.
        let gone = delete_project(&mut c, &p.id).unwrap();
        assert_eq!(gone.len(), 2);
        assert!(tasks::list(&c, None).unwrap().is_empty());
    }

    #[test]
    fn imported_tasks_are_read_only_and_manual_moves_push_state() {
        let f = fx("ops-imported");
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let p = create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None }, 1).unwrap();
        let r = add_repo(&c, &p.id, &NewRepo::default(), &f.repo_dir, 2).unwrap();
        let t = create_task(&mut c, &f.env, &new_task(&p, &r, "Importada"), 3).unwrap();
        let mut map = StateMap { confirmed_at: Some(1), ..Default::default() };
        map.push.insert(TaskStatus::InProgress, Some("s-prog".into()));
        let link = SourceLink {
            id: "l1".into(),
            project_id: p.id.clone(),
            provider: "linear".into(),
            scope: ScopeRef { kind: "team".into(), id: "tm".into(), name: "Eng".into() },
            default_repo_id: None,
            repo_rules: vec![],
            state_map: map,
            auto_import: false,
            created_at: 1,
            last_synced_at: None,
            last_sync_error: None,
            pending_state_changes: None,
        };
        rows::insert_source_link(&c, &link).unwrap();
        c.execute(
            "UPDATE tasks SET src_provider='linear', src_link_id='l1', src_external_id='e1', src_identifier='ENG-1', src_url='https://linear.app/acme/issue/ENG-1' WHERE id=?1",
            [&t.id],
        )
        .unwrap();
        let err = update_task(&mut c, &f.env, &t.id, &TaskPatch { title: Some("x".into()), ..Default::default() }, 4).unwrap_err();
        assert!(err.contains("read-only"));
        let plan: TaskPatch = serde_json::from_value(json!({"plan": {"kind": "text", "text": "mío"}})).unwrap();
        assert!(update_task(&mut c, &f.env, &t.id, &plan, 5).unwrap().plan_overridden);
        move_task(&mut c, &t.id, TaskStatus::InProgress, 1.0, 6).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM sync_outbox WHERE task_id = ?1 AND kind = 'set_state'", [&t.id], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        // Mover dentro de la misma columna no empuja nada.
        move_task(&mut c, &t.id, TaskStatus::InProgress, 3.0, 7).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        // Se puede borrar el proyecto aunque tenga tareas vinculadas.
        delete_project(&mut c, &p.id).unwrap();
    }

    #[test]
    fn settings_validation() {
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let s = set_settings(&mut c, &Settings { concurrency: 2, editor: Some("cursor".into()), reviewer: "code-reviewer".into(), default_executor: None }).unwrap();
        assert_eq!((s.concurrency, s.editor.as_deref()), (2, Some("cursor")));
        assert!(set_settings(&mut c, &Settings { concurrency: 0, ..s.clone() }).is_err());
        assert!(set_settings(&mut c, &Settings { editor: Some("vim; rm".into()), ..s.clone() }).is_err());
        let s = set_settings(&mut c, &Settings { editor: Some(" ".into()), ..s }).unwrap();
        assert_eq!(s.editor, None);
    }
}
