use super::*;
use crate::domain::{ExtKind, ExternalState, Priority, ScopeRef};

fn state(id: &str, name: &str, kind: ExtKind) -> ExternalState {
    ExternalState {
        id: id.into(),
        name: name.into(),
        kind,
        color: None,
    }
}

pub fn item(n: u32, desc: Option<&str>) -> ExternalItem {
    ExternalItem {
        external_id: format!("uuid-{n}"),
        identifier: format!("ENG-{n}"),
        url: format!("https://linear.app/acme/issue/ENG-{n}/x"),
        title: format!("Issue {n}"),
        description_md: desc.map(Into::into),
        state: state("s-todo", "Todo", ExtKind::Unstarted),
        scopes: vec![ScopeRef {
            kind: "team".into(),
            id: "team-eng".into(),
            name: "Engineering".into(),
        }],
        parent: None,
        children: Vec::new(),
        labels: Vec::new(),
        assignee: None,
        priority: Priority::None,
        updated_at: "2026-09-20T10:00:00.000Z".into(),
        created_at: Some("2026-09-01T10:00:00.000Z".into()),
        closed_at: None,
    }
}

#[test]
fn write_plan_only_when_changed() {
    let dir = std::env::temp_dir().join(format!("nodal-plan-{}", crate::util::new_id('x', 1)));
    let path = plan_path(&dir, "t1");
    assert!(write_plan(&path, "a").unwrap());
    assert!(!write_plan(&path, "a").unwrap());
    assert!(write_plan(&path, "b").unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
    std::fs::remove_dir_all(dir).unwrap();
}
