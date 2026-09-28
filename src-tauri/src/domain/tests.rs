use super::*;
use serde_json::json;

#[test]
fn enums_serialize_snake_case_and_match_as_str() {
    for s in TaskStatus::ALL {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        assert_eq!(TaskStatus::parse(s.as_str()), Some(*s));
    }
    for s in Isolation::ALL {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
    }
    for s in RunStatus::ALL {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
    }
    for s in RunOutcome::ALL {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
    }
    for s in Priority::ALL {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
    }
    assert_eq!(TaskStatus::parse("nope"), None);
}

#[test]
fn executor_shape() {
    let a = Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User };
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        json!({"kind": "agent", "name": "frontend-developer", "source": "user"})
    );
    assert_eq!(serde_json::to_value(Executor::Claude).unwrap(), json!({"kind": "claude"}));
    assert_eq!(
        serde_json::to_value(Executor::Workflow { name: "plan-task".into() }).unwrap(),
        json!({"kind": "workflow", "name": "plan-task"})
    );
}

#[test]
fn outbox_payload_shape() {
    let p = OutboxPayload::SetState { state_id: "s3".into() };
    assert_eq!(serde_json::to_value(&p).unwrap(), json!({"kind": "set_state", "stateId": "s3"}));
    assert_eq!(p.kind(), "set_state");
    let c = OutboxPayload::Comment { body: "hello".into() };
    assert_eq!(serde_json::to_value(&c).unwrap(), json!({"kind": "comment", "body": "hello"}));
    assert_eq!(c.kind(), "comment");
}

#[test]
fn repo_flattens_launch_options() {
    let r = Repo {
        id: "r".into(),
            project_id: "p".into(),
        path: "/x".into(),
        name: "x".into(),
        launch: LaunchOptions { model: Some("opus".into()), ..Default::default() },
        default_executor: None,
        default_isolation: Isolation::InPlace,
        default_finish: Finish::Pr,
        default_review: true,
        reviewer: None,
        position: 0,
        created_at: 1,
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["model"], "opus");
    assert_eq!(v["defaultIsolation"], "in_place");
    assert!(v.get("effort").is_none());
    assert_eq!(serde_json::from_value::<Repo>(v).unwrap(), r);
}

#[test]
fn state_map_keys_are_status_strings() {
    let mut m = StateMap::default();
    m.pull.insert("s1".into(), TaskStatus::InReview);
    m.push.insert(TaskStatus::Blocked, None);
    m.push.insert(TaskStatus::InProgress, Some("s2".into()));
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v["pull"]["s1"], "in_review");
    assert_eq!(v["push"]["blocked"], serde_json::Value::Null);
    assert_eq!(v["push"]["in_progress"], "s2");
    assert_eq!(serde_json::from_value::<StateMap>(v).unwrap(), m);
}

#[test]
fn settings_defaults_fill_missing_fields() {
    let s: Settings = serde_json::from_value(json!({"editor": "code"})).unwrap();
    assert_eq!(s.concurrency, DEFAULT_CONCURRENCY);
    assert_eq!(s.reviewer, DEFAULT_REVIEWER);
    assert_eq!(s.editor.as_deref(), Some("code"));
    assert_eq!(task_key("PAY", 1), "PAY-1");
}
