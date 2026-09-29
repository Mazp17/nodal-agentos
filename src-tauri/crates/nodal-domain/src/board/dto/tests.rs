use super::*;
use serde_json::json;

#[test]
fn patches_distinguish_missing_from_null() {
    let p: TaskPatch = serde_json::from_value(json!({"assignee": null, "title": "x"})).unwrap();
    assert_eq!(p.assignee, Some(None));
    assert_eq!(p.isolation, None);
    assert_eq!(p.title.as_deref(), Some("x"));
    let p: TaskPatch =
        serde_json::from_value(json!({"assignee": {"kind": "claude"}, "review": false})).unwrap();
    assert_eq!(p.assignee, Some(Some(Executor::Claude)));
    assert_eq!(p.review, Some(Some(false)));
    let p: RepoPatch = serde_json::from_value(json!({"model": null, "effort": "high"})).unwrap();
    assert_eq!(p.model, Some(None));
    assert_eq!(p.effort, Some(Some("high".into())));
    assert_eq!(p.permission_mode, None);
    assert!(serde_json::from_value::<TaskPatch>(json!({"projectId": "p"})).is_err());
}

#[test]
fn new_repo_flattens_options_and_plan_input_shape() {
    let r: NewRepo = serde_json::from_value(
        json!({"path": "/x", "model": "opus", "defaultIsolation": "in_place"}),
    )
    .unwrap();
    assert_eq!(r.launch.model.as_deref(), Some("opus"));
    assert_eq!(r.default_isolation, Some(Isolation::InPlace));
    let t: PlanInput = serde_json::from_value(json!({"kind": "text", "text": "# Plan"})).unwrap();
    assert_eq!(
        t,
        PlanInput::Text {
            text: "# Plan".into()
        }
    );
    assert!(serde_json::from_value::<PlanInput>(json!({"kind": "url", "path": "x"})).is_err());
    let l: LaunchInput =
        serde_json::from_value(json!({"extraInstructions": "x", "options": {"effort": "high"}}))
            .unwrap();
    assert_eq!(l.options.unwrap().effort.as_deref(), Some("high"));
}

#[test]
fn merge_input_squashes_and_never_pushes_by_default() {
    let m: MergeInput = serde_json::from_value(json!({})).unwrap();
    assert!(m.squash && !m.push && !m.cleanup);
}
