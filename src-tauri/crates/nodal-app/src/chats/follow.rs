//! A free chat follows a worktree its agent made or worked in: after a turn, if the turn's tool
//! calls point at a linked worktree of one of the chat's repos that no task or other chat owns,
//! that worktree becomes the chat's own, so the next turn runs there (`--resume` keeps the
//! conversation across worktrees of the same repo).

use std::path::Path;

use serde_json::Value;

use nodal_domain::ports::Git;
use nodal_store::board::{repos, tasks};
use nodal_store::{chats, Db};

/// Path-like inputs of the file tools.
const FILE_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Whether `word` appears in `text` as a whole token (not inside a longer name).
fn has_word(text: &str, word: &str) -> bool {
    let is_name = |c: char| c.is_alphanumeric() || "._-".contains(c);
    text.match_indices(word).any(|(i, _)| {
        !text[..i].chars().next_back().is_some_and(is_name)
            && !text[i + word.len()..].chars().next().is_some_and(is_name)
    })
}

/// Whether one tool call made or worked in the worktree at `path`: a file tool writing under
/// it, a shell command that `cd`s/points into it or runs `git worktree add` naming it, or
/// `EnterWorktree` naming it. Reading or listing (`git worktree list`) doesn't count.
fn touches(name: &str, input: &Value, path: &str) -> bool {
    let Some(base) = Path::new(path).file_name().and_then(|b| b.to_str()) else {
        return false;
    };
    match name {
        n if FILE_TOOLS.contains(&n) => ["file_path", "notebook_path"]
            .iter()
            .filter_map(|k| input.get(k).and_then(Value::as_str))
            // shortcut: compares paths as written, misses a symlinked spelling of the worktree.
            .any(|p| Path::new(p).starts_with(path)),
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            has_word(cmd, path) || (cmd.contains("worktree add") && has_word(cmd, base))
        }
        "EnterWorktree" => has_word(&input.to_string(), base),
        _ => false,
    }
}

/// `(repo_id, path)` of the candidate the turn touched last, else the only one `added` during
/// the turn (a path built at runtime, `git worktree add ../$NAME`, names nothing). Several
/// added and none touched: no guess.
pub(crate) fn pick<'a>(
    calls: &[(String, Value)],
    candidates: &'a [(String, String)],
    added: impl Fn(&str) -> bool,
) -> Option<&'a (String, String)> {
    let touched = calls.iter().rev().find_map(|(name, input)| candidates.iter().find(|(_, p)| touches(name, input, p)));
    touched.or_else(|| match candidates.iter().filter(|(_, p)| added(p)).collect::<Vec<_>>()[..] {
        [one] => Some(one),
        _ => None,
    })
}

/// Moves the free chat `chat_id` into the worktree its last turn worked in. `true` if it moved.
pub(crate) fn follow_worktree(
    db: &Db,
    git: &dyn Git,
    calls: &[(String, Value)],
    chat_id: &str,
) -> Result<bool, String> {
    if calls.is_empty() {
        return Ok(false);
    }
    let (chat, repos, taken) = {
        let conn = db.guard();
        let chat = chats::get(&conn, chat_id).map_err(|e| e.to_string())?;
        if chat.task_id.is_some() {
            return Ok(false);
        }
        let repos: Vec<_> = repos::list(&conn, Some(&chat.project_id))
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|r| chat.repo_id.as_ref().is_none_or(|id| *id == r.id))
            .collect();
        let mut taken: Vec<String> = tasks::list(&conn, Some(&chat.project_id))
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|t| t.worktree.map(|w| w.path))
            .collect();
        taken.extend(
            chats::list(&conn, &chat.project_id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|c| c.id != chat.id)
                .filter_map(|c| c.worktree),
        );
        (chat, repos, taken)
    };
    let candidates: Vec<(String, String)> = repos
        .iter()
        .flat_map(|r| {
            let wts = git.list_worktrees(Path::new(&r.path)).unwrap_or_default();
            wts.into_iter()
                .filter(|w| !w.is_main && !taken.contains(&w.path))
                .map(|w| (r.id.clone(), w.path))
                .collect::<Vec<_>>()
        })
        .collect();
    // `updated_at` is when the turn's message was sent (`ops::prepare_send`).
    let added = |p: &str| git.worktree_added_ms(Path::new(p)).is_some_and(|t| t >= chat.updated_at);
    let Some((repo_id, path)) = pick(calls, &candidates, added) else {
        return Ok(false);
    };
    if chat.worktree.as_ref() == Some(path) {
        return Ok(false);
    }
    chats::set_worktree(&db.guard(), chat_id, repo_id, path).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests;
