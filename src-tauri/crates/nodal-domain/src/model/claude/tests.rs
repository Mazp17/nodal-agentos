use super::*;

#[test]
fn append_system_prompt_is_one_arg_after_the_variadic_lists() {
    let extra = ExtraFlags {
        allowed_tools: vec!["Bash(npm test:*)".into()],
        append_system_prompt: Some("Don't ask: stop.".into()),
        ..Default::default()
    };
    assert_eq!(
        extra.to_args(),
        [
            "--allowedTools",
            "Bash(npm test:*)",
            "--append-system-prompt",
            "Don't ask: stop."
        ]
    );
    assert!(ExtraFlags::default().to_args().is_empty());
}
