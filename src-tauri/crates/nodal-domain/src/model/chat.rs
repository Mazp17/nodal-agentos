//! DTOs for a chat's `claude -p` stream-json process: what it is launched with, what it
//! reports, and the envelope streamed to the UI. The process itself lives in nodal-host.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::model::claude::{ToolResultInfo, TranscriptItem};

/// Event with every chat's output: `{chatId, event}` (mirror of `ChatEventEnvelope` in `api.ts`).
pub const CHAT_EVENT: &str = "nodal://chat";

/// How a chat's process is launched. A running process whose spec no longer matches is
/// restarted on the next message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSpec {
    pub cwd: PathBuf,
    /// Flags on top of `stream_json::CHAT_ARGS`, each one its own argument.
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// No process: the next message starts one.
    Stopped,
    /// Waiting for a message.
    Idle,
    /// A turn is running.
    Busy,
}

/// Events that come from Nodal rather than from `claude`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HostEvent {
    #[serde(rename_all = "camelCase")]
    State { state: RunState },
    /// A permission request was answered (here or in another window) or dropped by an interrupt.
    #[serde(rename_all = "camelCase")]
    PermissionResolved { request_id: String, allowed: bool },
    /// The process failed or exited in the middle of a turn.
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

/// A permission prompt the host must answer (`control_request` / `can_use_tool`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    /// The `tool_use` block it belongs to (`ToolUse.id` in the transcript).
    pub tool_use_id: Option<String>,
    /// What the tool is about to do, as Claude Code words it (a path, a command…).
    pub description: Option<String>,
    /// One line: the main argument, like transcript tool calls.
    pub summary: Option<String>,
    /// The tool's input as sent by Claude Code; allowing sends it back unchanged.
    pub input: Value,
}

/// What a chat process reports, ready for the UI. `TextDelta`/`ThinkingDelta` preview text
/// that a later `Item` carries in full.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamEvent {
    #[serde(rename_all = "camelCase")]
    Init {
        session_id: String,
        model: Option<String>,
        cwd: Option<String>,
        permission_mode: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    TextDelta {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta {
        text: String,
    },
    /// A complete text, thinking, tool call or (replayed) user message.
    #[serde(rename_all = "camelCase")]
    Item {
        item: TranscriptItem,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        tool_use_id: String,
        result: ToolResultInfo,
    },
    PermissionRequest(PermissionRequest),
    /// A tool was denied without asking (the permission mode or a rule decided).
    #[serde(rename_all = "camelCase")]
    PermissionDenied {
        tool_name: String,
        tool_use_id: Option<String>,
        message: Option<String>,
    },
    /// Tokens in the context after the last model response: input + cache reads + cache writes.
    #[serde(rename_all = "camelCase")]
    Usage {
        context_tokens: i64,
    },
    /// The turn is over: `ok` is false for errors and interrupts (`subtype`
    /// `error_during_execution`, `error_max_turns`…).
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        ok: bool,
        subtype: String,
        session_id: Option<String>,
        result: Option<String>,
        cost_usd: Option<f64>,
        context_window: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ChatEvent {
    Stream(StreamEvent),
    Host(HostEvent),
}

/// Payload of `nodal://chat`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEnvelope {
    pub chat_id: String,
    pub event: ChatEvent,
}

/// What a chat's process is doing now, for a UI that (re)opens the chat.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLive {
    pub state: RunState,
    /// Permission requests waiting for an answer, oldest first.
    pub pending: Vec<PermissionRequest>,
}

/// Claude Code's configured defaults (`model`, `effortLevel`), so the UI can say what
/// "default" means.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDefaults {
    /// As written in the settings (`opus`, `opus[1m]`, a full model id…); `None` if unset.
    pub model: Option<String>,
    pub effort: Option<String>,
}
