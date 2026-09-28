use super::*;

#[test]
fn simple_fields() {
    assert_eq!(title("  Fix   the\n bug ").unwrap(), "Fix the bug");
    assert!(title(" \n ").is_err());
    assert!(title(&"x".repeat(MAX_TITLE_CHARS + 1)).is_err());
    assert!(plan_text("  ").is_err());
    assert!(plan_text("# ok").is_ok());
    assert_eq!(project_key(" pay ").unwrap(), "PAY");
    assert_eq!(project_key("web2").unwrap(), "WEB2");
    for bad in ["P", "PAYMENTS", "2PAY", "PA-Y", ""] {
        assert!(project_key(bad).is_err(), "{bad}");
    }
    assert!(color("#d98c3f").is_ok() && color("#abc").is_ok() && color(PALETTE[0]).is_ok());
    assert!(color("red").is_err() && color("oklch(1;x)").is_err() && color("#12345").is_err());
    assert_eq!(
        labels(&[" ui ".into(), "UI".into(), "".into(), "api".into()]).unwrap(),
        ["ui", "api"]
    );
    assert!(labels(&["x".repeat(41)]).is_err());
    assert_eq!(acceptance(&["a\nb".into(), "  ".into()]).unwrap(), ["a b"]);
    assert!(executor(&Executor::Agent {
        name: "a b".into(),
        source: crate::model::AgentSource::User
    })
    .is_err());
    assert!(executor(&Executor::Workflow { name: "-x".into() }).is_err());
    assert!(executor(&Executor::Claude).is_ok());
    assert_eq!(editor(" cursor ").unwrap(), "cursor");
    assert!(editor("rm").is_err());
    assert!(concurrency(0).is_err() && concurrency(17).is_err() && concurrency(3).is_ok());
    assert_eq!(extra_instructions(Some("  ")).unwrap(), None);
    assert_eq!(reviewer(" code-reviewer ").unwrap(), "code-reviewer");
}
