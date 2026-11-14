//! The PR of a (repo, branch): found with `gh` or linked by hand.

use rusqlite::OptionalExtension;

use crate::rows::SqlEnum;
use crate::Conn as Connection;
use crate::DbError;
use nodal_domain::model::BranchPr;

/// The stored PR and when it was last looked up.
pub fn get(conn: &Connection, repo_id: &str, branch: &str) -> Result<Option<(BranchPr, i64)>, DbError> {
    Ok(conn
        .query_row(
            "SELECT url, number, state, head, manual, checked_at FROM branch_prs WHERE repo_id = ?1 AND branch = ?2",
            [repo_id, branch],
            |r| {
                let pr = BranchPr {
                    url: r.get(0)?,
                    number: r.get(1)?,
                    state: r.get::<_, SqlEnum<_>>(2)?.0,
                    head: r.get(3)?,
                    manual: r.get(4)?,
                    branch: branch.to_string(),
                };
                Ok((pr, r.get(5)?))
            },
        )
        .optional()?)
}

pub fn upsert(conn: &Connection, repo_id: &str, pr: &BranchPr, checked_at: i64) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO branch_prs (repo_id, branch, url, number, state, head, manual, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (repo_id, branch) DO UPDATE SET url = ?3, number = ?4, state = ?5, head = ?6, manual = ?7, checked_at = ?8",
        rusqlite::params![repo_id, pr.branch, pr.url, pr.number, SqlEnum(pr.state), pr.head, pr.manual, checked_at],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, repo_id: &str, branch: &str) -> Result<(), DbError> {
    conn.execute("DELETE FROM branch_prs WHERE repo_id = ?1 AND branch = ?2", [repo_id, branch])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rows, Db};
    use nodal_domain::model::PrState;
    use nodal_domain::testutil::{project_of, repo_of};

    #[test]
    fn upserts_gets_and_deletes() {
        let db = Db::open_in_memory().unwrap();
        let c = db.guard();
        rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
        rows::insert_repo(&c, &repo_of("r1", "p1", "/r1")).unwrap();
        assert_eq!(get(&c, "r1", "b").unwrap(), None);
        let mut pr = BranchPr { url: "u".into(), number: 3, state: PrState::Open, head: None, manual: false, branch: "b".into() };
        upsert(&c, "r1", &pr, 10).unwrap();
        assert_eq!(get(&c, "r1", "b").unwrap(), Some((pr.clone(), 10)));
        pr = BranchPr { state: PrState::Merged, head: Some("abc".into()), manual: true, ..pr };
        upsert(&c, "r1", &pr, 20).unwrap();
        assert_eq!(get(&c, "r1", "b").unwrap(), Some((pr, 20)));
        delete(&c, "r1", "b").unwrap();
        assert_eq!(get(&c, "r1", "b").unwrap(), None);
    }
}
