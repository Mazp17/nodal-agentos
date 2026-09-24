---
name: nodal-tasks
description: Create, update and follow tasks on a Nodal board through the `nodal` MCP server (list_projects, list_tasks, get_task, create_task, update_task, get_run). Use when work should be queued as a task instead of done now — follow-ups found while finishing a task, a plan to split into tasks, work for another repo — or to check a task's status or a run's result.
---

# Nodal tasks

Nodal is a macOS app that keeps a board of tasks per project and runs each task with an executor
(plain Claude, an agent or a workflow). While Nodal is open, the `nodal` MCP server lets you read
the board, create and update tasks, and read the result of runs. You cannot launch runs: a person
launches them from the app.

If the tools fail with "Open Nodal first", the app is not running. Say so and stop; do not retry in
a loop and do not fall back to another tracker unless you are asked to.

## Vocabulary

- **Project**: a group of repos with a `key` (`PAY`). Task keys are `{key}-{number}` (`PAY-12`).
  Every tool that takes a task accepts the key or the internal id (`t01…`).
- **Repo**: a git repo of one project. Every task belongs to exactly one repo, and runs happen there.
- **Task**: title, plan (markdown), acceptance criteria, priority, labels, executor and status.
- **Status**: `backlog`, `todo`, `in_progress`, `in_review`, `blocked`, `done`, `canceled`.
- **Run**: one execution of a task. `kind: work` does the work; `kind: review` checks it against the
  acceptance criteria and points to the work run with `parentRunId`.

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

The executor gets the title, the plan and the acceptance criteria and nothing else: it has not seen
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

**Acceptance criteria**: the reviewer checks each one and fails the run if one is unmet, so each
must be checkable by reading the diff or running a command:

- good: "`cargo test --lib` passes and a test covers an expired signature";
- good: "An invalid signature returns 401 and nothing is written to the database";
- bad: "The code is clean", "It works well".

Three to six criteria is usually right. Put the tests you expect in the criteria, not only in the plan.

**Priority**: `urgent`, `high`, `medium`, `low` or `none` (default). Use `urgent` only for
something broken for users now. **Labels**: short and reused; check the labels existing tasks use.

**Status**: new tasks start in `todo` (ready to run). Use `backlog` for ideas, or when the task
needs a person's decision before it can run.

## How to pick the repo and the executor

**Repo**: call `list_projects`; it returns each project with its repos (id, name, path). Pass
`repo` as the repo id, its name, or a path at or inside it — the path of the repo you are working
in is usually the right answer for follow-ups. If two projects have a repo with the same name, also
pass `project` (key or id). A task that touches two repos is two tasks, one per repo.

**Executor** (`executor`): omit it unless you have a reason. Without one, the task uses the repo's default,
then the project's, then the app's (plain Claude at the end). Set it when the work clearly fits a
specialist:

- `{"kind": "claude"}`: a plain session with the task as the prompt;
- `{"kind": "agent", "name": "<agent>", "source": "user" | "repo" | "plugin"}`: an agent defined in
  `~/.claude/agents/<agent>.md` (`user`), `<repo>/.claude/agents/<agent>.md` (`repo`) or an enabled
  plugin (`plugin`, named `<plugin>:<agent>`);
- `{"kind": "workflow", "name": "<workflow>"}`: a workflow from `~/.claude/workflows` or
  `<repo>/.claude/workflows`.

Only name an agent or a workflow you have seen exist in one of those places: the name is not
checked when the task is created, and the run fails when it is launched. To go back to the default,
call `update_task` with `"executor": null`.

## Updating tasks

`update_task` changes only the fields you pass. Useful moves:

- refine the plan or the acceptance criteria after learning something;
- `done` or `canceled` when the work is finished or no longer needed (the queue never overrides
  those two);
- `blocked` with the reason in the plan, when it cannot go on without someone;
- `repo` to move it to another repo of the same project (refused while a run is pending or the task
  has a worktree).

Setting `in_progress` does not launch anything. Tasks imported from Linear only accept status,
plan, acceptance criteria, executor and repo; their title, priority and labels come from Linear.

## How to read a run's result

`get_task` returns the task, its plan text and its latest runs (newest first). `get_run` with
`task` returns the latest work run of the task and its review; with `runId`, that run.

Read it in this order:

1. **`run.status`**: `queued`, `launching` or `launched` means it is not over; check again later
   instead of waiting in a loop. `failed` or `canceled` means it did not finish: read `run.error`.
2. **`run.outcome`** once `finished`:
   - `green`: the executor reports the work done;
   - `yellow`: done with caveats (workflows) — read the summary before trusting it;
   - `red`: the executor reports it is blocked;
   - `stopped`: someone stopped it;
   - `unknown`: it ended without a result Nodal could read.
3. **`run.summary`**: what the executor says it did, or why it is blocked. **`run.prUrl`** and
   **`run.branch`**: where the changes are.
4. **`review`** (may be `null`): the review of that work run. `review.verdict.pass` is the gate;
   `review.verdict.unmet` lists the acceptance criteria or problems that failed it, and
   `review.verdict.nits` minor suggestions that did not. A review still running has a `status` but no
   `verdict` yet. `null` means the task had no review, so `green` is only the executor's word.

The task status follows the result: a failed, stopped or `red` run, or a failed review, moves the
task to `blocked`; a passed review (or a `green`/`yellow` run without review) moves it to
`in_review`, waiting for a person. So a `blocked` task is the one to look at: read the summary and
the unmet criteria, then fix the plan or the criteria with `update_task`, or create a follow-up
task, and tell the user it is ready to be launched again.

Report a run's result to the user as it is: say "the review failed on X" rather than "done" when
`verdict.pass` is false, and do not call a change merged or shipped because a run is `green`.

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

Then tell the user the key it got (`PAY-13`) and that it is on the board, not launched.
