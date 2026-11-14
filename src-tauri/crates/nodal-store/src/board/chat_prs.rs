//! The PRs a chat opened or linked.

use crate::Conn as Connection;
use crate::DbError;
use nodal_domain::model::ChatPr;

/// Records `pr` for the chat unless that URL is already there. `true` if it was new.
pub fn insert_if_new(conn: &Connection, chat_id: &str, pr: &ChatPr) -> Result<bool, DbError> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO chat_prs (chat_id, repo_id, number, url, branch, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![chat_id, pr.repo_id, pr.number, pr.url, pr.branch, pr.created_at],
    )?;
    Ok(n > 0)
}

/// The chat's PRs, newest first.
pub fn list(conn: &Connection, chat_id: &str) -> Result<Vec<ChatPr>, DbError> {
    let mut st = conn.prepare(
        "SELECT repo_id, url, number, branch, created_at FROM chat_prs WHERE chat_id = ?1 ORDER BY created_at DESC, rowid DESC",
    )?;
    let rows = st.query_map([chat_id], |r| {
        Ok(ChatPr { repo_id: r.get(0)?, url: r.get(1)?, number: r.get(2)?, branch: r.get(3)?, created_at: r.get(4)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rows, Db};
    use nodal_domain::model::{Chat, LaunchOptions};
    use nodal_domain::testutil::{project_of, repo_of};

    #[test]
    fn inserts_once_per_url_and_lists_newest_first() {
        let db = Db::open_in_memory().unwrap();
        let c = db.guard();
        rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
        rows::insert_repo(&c, &repo_of("r1", "p1", "/r1")).unwrap();
        let chat = Chat {
            id: "c1".into(),
            project_id: "p1".into(),
            repo_id: Some("r1".into()),
            title: None,
            session_title: None,
            agent: None,
            task_id: None,
            worktree: None,
            archived_at: None,
            session_id: None,
            launch: LaunchOptions::default(),
            created_at: 1,
            updated_at: 1,
            pinned_at: None,
        };
        rows::insert_chat(&c, &chat).unwrap();
        let pr = |n: i64, at| ChatPr { repo_id: "r1".into(), url: format!("u{n}"), number: n, branch: Some(format!("b{n}")), created_at: at };
        assert!(insert_if_new(&c, "c1", &pr(1, 10)).unwrap());
        assert!(!insert_if_new(&c, "c1", &pr(1, 99)).unwrap());
        assert!(insert_if_new(&c, "c1", &pr(2, 20)).unwrap());
        assert_eq!(list(&c, "c1").unwrap(), [pr(2, 20), pr(1, 10)]);
        assert!(list(&c, "other").unwrap().is_empty());
    }
}
