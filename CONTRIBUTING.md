# Contributing to Nodal

Thanks for your interest in Nodal. Issues, ideas and pull requests are welcome.

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Before you start

- For anything bigger than a small fix, open an issue first to agree on the approach.
- Nodal is macOS-only for now. Work toward Linux/Windows support is welcome, but please discuss it in an issue first.
- Nodal is independent from Anthropic. Do not add anything that implies otherwise.

## Setup

Requirements: macOS, Node.js 20+, pnpm 10, a stable Rust toolchain (with `clippy`), `git`, and [Claude Code](https://docs.claude.com/en/docs/claude-code) if you want to run tasks for real.

```bash
pnpm install
pnpm tauri dev        # run the app with hot reload
```

## Project layout

```
src-tauri/src/          Rust backend (Tauri commands)
  domain/               core types: Project, Repo, Task, Run, SourceLink…
  db/                   SQLite connection, versioned schema (PRAGMA user_version), queries
  work/                 the single run queue: launch, executors, worktrees, review, transitions, diff
  runs/                 talking to the `claude` CLI, reading sessions and transcripts
  providers/, linear/   task-manager integration (Linear), sync worker, state mapping, import
  activity/             what Claude is doing in each repo
  migrate/              import of data from earlier versions
  secrets.rs            API keys in the macOS Keychain
  events.rs             `nodal://changed` events for the frontend
src/                    React + TypeScript frontend
  domain/               typed command wrappers (api.ts), shared types, data store and hooks
  shell/                app shell, sidebar, topbar, navigation, command palette
  features/             board, tasks, executors, runs, activity, projects, providers, settings, onboarding
  ui/                   shared components (dialogs, markdown, links…)
  styles/               design tokens and base styles
```

`src/domain/api.ts` and `src/domain/types.ts` mirror the Rust commands and types. When you add or change a command, update both sides in the same change.

## Checks

CI runs these on every pull request; run them before opening one. All must pass:

```bash
pnpm tsc --noEmit
pnpm build                                    # includes lint:no-native-dialogs
cd src-tauri
cargo test --lib
cargo clippy --all-targets -- -D warnings
```

**Coverage.** Rust line coverage must stay at or above **75%**; CI fails below that and posts a coverage report on the pull request. New logic comes with tests. To check locally (needs `cargo install cargo-llvm-cov` and `rustup component add llvm-tools-preview`):

```bash
cd src-tauri
cargo llvm-cov --lib --summary-only
```

Tests that hit real services or real local data are `#[ignore]`. Linear live tests read `LINEAR_API_KEY` from the environment and only read data. Never commit a key.

## Guidelines

- **No real data.** Fixtures, examples, screenshots and tests must use made-up names (`acme`, `Jane Doe`, `/Users/me/...`). No company names, real people, real paths or real issue ids.
- **Claude Code internals.** Nodal reads files that Claude Code writes but does not document. Keep that parsing inside `src-tauri/src/runs/claude_fs.rs` and `src-tauri/src/activity/claude_sessions.rs`, with fixtures, so a format change is fixed in one place.
- **No shells.** External commands (`claude`, `git`, `gh`, `osascript`) are spawned with argument arrays, never through a shell, and user input is validated before it becomes an argument.
- **Database changes** are additive migrations: bump `PRAGMA user_version` and add a migration test from the previous version.
- **Native dialogs don't work under Tauri.** Don't use `window.confirm`, `alert` or `prompt`; use `useConfirm()` from `src/ui/ConfirmDialog.tsx`. `pnpm build` fails if you do.
- **Links** to external sites open through the opener plugin (`src/ui/ExternalLink.tsx`), never by navigating the webview.
- **UI text is in English.** Code comments are currently mixed English/Spanish; new comments in English are preferred.
- Keep changes focused. Match the style of the surrounding code.

## Commits and pull requests

- Short commit messages; the subject line is usually enough (`feat(board): reorder tasks in one call`).
- Pull request descriptions should be brief: what changed, why, and how you tested it. Long reasoning belongs in code comments or the changelog.
- Include screenshots for UI changes.
- Add user-visible changes to [CHANGELOG.md](CHANGELOG.md) under `[Unreleased]`.
- Releases are cut by the maintainer following [RELEASING.md](RELEASING.md).

## Reporting bugs

Open an issue with your macOS and Claude Code versions (`claude --version`), what you did, what you expected and what happened. Remove API keys, private paths and company data from logs before posting.

Security issues go through a private report, not a public issue: see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE.md).
