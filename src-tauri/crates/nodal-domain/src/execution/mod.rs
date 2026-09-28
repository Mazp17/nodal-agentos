//! Pure run/queue logic: state transitions, the queue's decisions, executor-result parsing,
//! launch-option validation, prompt building and worktree naming.

pub mod options;
pub mod prompts;
pub mod queue;
pub mod report;
pub mod transitions;
pub mod worktree;
