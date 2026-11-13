use super::*;
use serde_json::json;

#[test]
fn patches_distinguish_missing_from_null() {
    let p: TaskPatch = serde_json::from_value(json!({"title": "x"})).unwrap();
    assert_eq!(p.title.as_deref(), Some("x"));
    assert!(p.plan.is_none());
    let p: RepoPatch = serde_json::from_value(json!({"model": null, "effort": "high"})).unwrap();
    assert_eq!(p.model, Some(None));
    assert_eq!(p.effort, Some(Some("high".into())));
    assert_eq!(p.permission_mode, None);
    assert!(serde_json::from_value::<TaskPatch>(json!({"projectId": "p"})).is_err());
}

#[test]
fn new_repo_flattens_options_and_plan_input_shape() {
    let r: NewRepo = serde_json::from_value(
        json!({"path": "/x", "model": "opus"}),
    )
    .unwrap();
    assert_eq!(r.launch.model.as_deref(), Some("opus"));
    let t: PlanInput = serde_json::from_value(json!({"kind": "text", "text": "# Plan"})).unwrap();
    assert_eq!(
        t,
        PlanInput::Text {
            text: "# Plan".into()
        }
    );
    assert!(serde_json::from_value::<PlanInput>(json!({"kind": "url", "path": "x"})).is_err());
}

#[test]
fn merge_input_squashes_and_never_pushes_by_default() {
    let m: MergeInput = serde_json::from_value(json!({})).unwrap();
    assert!(m.squash && !m.push && !m.cleanup);
}
