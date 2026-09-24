# Changelog

All notable changes to Nodal are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/). While the version is `0.x`, any release can include breaking changes.

## [Unreleased]

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
- Run detail: phases, subagents, transcripts, token usage and the worktree diff.
- Activity view with the Claude Code sessions running in each repo, including the ones Nodal did not start.
- Linear integration: import issues, routing rules by project or label, state mapping, status push, closing comments and moved-task detection. API key stored in the macOS Keychain.
- Diagnostics: `claude` and `git` versions and Claude Code trust per repo.
- Command palette (⌘K) and Dock badge with runs waiting for you.
- Import of data from earlier development versions.
