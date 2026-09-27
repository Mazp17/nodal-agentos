//! Per-project chats: `claude -p` sessions scoped to a project (or narrowed to one repo)
//! and driven from the UI, with permission prompts answered in the app.
//!
//! - `ops`: synchronous CRUD and validation over the database;
//! - `context`: the Nodal context a chat starts with (MCP, other repos, system prompt);
//! - `process`: one `claude` process per active chat and its `nodal://chat` events;
//! - `commands`: the Tauri commands.

pub mod commands;
pub mod context;
pub mod process;

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Deserialize;

use crate::db::queries::{chats, double_option, projects, repos};
use crate::domain::{Chat, LaunchOptions, Project, Repo};
use crate::runs::options;
use crate::util::{clip_chars, new_id};

pub use process::{Chats, Spec};

/// Managed state: the processes of every chat.
#[derive(Clone)]
pub struct ChatState(pub Chats);

const TITLE_MAX: usize = 80;
/// A message goes to stdin as one JSON line: a generous cap keeps a paste from being huge.
pub const MESSAGE_MAX: usize = 100_000;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChat {
    #[serde(default)]
    pub repo_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(flatten)]
    pub launch: LaunchOptions,
}

/// Missing field = leave as is, `null` = clear.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatPatch {
    #[serde(default, deserialize_with = "double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub repo_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub effort: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub permission_mode: Option<Option<String>>,
}

fn title(t: Option<&str>) -> Option<String> {
    let one_line = t?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!one_line.is_empty()).then(|| clip_chars(&one_line, TITLE_MAX))
}

fn check_repo(conn: &Connection, project_id: &str, repo_id: Option<&str>) -> Result<(), String> {
    if let Some(id) = repo_id {
        if repos::get(conn, id)?.project_id != project_id {
            return Err("That repo belongs to another project.".into());
        }
    }
    Ok(())
}

pub mod ops {
    use super::*;

    pub fn create(conn: &Connection, project_id: &str, input: &NewChat, now: i64) -> Result<Chat, String> {
        projects::get(conn, project_id)?;
        let repo_id = input.repo_id.clone().filter(|r| !r.is_empty());
        check_repo(conn, project_id, repo_id.as_deref())?;
        let chat = Chat {
            id: new_id('c', now),
            project_id: project_id.to_string(),
            repo_id,
            title: title(input.title.as_deref()),
            session_title: None,
            session_id: None,
            launch: options::normalize(&input.launch).map_err(|e| e.join("\n"))?,
            created_at: now,
            updated_at: now,
        };
        chats::insert(conn, &chat)?;
        Ok(chat)
    }

    pub fn update(conn: &Connection, id: &str, patch: &ChatPatch) -> Result<Chat, String> {
        let mut c = chats::get(conn, id)?;
        if let Some(t) = &patch.title {
            c.title = title(t.as_deref());
        }
        if let Some(r) = &patch.repo_id {
            let r = r.clone().filter(|r| !r.is_empty());
            check_repo(conn, &c.project_id, r.as_deref())?;
            c.repo_id = r;
        }
        let mut launch = c.launch.clone();
        for (field, value) in [
            (&mut launch.model, &patch.model),
            (&mut launch.effort, &patch.effort),
            (&mut launch.permission_mode, &patch.permission_mode),
        ] {
            if let Some(v) = value {
                *field = v.clone();
            }
        }
        c.launch = options::normalize(&launch).map_err(|e| e.join("\n"))?;
        chats::update(conn, &c)?;
        Ok(c)
    }

    /// Checks the message, marks the chat as used now (naming it after its first message)
    /// and returns what launching it needs.
    pub fn prepare_send(conn: &Connection, id: &str, text: &str, now: i64) -> Result<(Chat, Project, Vec<Repo>), String> {
        if text.trim().is_empty() {
            return Err("The message is empty.".into());
        }
        if text.chars().count() > MESSAGE_MAX {
            return Err(format!("The message is too long (over {MESSAGE_MAX} characters)."));
        }
        let mut c = chats::get(conn, id)?;
        if c.title.is_none() {
            c.title = title(Some(text));
        }
        c.updated_at = now;
        chats::update(conn, &c)?;
        let project = projects::get(conn, &c.project_id)?;
        let repos = repos::list(conn, Some(&c.project_id))?;
        Ok((c, project, repos))
    }
}

