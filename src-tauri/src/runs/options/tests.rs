use super::*;

fn opts(model: Option<&str>, effort: Option<&str>, perm: Option<&str>) -> LaunchOptions {
    LaunchOptions {
        model: model.map(String::from),
        effort: effort.map(String::from),
        permission_mode: perm.map(String::from),
    }
}

#[test]
fn models() {
    for ok in [
        "opus",
        "opus[1m]",
        "sonnet",
        "fable",
        "haiku",
        "claude-sonnet-5",
        "claude-opus-4-1",
        "claude-sonnet-4.5",
        "claude-opus-5[1m]",
    ] {
        assert!(is_valid_model(ok), "{ok}");
    }
    for bad in [
        "",
        "gpt-4",
        "claude-",
        "--model",
        "claude-x; rm -rf ~",
        "Claude-Sonnet",
        "claude-a b",
    ] {
        assert!(!is_valid_model(bad), "{bad}");
    }
}

#[test]
fn args_are_separate_and_only_when_set() {
    assert!(to_args(&LaunchOptions::default()).unwrap().is_empty());
    assert_eq!(
        to_args(&opts(Some(" sonnet "), Some("xhigh"), Some("acceptEdits"))).unwrap(),
        [
            "--model",
            "sonnet",
            "--effort",
            "xhigh",
            "--permission-mode",
            "acceptEdits"
        ]
    );
    assert_eq!(
        to_args(&opts(Some(""), None, Some("plan"))).unwrap(),
        ["--permission-mode", "plan"]
    );
}

#[test]
fn rejects_unknown_values() {
    let err = normalize(&opts(Some("gpt"), Some("ultra"), Some("yolo"))).unwrap_err();
    assert_eq!(err.len(), 3);
    assert!(to_args(&opts(None, Some("--help"), None)).is_err());
}
