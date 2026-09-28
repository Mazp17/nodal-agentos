//! Moved to `SRC/commands/{board,execution,sessions}.rs`; re-exported so current uses don't
//! break.

/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::board::{
    add_repo, add_task_relation, create_project, create_task, delete_project, delete_repo, delete_task, get_settings,
    get_task, list_executors, list_hidden_executors, list_projects, list_repos, list_task_relations, list_tasks,
    move_task, read_task_plan, remove_task_relation, reorder_tasks, set_executor_hidden, set_settings, update_project,
    update_repo, update_task,
};
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use crate::commands::execution::{
    cancel_run, cleanup_worktree, confirm_run, get_run, hand_off, latest_runs_by_task, launch_task, list_queue,
    list_runs_light, list_task_runs, merge_worktree, open_in_editor, open_worktree, reorder_queue, review_now, run_diff,
    work_summary, worktree_status,
};
/// Only the bridge is left; nothing in this crate calls it directly anymore.
#[allow(unused_imports)]
pub use crate::commands::sessions::get_run_transcript;
