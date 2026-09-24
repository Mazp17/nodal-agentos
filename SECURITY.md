# Security Policy

## Supported versions

Nodal is in early alpha. Only the latest commit on `main` gets security fixes.

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Report it privately through GitHub: go to the repository's **Security** tab and click **Report a vulnerability**. Include:

- what the issue is and where it lives (file, command, screen);
- steps to reproduce, or a proof of concept;
- the impact you expect (what an attacker gets);
- your macOS, Nodal (commit) and Claude Code (`claude --version`) versions.

Remove real API keys, private paths and company data from anything you send.

You should get an acknowledgement within 7 days. We will keep you posted while we work on a fix and credit you in the release notes unless you prefer otherwise. Please give us a reasonable time to ship a fix before disclosing the issue publicly.

## What Nodal touches

Knowing this helps decide whether something is a vulnerability:

- **Processes.** Nodal spawns `claude`, `git`, `gh` and `osascript` with argument arrays, never through a shell. Task fields, branch names and paths are validated before they become arguments.
- **Claude Code permissions.** Runs use the permission mode configured per repo, which can be as permissive as `bypassPermissions`. The automatic reviewer always runs with `dontAsk` and a read-only allowlist, whatever the repo is configured with.
- **Secrets.** Task-manager API keys live in the macOS Keychain (service `com.nodal.app`). They are never sent to the frontend or written to logs; the UI only shows the last 4 characters.
- **Network.** The only outbound calls go to the task-manager APIs you connect (today, Linear). No server, no telemetry.
- **Local data.** An SQLite database and plans in the app data folder, worktrees under `~/.nodal`. Nodal reads Claude Code's session files under `~/.claude` (or `CLAUDE_CONFIG_DIR`).
- **Webview.** Content from task managers (issue descriptions, comments) is rendered as Markdown without raw HTML, under a strict CSP. External links open in the system browser.

## In scope

- Command or argument injection through task data, issue content, repo paths or branch names.
- Writing or deleting files outside the repo, its worktree or Nodal's own folders.
- API keys leaking to the frontend, logs, the database, transcripts or the network.
- Script execution or navigation in the webview from rendered content.
- The reviewer being able to modify files or run commands outside its allowlist.

## Out of scope

- What an agent does inside a repo with the permission mode you configured for it. That is Claude Code's behavior; report it to Anthropic.
- Bugs in Claude Code, `git`, `gh` or the Linear API themselves.
- Attacks that need an attacker already running code as your macOS user.
- Missing hardening without a concrete impact.
