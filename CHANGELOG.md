# Changelog

All notable changes to Nodal are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/). Until 1.0, a minor release can include breaking changes.

## [Unreleased]

### Added

- MCP server for agents: while Nodal is open, `nodal-mcp` (`claude mcp add nodal -- nodal-mcp`) lists projects and tasks, creates and updates tasks and reads run results, through a local Unix socket readable only by you. Launching runs stays in the app.
- `nodal-tasks` agent skill (`skills/nodal-tasks`, `npx skills add Mazp17/nodal-agentos --skill nodal-tasks`): when to create a task instead of doing the work, how to write its plan and acceptance criteria, how to pick the repo and the executor, and how to read a run's result.

### Changed

- Every run is launched with `--append-system-prompt` telling it it's unattended, so it reports and stops instead of ending on a question and leaving the task In Progress.
- New Nodal logo: app icon and in-app mark.
- Redesigned New task dialog: repo picker in the header, Write/Preview for the plan, "Done when" criteria with suggestions from the plan, and a collapsible Execution section. Save with ⌘↵.
- The `plan.md` written for tasks imported from Linear is now in English ("Subtasks", "_No description._" and the footer).

### Fixed

- The executor picker at the foot of the task panel no longer gets clipped, so you can search and pick an executor.

## [0.2.0] - 2026-09-24

### Added

- In-app updates: Nodal checks for a new version on launch, once a day while it's open, and from Nodal → Check for Updates… or Settings → Updates; shows the release notes and installs and restarts only if you choose Update. The sidebar footer shows the installed version, or "Update to x.y.z" when there is a newer one. Debug builds don't check. Coming from 0.1.0, download this version by hand one last time.

### Changed

- The task panel opens with the latest run (status, phases, result and its actions) and a highlighted launch card; the plan and step history follow.

## [0.1.0] - 2026-09-24

First public alpha (macOS only).

### Added

- Local projects that group one or more git repos; every task belongs to one repo.
- Tasks with a markdown plan (inline or a `.md` file in the repo), acceptance criteria, priority and labels, on a board you can reorder.
- Executors: an agent (`~/.claude/agents`, the repo's `.claude/agents` or a plugin), a workflow (`~/.claude/workflows` or the repo's) or plain Claude, plus a global default executor.
- Isolation per task in its own git worktree and branch (`~/.nodal/worktrees/…`, `nodal/<task>`), or in place. Guarded worktree clean-up.
- Finish modes: leave changes uncommitted, commit, or commit, push and open a pull request.
- Automatic read-only review against the acceptance criteria; pass moves the task to In Review, fail to Blocked with the findings.
- Hand-offs between executors on the same branch.
- One global run queue with a concurrency limit and manual reordering.
- Run detail: phases, subagents grouped by phase, transcripts, token usage and results rendered as markdown.
- Diff of each run's changes, with "Open in editor" in your installed code editor.
- Activity view with the Claude Code sessions running in each repo, including the ones Nodal did not start.
- Linear integration: import issues, routing rules by project or label, state mapping, status push, closing comments and moved-task detection. API key stored in the macOS Keychain.
- Diagnostics: `claude` and `git` versions and Claude Code trust per repo.
- Command palette (⌘K) and Dock badge with runs waiting for you.
- Import of data from earlier development versions.
- Universal `.dmg` for Apple Silicon and Intel. Not notarized by Apple: see the install steps in the README.

[Unreleased]: https://github.com/Mazp17/nodal-agentos/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Mazp17/nodal-agentos/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Mazp17/nodal-agentos/releases/tag/v0.1.0
