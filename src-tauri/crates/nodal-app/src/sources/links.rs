//! Source link types and the routing-rule bookkeeping shared by the hub's facade methods.
//! Moved from the shell's `commands/sources.rs`.

use serde::{Deserialize, Deserializer};

use nodal_domain::model::{RepoRule, RuleKind, ScopeRef, SourceLink};
use nodal_domain::util::new_id;
use nodal_store::{Conn as Connection, StoreError};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSourceLink {
    pub project_id: String,
    pub provider: String,
    pub scope: ScopeRef,
    #[serde(default)]
    pub default_repo_id: Option<String>,
    #[serde(default)]
    pub repo_rules: Vec<RepoRule>,
    #[serde(default)]
    pub auto_import: bool,
}

/// Distinguishes "missing" (`None`) from `null` (`Some(None)`).
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLinkPatch {
    #[serde(default, deserialize_with = "nullable")]
    pub default_repo_id: Option<Option<String>>,
    pub repo_rules: Option<Vec<RepoRule>>,
    pub auto_import: Option<bool>,
}

pub fn check_link_repos(conn: &Connection, link: &SourceLink) -> Result<(), StoreError> {
    let repos = link.default_repo_id.iter().chain(link.repo_rules.iter().map(|r| &r.repo_id));
    for repo in repos {
        nodal_store::sources::check_repo_in_project(conn, repo, &link.project_id)?;
    }
    Ok(())
}

/// Rules sent by the UI → stored rules. A new rule (no id, or a different kind or value
/// than the stored one with that id) gets an id and `created_at = now`: its auto-import
/// counts from then on. Unchanged ones keep their id and `created_at` (the client is not
/// trusted). Rejects empty values and two rules for the same project.
pub fn prepare_rules(old: &[RepoRule], incoming: Vec<RepoRule>, now: i64) -> Result<Vec<RepoRule>, String> {
    let mut out: Vec<RepoRule> = Vec::with_capacity(incoming.len());
    for mut r in incoming {
        r.value = r.value.trim().to_string();
        r.name = r.name.trim().to_string();
        if r.value.is_empty() || r.kind == RuleKind::Unknown {
            return Err(match r.kind {
                RuleKind::Label => "Routing rules need a label.".into(),
                RuleKind::Project => "Project rules need a project.".into(),
                RuleKind::Unknown => "Unknown routing rule type.".into(),
            });
        }
        if r.name.is_empty() {
            r.name = r.value.clone();
        }
        let same = old.iter().find(|o| !r.id.is_empty() && o.id == r.id && o.kind == r.kind && o.value == r.value);
        match same {
            Some(o) => r.created_at = o.created_at,
            None => {
                r.id = new_id('r', now);
                r.created_at = now;
            }
        }
        if out.iter().any(|o| o.id == r.id) {
            r.id = new_id('r', now);
        }
        if r.is_project() && out.iter().any(|o| o.is_project() && o.value == r.value) {
            return Err(format!("The project \"{}\" already has a rule.", r.name));
        }
        out.push(r);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