/// Where a chat runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChatCwd<'a> {
    /// The chat's repo or, in a project without a root folder, its first repo.
    Repo(&'a Repo),
    /// The project's root folder, for a repo-less chat: it sees the repos and everything
    /// else under it. Never a run target.
    Root(&'a str),
}

impl<'a> ChatCwd<'a> {
    pub fn path(&self) -> &'a str {
        match self {
            ChatCwd::Repo(r) => &r.path,
            ChatCwd::Root(p) => p,
        }
    }

    /// Whether `repo` is already visible from here without `--add-dir`. Both paths are
    /// canonical, and `Path::starts_with` compares whole components (`acme` ≠ `acme-web`).
    pub fn covers(&self, repo: &Repo) -> bool {
        match self {
            ChatCwd::Repo(r) => r.id == repo.id,
            ChatCwd::Root(p) => Path::new(&repo.path).starts_with(p),
        }
    }
}

/// Where the chat runs: its repo, else the project's root folder, else its first repo.
pub fn chat_cwd<'a>(chat: &Chat, project: &'a Project, repos: &'a [Repo]) -> Result<ChatCwd<'a>, String> {
    let found = match (&chat.repo_id, project.root_path.as_deref().filter(|p| !p.is_empty())) {
        (Some(id), _) => repos.iter().find(|r| &r.id == id).map(ChatCwd::Repo),
        (None, Some(root)) => Some(ChatCwd::Root(root)),
        (None, None) => repos.first().map(ChatCwd::Repo),
    };
    found.ok_or_else(|| "Add a repo to the project before chatting: the chat runs inside one.".to_string())
}

