use super::*;

fn c(kind: Kind, p: Option<&str>) -> Changed {
    Changed { kind, project_id: p.map(str::to_string) }
}

#[test]
fn pending_schedules_once_and_dedupes() {
    let mut p = Pending::default();
    assert!(p.push(c(Kind::Tasks, Some("p1"))));
    assert!(!p.push(c(Kind::Tasks, Some("p1"))));
    assert!(!p.push(c(Kind::Runs, None)));
    assert_eq!(p.drain(), vec![c(Kind::Tasks, Some("p1")), c(Kind::Runs, None)]);
    assert!(p.push(c(Kind::Queue, None)), "after draining it schedules again");
}

#[test]
fn global_notice_covers_project_ones() {
    let mut p = Pending::default();
    p.push(c(Kind::Tasks, Some("p1")));
    p.push(c(Kind::Tasks, None));
    p.push(c(Kind::Sources, Some("p2")));
    assert_eq!(p.drain(), vec![c(Kind::Tasks, None), c(Kind::Sources, Some("p2"))]);
}

#[test]
fn payload_shape() {
    let v = serde_json::to_value(c(Kind::Sources, Some("p1"))).unwrap();
    assert_eq!(v, serde_json::json!({"kind": "sources", "projectId": "p1"}));
    let v = serde_json::to_value(c(Kind::Queue, None)).unwrap();
    assert_eq!(v, serde_json::json!({"kind": "queue"}));
    let v = serde_json::to_value(c(Kind::Chats, Some("p1"))).unwrap();
    assert_eq!(v, serde_json::json!({"kind": "chats", "projectId": "p1"}));
}

#[test]
fn events_without_app_are_noop() {
    Events::default().notify(Kind::Tasks, None);
}
