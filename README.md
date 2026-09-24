# Nodal

[![CI](https://github.com/Mazp17/nodal-agentos/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Mazp17/nodal-agentos/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Mazp17/nodal-agentos?include_prereleases&sort=semver&label=release)](https://github.com/Mazp17/nodal-agentos/releases)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple)](#install)
[![License: MIT](https://img.shields.io/github/license/Mazp17/nodal-agentos)](LICENSE.md)

**Agent OS for Claude.** Plan tasks on a board, hand them to Claude Code agents and follow every run across all your repos.

[Español](README.es.md)

![Nodal board](docs/screenshots/board.png)

> Early alpha, macOS only. Independent project, not affiliated with Anthropic.

## Install

1. Download the latest `.dmg` from [Releases](https://github.com/Mazp17/nodal-agentos/releases) (Apple Silicon and Intel) and drag Nodal to Applications.
2. Nodal is not notarized by Apple, so macOS blocks the first launch. Run once:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Nodal.app
   ```

You need [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and logged in, and `git`. `gh` is optional, for tasks that open pull requests. Open `claude` once in each repo and accept the trust dialog before running tasks there.

## What it does

- **Board of tasks** grouped in projects, each project with one or more local repos. Plans in markdown, acceptance criteria, priority and labels.
- **Delegate** each task to one of your agents (`~/.claude/agents`, the repo's or a plugin), a workflow, or plain Claude.
- **Isolated by default**: every task gets its own git worktree and branch, so several run on the same repo at once.
- **Finish** with uncommitted changes, a commit, or a pull request.
- **Automatic review** against the acceptance criteria: pass goes to In Review, fail to Blocked with the findings.
- **One queue** with a global concurrency limit, and a live view of each run: phases, subagents, transcript, tokens and diff.
- **Linear, optional**: import issues, route them to repos and keep statuses in sync. More task managers are planned.

## How it works

Nodal drives the `claude` CLI you already have (`claude --bg`, `--agent`, `/<workflow>`) and reads progress from `claude agents --json` and the session files under `~/.claude/projects`. Those files are an undocumented internal format, so a Claude Code update can break the parsing.

Everything stays on your Mac: an SQLite database, worktrees under `~/.nodal`, API keys in the Keychain. No server, no telemetry.

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
