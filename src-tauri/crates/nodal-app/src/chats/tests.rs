use std::path::{Path, PathBuf};

use nodal_domain::testutil::{project_of, repo_of};
use nodal_store::rows::{insert_project, insert_repo};
use nodal_store::Db;

use super::*;

fn seed(c: &Connection) {
    insert_project(c, &project_of("p1", "PAY")).unwrap();
    insert_project(c, &project_of("p2", "WEB")).unwrap();
    insert_repo(c, &repo_of("r1", "p1", "/Users/me/Code/acme-api")).unwrap();
    let mut r2 = repo_of("r2", "p1", "/Users/me/Code/acme-web");
    r2.position = 1;
    insert_repo(c, &r2).unwrap();
    insert_repo(c, &repo_of("r3", "p2", "/Users/me/Code/acme-docs")).unwrap();
}

fn patch(json: &str) -> ChatPatch {
    serde_json::from_str(json).unwrap()
}

#[test]
fn create_validates_project_repo_and_options() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let chat = ops::create(&c, "p1", &NewChat::default(), 7).unwrap();
    assert!(chat.id.starts_with('c'));
    assert_eq!(
        (
            chat.repo_id.as_deref(),
            chat.title.as_deref(),
            chat.session_id.as_deref()
        ),
        (None, None, None)
    );
    assert_eq!((chat.created_at, chat.updated_at), (7, 7));

    let input: NewChat =
        serde_json::from_str(r#"{"repoId":"r2","title":"  Plan\n the  login ","model":" sonnet ","permissionMode":"plan"}"#)
            .unwrap();
    let chat = ops::create(&c, "p1", &input, 8).unwrap();
    assert_eq!(
        (chat.repo_id.as_deref(), chat.title.as_deref()),
        (Some("r2"), Some("Plan the login"))
    );
    assert_eq!(
        (
            chat.launch.model.as_deref(),
            chat.launch.permission_mode.as_deref()
        ),
        (Some("sonnet"), Some("plan"))
    );

    let other = NewChat {
        repo_id: Some("r3".into()),
        ..NewChat::default()
    };
    assert_eq!(
        ops::create(&c, "p1", &other, 9).unwrap_err(),
        "That repo belongs to another project."
    );
    assert!(ops::create(&c, "missing", &NewChat::default(), 9).is_err());
    let bad = NewChat {
        launch: LaunchOptions {
            effort: Some("ultra".into()),
            ..Default::default()
        },
        ..NewChat::default()
    };
    assert!(ops::create(&c, "p1", &bad, 9)
        .unwrap_err()
        .contains("Invalid effort"));
}