/// How the chat's process is launched: in its repo or the project's root folder, with its
/// own model, effort and permission mode, and the Nodal context (`mcp` is `nodal-mcp`, when
/// found).
pub fn spec(chat: &Chat, project: &Project, repos: &[Repo], mcp: Option<&Path>) -> Result<Spec, String> {
    let cwd = chat_cwd(chat, project, repos)?;
    let mut args = options::to_args(&chat.launch)?;
    args.extend(context::args(chat, project, repos, cwd, mcp));
    Ok(Spec { cwd: PathBuf::from(cwd.path()), args })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::rows::{insert_project, insert_repo};
    use crate::work::testutil::{project_of, repo_of};

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
        let db = open_in_memory().unwrap();
        let c = db.lock().unwrap();
        seed(&c);
        let chat = ops::create(&c, "p1", &NewChat::default(), 7).unwrap();
        assert!(chat.id.starts_with('c'));
        assert_eq!((chat.repo_id.as_deref(), chat.title.as_deref(), chat.session_id.as_deref()), (None, None, None));
        assert_eq!((chat.created_at, chat.updated_at), (7, 7));

        let input: NewChat =
            serde_json::from_str(r#"{"repoId":"r2","title":"  Plan\n the  login ","model":" sonnet ","permissionMode":"plan"}"#)
                .unwrap();
        let chat = ops::create(&c, "p1", &input, 8).unwrap();
        assert_eq!((chat.repo_id.as_deref(), chat.title.as_deref()), (Some("r2"), Some("Plan the login")));
        assert_eq!((chat.launch.model.as_deref(), chat.launch.permission_mode.as_deref()), (Some("sonnet"), Some("plan")));

        let other = NewChat { repo_id: Some("r3".into()), ..NewChat::default() };
        assert_eq!(ops::create(&c, "p1", &other, 9).unwrap_err(), "That repo belongs to another project.");
        assert!(ops::create(&c, "missing", &NewChat::default(), 9).is_err());
        let bad = NewChat { launch: LaunchOptions { effort: Some("ultra".into()), ..Default::default() }, ..NewChat::default() };
        assert!(ops::create(&c, "p1", &bad, 9).unwrap_err().contains("Invalid effort"));
    }

    #[test]
    fn update_patches_only_what_it_gets() {
        let db = open_in_memory().unwrap();
        let c = db.lock().unwrap();
        seed(&c);
        let input = NewChat { repo_id: Some("r1".into()), launch: LaunchOptions { model: Some("opus".into()), ..Default::default() }, ..NewChat::default() };
        let chat = ops::create(&c, "p1", &input, 1).unwrap();

        let u = ops::update(&c, &chat.id, &patch(r#"{"title":"Billing","effort":"high"}"#)).unwrap();
        assert_eq!((u.title.as_deref(), u.repo_id.as_deref()), (Some("Billing"), Some("r1")));
        assert_eq!((u.launch.model.as_deref(), u.launch.effort.as_deref()), (Some("opus"), Some("high")));

        let u = ops::update(&c, &chat.id, &patch(r#"{"repoId":null,"model":null}"#)).unwrap();
        assert_eq!((u.repo_id.as_deref(), u.launch.model.as_deref()), (None, None));
        assert_eq!(u.launch.effort.as_deref(), Some("high"));

        assert!(ops::update(&c, &chat.id, &patch(r#"{"repoId":"r3"}"#)).is_err());
        assert!(ops::update(&c, &chat.id, &patch(r#"{"permissionMode":"yolo"}"#)).is_err());
        assert!(serde_json::from_str::<ChatPatch>(r#"{"sessionId":"x"}"#).is_err(), "the session isn't patchable");
        assert_eq!(chats::get(&c, &chat.id).unwrap(), u);
    }

    #[test]
    fn prepare_send_names_and_touches_the_chat() {
        let db = open_in_memory().unwrap();
        let c = db.lock().unwrap();
        seed(&c);
        let chat = ops::create(&c, "p1", &NewChat::default(), 1).unwrap();
        assert_eq!(ops::prepare_send(&c, &chat.id, "  \n ", 2).unwrap_err(), "The message is empty.");
        assert!(ops::prepare_send(&c, &chat.id, &"x".repeat(MESSAGE_MAX + 1), 2).unwrap_err().contains("too long"));

        let long = format!("Turn the checkout flow\ninto tasks {}", "a".repeat(200));
        let (sent, project, repos) = ops::prepare_send(&c, &chat.id, &long, 5).unwrap();
        assert_eq!(sent.updated_at, 5);
        let t = sent.title.clone().unwrap();
        assert!(t.starts_with("Turn the checkout flow into tasks") && t.chars().count() == TITLE_MAX, "{t}");
        assert_eq!(project.id, "p1");
        assert_eq!(repos.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);

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
            launch: LaunchOptions { model: Some("haiku".into()), effort: Some("low".into()), permission_mode: None },
            created_at: 1,
            updated_at: 1,
        };
        let s = spec(&chat, &project, &repos, None).unwrap();
        assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme-api"));
        assert_eq!(s.args[..6], ["--model", "haiku", "--effort", "low", "--add-dir", "/Users/me/Code/acme-web"]);
        assert_eq!(s.args[6], "--append-system-prompt");

        chat.repo_id = Some("r2".into());
        let s = spec(&chat, &project, &repos, Some(Path::new("/Users/me/nodal-mcp"))).unwrap();
        assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme-web"));
        assert!(s.args.iter().any(|a| a == "--mcp-config") && !s.args.iter().any(|a| a == "--add-dir"));
        assert!(!s.args.iter().any(|a| a == "--permission-mode"), "the repo's permission mode isn't inherited");

        chat.repo_id = Some("gone".into());
        assert!(spec(&chat, &project, &repos, None).is_err());
        chat.repo_id = None;
        assert!(spec(&chat, &project, &[], None).unwrap_err().contains("Add a repo"));
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
        assert_eq!(s.args[..2], ["--add-dir", "/Users/me/Code/acme-docs"], "only the repo outside the root");
        assert!(s.args.last().unwrap().contains("This chat runs at the project root."));
        assert_eq!(s.cwd, PathBuf::from(chat_cwd(&chat, &project, &repos).unwrap().path()), "the transcript looks it up here");

        let s = spec(&chat, &project, &[], None).unwrap();
        assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme"), "a root is enough to chat");

        chat.repo_id = Some("r2".into());
        let s = spec(&chat, &project, &repos, None).unwrap();
        assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme/web"), "a repo chat ignores the root");
        assert!(!s.args.iter().any(|a| a == "--add-dir"));

        chat.repo_id = None;
        project.root_path = None;
        assert_eq!(chat_cwd(&chat, &project, &repos).unwrap(), ChatCwd::Repo(&repos[0]));
    }
}
