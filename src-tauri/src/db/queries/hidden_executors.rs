//! Agents and workflows each project hides from its pickers. Launching never reads this:
//! a task already assigned to a hidden executor still runs.

use rusqlite::{named_params, Connection};

use super::is_constraint;
use crate::db::DbError;
use crate::domain::{AgentSource, HiddenExecutor, HiddenKind};

const NAME_MAX: usize = 200;

fn invalid(msg: &str) -> DbError {
    DbError::Invalid(msg.into())
}

fn check(e: &HiddenExecutor) -> Result<(), DbError> {
    let name = e.name.trim();
    if name.is_empty() || name != e.name || e.name.chars().count() > NAME_MAX {
        return Err(invalid("Invalid executor name."));
    }
    match (e.source, &e.repo_id) {
        (AgentSource::Repo, None) => Err(invalid("A repo-level executor needs its repo.")),
        (AgentSource::User | AgentSource::Plugin, Some(_)) => Err(invalid("Only repo-level executors belong to a repo.")),
        _ => Ok(()),
    }
}

pub fn list(conn: &Connection, project_id: &str) -> Result<Vec<HiddenExecutor>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT kind, source, name, repo_id FROM project_hidden_executors WHERE project_id = ?1
         ORDER BY kind, source, name, repo_id",
    )?;
    let rows = stmt.query_map([project_id], |r| {
        let kind: String = r.get(0)?;
        let source: String = r.get(1)?;
        Ok((kind, source, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, source, name, repo_id) = row?;
        // The CHECKs keep these valid; an unknown value is skipped rather than failing the list.
        if let (Some(kind), Some(source)) = (HiddenKind::parse(&kind), AgentSource::parse(&source)) {
            out.push(HiddenExecutor { kind, source, name, repo_id });
        }
    }
    Ok(out)
}

/// Hides (`hidden`) or shows `e` in the project. Idempotent both ways.
pub fn set(conn: &Connection, project_id: &str, e: &HiddenExecutor, hidden: bool) -> Result<(), DbError> {
    check(e)?;
    let params = named_params! {
        ":project": project_id, ":kind": e.kind.as_str(), ":source": e.source.as_str(),
        ":name": e.name, ":repo": e.repo_id,
    };
    if hidden {
        conn.execute(
            "INSERT INTO project_hidden_executors (project_id, kind, source, name, repo_id)
             VALUES (:project, :kind, :source, :name, :repo) ON CONFLICT DO NOTHING",
            params,
        )
        .map_err(|err| {
            if is_constraint(&err) {
                invalid("That project or repo no longer exists, or the repo belongs to another project.")
            } else {
                err.into()
            }
        })?;
    } else {
        conn.execute(
            "DELETE FROM project_hidden_executors
             WHERE project_id = :project AND kind = :kind AND source = :source AND name = :name
               AND ifnull(repo_id, '') = ifnull(:repo, '')",
            params,
        )?;
    }
    Ok(())
}