#[test]
fn update_patches_only_what_it_gets() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let input = NewChat {
        repo_id: Some("r1".into()),
        launch: LaunchOptions {
            model: Some("opus".into()),
            ..Default::default()
        },
        ..NewChat::default()
    };
    let chat = ops::create(&c, "p1", &input, 1).unwrap();

    let u = ops::update(
        &c,
        &chat.id,
        &patch(r#"{"title":"Billing","effort":"high"}"#),
    )
    .unwrap();
    assert_eq!(
        (u.title.as_deref(), u.repo_id.as_deref()),
        (Some("Billing"), Some("r1"))
    );
    assert_eq!(
        (u.launch.model.as_deref(), u.launch.effort.as_deref()),
        (Some("opus"), Some("high"))
    );

    let u = ops::update(&c, &chat.id, &patch(r#"{"repoId":null,"model":null}"#)).unwrap();
    assert_eq!(
        (u.repo_id.as_deref(), u.launch.model.as_deref()),
        (None, None)
    );
    assert_eq!(u.launch.effort.as_deref(), Some("high"));

    assert!(ops::update(&c, &chat.id, &patch(r#"{"repoId":"r3"}"#)).is_err());
    assert!(ops::update(&c, &chat.id, &patch(r#"{"permissionMode":"yolo"}"#)).is_err());
    assert!(
        serde_json::from_str::<ChatPatch>(r#"{"sessionId":"x"}"#).is_err(),
        "the session isn't patchable"
    );
    assert_eq!(chats::get(&c, &chat.id).unwrap(), u);
}

#[test]
fn prepare_send_names_and_touches_the_chat() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let chat = ops::create(&c, "p1", &NewChat::default(), 1).unwrap();
    assert_eq!(
        ops::prepare_send(&c, &chat.id, "  \n ", 2).unwrap_err(),
        "The message is empty."
    );
    assert!(
        ops::prepare_send(&c, &chat.id, &"x".repeat(MESSAGE_MAX + 1), 2)
            .unwrap_err()
            .contains("too long")
    );

    let long = format!("Turn the checkout flow\ninto tasks {}", "a".repeat(200));
    let (sent, project, repos) = ops::prepare_send(&c, &chat.id, &long, 5).unwrap();
    assert_eq!(sent.updated_at, 5);
    let t = sent.title.clone().unwrap();
    assert!(
        t.starts_with("Turn the checkout flow into tasks") && t.chars().count() == TITLE_MAX,
        "{t}"
    );
    assert_eq!(project.id, "p1");
    assert_eq!(
        repos.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["r1", "r2"]
    );

    let (again, _, _) = ops::prepare_send(&c, &chat.id, "Second message", 6).unwrap();
    assert_eq!(again.title, sent.title, "the title is kept");
    assert_eq!(chats::get(&c, &chat.id).unwrap().updated_at, 6);
}

#[test]
fn spec_runs_in_the_chat_repo_or_the_first_one() {
    let project = project_of("p1", "PAY");
    let mut second = repo_of("r2", "p1", "/Users/me/Code/acme-web");
    second.launch.permission_mode = Some("bypassPermissions".into());
    let repos = vec![repo_of("r1", "p1", "/Users/me/Code/acme-api"), second];
    let mut chat = Chat {
        id: "c1".into(),
        project_id: "p1".into(),
        repo_id: None,
        title: None,
        session_title: None,
        session_id: None,
        launch: LaunchOptions {
            model: Some("haiku".into()),
            effort: Some("low".into()),
            permission_mode: None,
        },
        created_at: 1,
        updated_at: 1,
    };
    let s = spec(&chat, &project, &repos, None).unwrap();
    assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme-api"));
    assert_eq!(
        s.args[..6],
        [
            "--model",
            "haiku",
            "--effort",
            "low",
            "--add-dir",
            "/Users/me/Code/acme-web"
        ]
    );
    assert_eq!(s.args[6], "--append-system-prompt");

    chat.repo_id = Some("r2".into());
    let s = spec(
        &chat,
        &project,
        &repos,
        Some(Path::new("/Users/me/nodal-mcp")),
    )
    .unwrap();
    assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme-web"));
    assert!(s.args.iter().any(|a| a == "--mcp-config") && !s.args.iter().any(|a| a == "--add-dir"));
    assert!(
        !s.args.iter().any(|a| a == "--permission-mode"),
        "the repo's permission mode isn't inherited"
    );

    chat.repo_id = Some("gone".into());
    assert!(spec(&chat, &project, &repos, None).is_err());
    chat.repo_id = None;
    assert!(spec(&chat, &project, &[], None)
        .unwrap_err()
        .contains("Add a repo"));
}

#[test]
fn a_repo_less_chat_runs_at_the_project_root_when_it_has_one() {
    let mut project = project_of("p1", "PAY");
    project.root_path = Some("/Users/me/Code/acme".into());
    let repos = vec![
        repo_of("r1", "p1", "/Users/me/Code/acme/api"),
        repo_of("r2", "p1", "/Users/me/Code/acme/web"),
        repo_of("r3", "p1", "/Users/me/Code/acme-docs"),
    ];
    let mut chat = Chat {
        id: "c1".into(),
        project_id: "p1".into(),
        repo_id: None,
        title: None,
        session_title: None,
        session_id: None,
        launch: LaunchOptions::default(),
        created_at: 1,
        updated_at: 1,
    };
    let s = spec(&chat, &project, &repos, None).unwrap();
    assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme"));
    assert_eq!(
        s.args[..2],
        ["--add-dir", "/Users/me/Code/acme-docs"],
        "only the repo outside the root"
    );
    assert!(s
        .args
        .last()
        .unwrap()
        .contains("This chat runs at the project root."));
    assert_eq!(
        s.cwd,
        PathBuf::from(chat_cwd(&chat, &project, &repos).unwrap().path()),
        "the transcript looks it up here"
    );

    let s = spec(&chat, &project, &[], None).unwrap();
    assert_eq!(
        s.cwd,
        PathBuf::from("/Users/me/Code/acme"),
        "a root is enough to chat"
    );

    chat.repo_id = Some("r2".into());
    let s = spec(&chat, &project, &repos, None).unwrap();
    assert_eq!(
        s.cwd,
        PathBuf::from("/Users/me/Code/acme/web"),
        "a repo chat ignores the root"
    );
    assert!(!s.args.iter().any(|a| a == "--add-dir"));

    chat.repo_id = None;
    project.root_path = None;
    assert_eq!(
        chat_cwd(&chat, &project, &repos).unwrap(),
        ChatCwd::Repo(&repos[0])
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // builds its input from a real session file, like the transcript command does
fn first_message_goes_back_into_the_items() {
    use nodal_domain::model::claude::TranscriptItem;

    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Split the checkout into tasks"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Here is a plan."}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":"Thanks"}}"#,
    ];
    let path = std::env::temp_dir().join(format!(
        "nodal-chat-transcript-{}.jsonl",
        std::process::id()
    ));
    std::fs::write(&path, lines.join("\n")).unwrap();
    let t = nodal_host::claude::fs::transcript::read_session_transcript(&path, "c1", None, None, 10)
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let t = transcript::with_first_message(t);
    let texts: Vec<_> = t
        .items
        .iter()
        .map(|i| match i {
            TranscriptItem::User { text, .. } => format!("user: {text}"),
            TranscriptItem::Text { text, .. } => format!("claude: {text}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        texts,
        [
            "user: Split the checkout into tasks",
            "claude: Here is a plan.",
            "user: Thanks"
        ]
    );
    assert_eq!((t.prompt.as_deref(), t.total_items), (None, 3));

    let cut = Transcript {
        omitted: 2,
        prompt: Some("first".into()),
        ..t.clone()
    };
    assert_eq!(
        transcript::with_first_message(cut).prompt.as_deref(),
        Some("first"),
        "not next to a gap"
    );
}
