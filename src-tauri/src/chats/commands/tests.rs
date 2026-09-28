use super::*;
use crate::runs::types::TranscriptItem;

#[test]
fn first_message_goes_back_into_the_items() {
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Split the checkout into tasks"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Here is a plan."}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":"Thanks"}}"#,
    ];
    let path = std::env::temp_dir().join(format!("nodal-chat-transcript-{}.jsonl", std::process::id()));
    std::fs::write(&path, lines.join("\n")).unwrap();
    let t = claude_fs::read_session_transcript(&path, "c1", None, None, 10).unwrap().unwrap();
    let _ = std::fs::remove_file(&path);
    let t = with_first_message(t);
    let texts: Vec<_> = t
        .items
        .iter()
        .map(|i| match i {
            TranscriptItem::User { text, .. } => format!("user: {text}"),
            TranscriptItem::Text { text, .. } => format!("claude: {text}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(texts, ["user: Split the checkout into tasks", "claude: Here is a plan.", "user: Thanks"]);
    assert_eq!((t.prompt.as_deref(), t.total_items), (None, 3));

    let cut = Transcript { omitted: 2, prompt: Some("first".into()), ..t.clone() };
    assert_eq!(with_first_message(cut).prompt.as_deref(), Some("first"), "not next to a gap");
}
