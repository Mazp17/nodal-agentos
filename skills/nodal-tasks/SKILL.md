---
name: nodal-tasks
description: Create, update and start work on tasks on a Nodal board through the `nodal` MCP server (list_projects, list_tasks, get_task, create_task, update_task, start_task_chat). Use when work should be queued as a task instead of done now — follow-ups found while finishing a task, a plan to split into tasks, work for another repo — or to open the chat that works a task.
---

# Nodal tasks

Nodal is a macOS app that keeps a board of tasks per project; each task is worked in a Claude chat
in its own git worktree. While Nodal is open, the `nodal` MCP server lets you read the board,
create and update tasks, and open a task's chat. Start a task chat only when the user asks for it:
it spends their tokens.

If the tools fail with "Open Nodal first", the app is not running or its MCP server is off. Say so and stop; do not retry in
a loop and do not fall back to another tracker unless you are asked to.

## Vocabulary

- **Project**: a group of repos with a `key` (`PAY`). Task keys are `{key}-{number}` (`PAY-12`).
  Every tool that takes a task accepts the key or the internal id (`t01…`).
- **Repo**: a git repo of one project. Every task belongs to exactly one repo, and its chat works there.
- **Task**: title, plan (markdown), acceptance criteria, priority, labels and status.
- **Status**: `backlog`, `todo`, `in_progress`, `in_review`, `blocked`, `done`, `canceled`.

## When to create a task instead of doing the work now

Create a task when:

- the work is outside what you were asked to do (a bug you noticed, a refactor, missing tests,
  docs that drifted). Note it as a task and keep your change focused;
- you are finishing a task and there are follow-ups: things you left out on purpose, known limits,
  risks the reviewer should know about later;
- you are planning and the plan has steps that can be run and reviewed on their own: one task per
  step, not one task for everything;
- the work belongs to another repo, needs a person's decision first, or is too big for this session;
- the user asks for it ("leave it for later", "make it a task").

Do the work now instead when it is small, inside your scope, and you have what you need to finish
and verify it. A task costs a person's attention on the board: do not create tasks for trivia, and
never create a task just to report what you already did.

Before creating, call `list_tasks` for the project (statuses `backlog`, `todo`, `in_progress`,
`blocked`) and check that the task does not already exist. If it does, update it instead;
otherwise call `create_task`. It puts the task on the board and does not launch it.

## How to write a good task

The agent gets the title, the plan and the acceptance criteria and nothing else: it has not seen
your conversation. Write for someone who starts from zero in the repo.

**Title**: one line, under 200 characters, imperative and specific.
"Validate the webhook signature before parsing the body", not "Webhook fixes".

**Plan** (markdown), in this order:

1. **Context**: why the task exists and what is wrong today, with the evidence (error text, the
   failing command, the file and line where it happens).
2. **Plan**: the steps, each one concrete. Name files, functions, commands and the existing code to
   reuse. Say what is out of scope when there is a tempting neighbour.
3. **Notes** (optional): decisions already made, constraints, links to related tasks by key.

Keep it the size of the work: a few lines for a small fix, sections for a feature. Do not paste
long logs; quote the lines that matter. Use made-up names in examples, never secrets.

When the plan already lives in the repo as a `.md` file, pass `planFile` (a path inside the repo)
instead of `plan`, so the task follows the file.

**Acceptance criteria**: the reviewer checks each one, so each
must be checkable by reading the diff or running a command:

- good: "`cargo test --lib` passes and a test covers an expired signature";
- good: "An invalid signature returns 401 and nothing is written to the database";
- bad: "The code is clean", "It works well".

Three to six criteria is usually right. Put the tests you expect in the criteria, not only in the plan.

**Priority**: `urgent`, `high`, `medium`, `low` or `none` (default). Use `urgent` only for
something broken for users now. **Labels**: short and reused; check the labels existing tasks use.

**Status**: new tasks start in `todo` (ready to run). Use `backlog` for ideas, or when the task
needs a person's decision before it can run.

## How to pick the repo

**Repo**: call `list_projects`; it returns each project with its repos (id, name, path). Pass
`repo` as the repo id, its name, or a path at or inside it — the path of the repo you are working
in is usually the right answer for follow-ups. If two projects have a repo with the same name, also
pass `project` (key or id). A task that touches two repos is two tasks, one per repo.

## Proposing a task from a Nodal chat

Inside a chat in the Nodal app, the server also has `propose_task`. It takes the same fields as
`create_task` (`repo`, `project`, `title`, `plan`, `acceptance`, `priority`, `labels`,
`status`; `plan` is required and there is no `planFile`) and validates them the same way, but
creates nothing: the chat shows the task as a card and the user creates it with one click. It
returns the task as it would be created (`newTask`), with `created: false`. In a chat, prefer it to
`create_task` unless the user asks you to create the task yourself. Outside a chat the tool is not
listed.

## Reading tasks

`list_tasks` lists a project's tasks by status; `get_task` returns one task with its plan text.

## Updating tasks

`update_task` changes only the fields you pass. Useful moves:

- refine the plan or the acceptance criteria after learning something;
- `done` or `canceled` when the work is finished or no longer needed;
- `blocked` with the reason in the plan, when it cannot go on without someone;
- `repo` to move it to another repo of the same project (refused while the task has a worktree).

Tasks imported from Linear only accept status,
plan, acceptance criteria and repo; their title, priority and labels come from Linear.

## Starting work on a task

`start_task_chat` takes a `task` and opens the chat that works it, in the task's own worktree: it
returns the existing chat, or creates one, sends the task as the first message and moves the task
to `in_progress`. Returns `chatId`, `title` and `taskId`. Tell the user the chat is open in Nodal;
follow the work there. When a turn leaves changes the task moves to `in_review` on its own.
Do it only when the user asks.

## Example

A finished task left a follow-up in the repo at `/Users/me/Code/acme-web`:

```json
{
  "repo": "/Users/me/Code/acme-web",
  "title": "Retry failed invoice emails with backoff",
  "plan": "## Context\n\nPAY-12 sends invoice emails once; a timeout from the mail provider drops the email (see `src/mail/send.ts:48`).\n\n## Plan\n\n1. Wrap `sendInvoiceEmail` in the existing `retry()` helper from `src/util/retry.ts`, 3 attempts, exponential backoff.\n2. Log the final failure with the invoice id.\n\nOut of scope: moving emails to a queue.",
  "acceptance": [
    "A timeout on the first two attempts still sends the email",
    "After three failures the invoice id is logged at error level",
    "`pnpm test` passes with new tests for both cases"
  ],
  "priority": "medium",
  "labels": ["email"]
}
```

Then tell the user the key it got (`PAY-13`) and that it is on the board.
