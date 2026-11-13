# Nodal

[![CI](https://github.com/Mazp17/nodal-agentos/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Mazp17/nodal-agentos/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Mazp17/nodal-agentos?include_prereleases&sort=semver&label=release)](https://github.com/Mazp17/nodal-agentos/releases)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple)](#install)
[![License: MIT](https://img.shields.io/github/license/Mazp17/nodal-agentos)](LICENSE.md)

**A desktop workspace for Claude Code.** Chat with Claude across all your projects, answer what it needs from one inbox, and turn conversations into tasks that run in their own worktree.

[Español](README.es.md)

![Nodal overview](docs/screenshots/overview.png)

> macOS and Windows. Independent project, not affiliated with Anthropic.

## Install

1. Download the latest `.dmg` from [Releases](https://github.com/Mazp17/nodal-agentos/releases) (Apple Silicon and Intel) and drag Nodal to Applications.
2. Nodal is not notarized by Apple, so macOS blocks the first launch. Run once:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Nodal.app
   ```

After that, Nodal updates itself: it checks for a new version on launch and once a day (and from Nodal → Check for Updates… or Settings → Updates), shows the release notes and installs it only if you choose Update. Updates don't need the `xattr` step again. Nodal 0.1.0 predates in-app updates, so from 0.1.0 download the next version by hand one last time.

You need [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and logged in, and `git`. `gh` is optional, for tasks that open pull requests. Open `claude` once in each repo and accept the trust dialog before running tasks there.

## What it does

- **Chats per project**: talk to Claude about a whole project or one repo, with your own agent (`~/.claude/agents`, the repo's or a plugin's), model and effort. Tool calls, diffs and subagents show live. Sessions show up in `claude --resume`.
- **Overview inbox**: approvals and failed chats waiting on you, what just finished, your projects, and a summary of the tokens Claude Code used on this machine.
- **Tasks as chats**: plan tasks on a board (markdown plans, acceptance criteria, priority, labels). Opening a task starts a chat in its own git worktree and branch, so several run on the same repo at once; its status follows the chat.
- **Git in the app**: files panel and editor, commit all or picked files, diff against the branch base, open a pull request with `gh` or ask the chat.
- **Linear, optional**: import issues, route them to repos and keep statuses in sync.

## How it works

Nodal drives the `claude` CLI you already have, in `claude -p` stream-json mode, and reads history and token usage from the session files under `~/.claude/projects`. Parts of that protocol and those files are undocumented, so a Claude Code update can break them.

Everything stays on your Mac: an SQLite database, worktrees under `~/.nodal`, API keys in the Keychain. No network server, no telemetry.

### Agents (MCP)

While Nodal is open, agents can list projects and tasks, create and update tasks and open a task's chat through MCP. The app listens on a Unix socket in its data folder (readable only by you, no network port) and `nodal-mcp`, which ships inside the app, bridges it to stdio. Register it:

```bash
claude mcp add nodal -- /Applications/Nodal.app/Contents/MacOS/nodal-mcp
```

From source, build it with `cargo build --release --bin nodal-mcp` in `src-tauri` (after `pnpm build`) and register `src-tauri/target/release/nodal-mcp` instead.

Tools: `list_projects`, `list_tasks`, `get_task`, `create_task`, `update_task` and `start_task_chat`. Nodal's own chats launch it with `--chat`, which adds `propose_task`: the chat shows a task card and nothing is created until you accept it. A debug build of `nodal-mcp` talks to a debug build of the app.

The [`nodal-tasks` skill](skills/nodal-tasks/SKILL.md) teaches agents when to create a task instead of doing the work, how to write its plan and acceptance criteria, how to pick the repo, and how to start a task's chat. Install it with [skills](https://skills.sh):

```bash
npx skills add Mazp17/nodal-agentos --skill nodal-tasks
```

## Build from source

Needs Node.js 20+, pnpm 10 and stable Rust.

```bash
git clone https://github.com/Mazp17/nodal-agentos.git nodal
cd nodal
pnpm install
pnpm tauri dev
```

Built with [Tauri 2](https://tauri.app) (Rust + SQLite) and React 19 + TypeScript. See [CONTRIBUTING.md](CONTRIBUTING.md) to work on it and [RELEASING.md](RELEASING.md) for how versions ship.

## License

[MIT](LICENSE.md) © Nodal contributors
