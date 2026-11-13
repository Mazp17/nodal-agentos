//! Task status transitions and the provider writes they imply (pure).

use crate::model::{Task, TaskStatus};

/// Final task status: Done/Canceled (set by hand) aren't overwritten.
pub fn apply_status(current: TaskStatus, next: Option<TaskStatus>) -> Option<TaskStatus> {
    match (current, next) {
        (TaskStatus::Done | TaskStatus::Canceled, _) => None,
        (c, Some(n)) if c != n => Some(n),
        _ => None,
    }
}

/// Statuses Nodal pushes to the provider. The
/// provider resolves it with its `state_map` (without a mapping, it's dropped).
pub const PUSHED_STATUSES: [TaskStatus; 4] = [
    TaskStatus::InProgress,
    TaskStatus::InReview,
    TaskStatus::Blocked,
    TaskStatus::Todo,
];

/// What to write to the provider for a change made in Nodal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxOp {
    Status(TaskStatus),
    Comment(String),
}

/// Provider writes for a task change. Nothing if the task is local or if the executor syncs
/// the provider on its own (`managesSource` of the same provider). The mapping (pending,
/// "Don't sync", vanished states) is resolved by the provider when draining.
pub fn outbox_ops(
    task: &Task,
    new_status: Option<TaskStatus>,
    comment: Option<String>,
    manages_source: Option<&str>,
) -> Vec<OutboxOp> {
    let Some(src) = &task.source else {
        return Vec::new();
    };
    if manages_source == Some(src.provider.as_str()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(s) = new_status.filter(|s| PUSHED_STATUSES.contains(s)) {
        out.push(OutboxOp::Status(s));
    }
    if let Some(body) = comment {
        out.push(OutboxOp::Comment(body));
    }
    out
}

#[cfg(test)]
mod tests;
