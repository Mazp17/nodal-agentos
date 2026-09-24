# Nodal

**Agent OS for Claude.** A desktop app to plan work, hand it to Claude Code agents, and watch them run across all your projects.

[Español](README.es.md)

> Status: early alpha, macOS only. Expect rough edges and breaking changes.
>
> Nodal is an independent open source project. It is not affiliated with or endorsed by Anthropic.

## Why

Claude Code runs well in a single terminal. It gets hard to follow once you have several repos, several tasks and several agents at the same time: what is running where, which run is waiting for a permission prompt, which one finished and needs review, which Linear issue it belongs to.

Nodal is the control room for that. You keep your tasks on a board, assign each one to an executor, and Nodal launches Claude Code in the background, isolates the work, reviews it and keeps the task status in sync.

## What it does

- **Local projects with multiple repos.** A project groups one or more local git repos. Every task belongs to exactly one repo.
- **Tasks with a plan.** Write the plan in markdown or point to a `.md` file in the repo. Add acceptance criteria, priority and labels.
- **Delegate to an executor.** Assign a task to:
  - an **agent** you already have (`~/.claude/agents`, the repo's `.claude/agents` or a plugin), e.g. `frontend-developer`;
  - a **workflow** (`~/.claude/workflows` or the repo's), e.g. a plan → implement → verify → review pipeline;
  - plain **Claude**.
- **Isolation per task.** By default each task gets its own git worktree and branch (`~/.nodal/worktrees/…`, `nodal/<task>`), so several tasks can run on the same repo at once. You can switch a task to work in place.
- **Finish modes.** Leave the changes uncommitted, commit them, or commit, push and open a pull request.
- **Automatic review.** When an agent finishes, a read-only reviewer checks the work against the acceptance criteria. Pass moves the task to In Review; fail moves it to Blocked with the findings, and you decide what happens next.
- **Hand-offs.** Pass a task from one executor to another on the same branch (e.g. `frontend-developer` → `code-reviewer`). The task keeps the whole chain of steps.
- **One global queue.** A single concurrency limit for every run, with a visible queue you can reorder.
- **Follow every run.** Phases, subagents, transcripts, tokens, the diff of the worktree, and the sessions Claude is running in each repo (including the ones Nodal did not start).
- **Task managers, optional.** Import issues from Linear and keep them linked: Nodal pushes status changes (In Progress, In Review, Blocked) and posts a closing comment. You map Linear states to Nodal states yourself; Nodal proposes the mapping. You can route issues to repos by Linear project or label. Asana, Azure DevOps and GitHub Issues are planned.

Nodal works with no task manager at all.

## How it works

Nodal does not replace Claude Code: it drives the `claude` CLI you already have installed.

- Runs are launched with `claude --bg`, optionally with `--agent <name>` or as `/<workflow> <args>`.
- Progress is read from `claude agents --json` and from the session files Claude Code writes under `~/.claude/projects`. **These files are an internal, undocumented format**, isolated in a single parser module; a Claude Code update can break it.
- Everything is local: an SQLite database in the app data folder, plans next to it, worktrees under `~/.nodal`. API keys for task managers live in the macOS Keychain. Nodal has no server and sends no telemetry.

## Requirements

- macOS (Keychain and Terminal integration are macOS-only for now).
- [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and logged in (`claude` on your `PATH` or in `~/.local/bin`).
- `git`. `gh` (GitHub CLI) if you want tasks to open pull requests.
- Claude Code must trust a repo before Nodal can run tasks in it: open `claude` once in the repo and accept the trust dialog. Nodal's Diagnostics screen shows which repos are trusted.

To build from source you also need Node.js 20+, pnpm 10 and a stable Rust toolchain.

## Getting started

```bash
git clone https://github.com/Mazp17/nodal-agentos.git nodal
cd nodal
pnpm install
pnpm tauri dev
```

On first launch Nodal asks you to create a project, add a repo and, optionally, connect Linear with a personal API key.

To produce an app bundle:

```bash
pnpm tauri build
```

## Tech stack

- [Tauri 2](https://tauri.app) with a Rust backend (`src-tauri/`): SQLite via `rusqlite`, the run queue, worktrees, task-manager sync.
- React 19 + TypeScript + Vite frontend (`src/`).

See [CONTRIBUTING.md](CONTRIBUTING.md) for the project layout and how to work on it.

## License

[MIT](LICENSE.md) © Nodal contributors
