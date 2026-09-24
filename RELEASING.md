# Releasing Nodal

## Versions

Nodal follows [Semantic Versioning](https://semver.org/). While the version is `0.x`:

- **MINOR** (`0.1.0` → `0.2.0`): new features, or anything that breaks what users already rely on (data, settings, behavior).
- **PATCH** (`0.2.0` → `0.2.1`): fixes only.
- `1.0.0` comes when the app is declared stable. Until then every GitHub release is marked as a pre-release.

The version lives in `package.json` (`tauri.conf.json` reads it from there) and in `src-tauri/Cargo.toml`. `pnpm release:bump <x.y.z>` updates both, plus `Cargo.lock`.

## When to release

On demand, not on a schedule. Cut a release when all of these hold:

1. `[Unreleased]` in `CHANGELOG.md` has changes a user would notice.
2. CI is green on `main`.
3. There are no known bugs that block normal use.
4. The `.dmg` passes the smoke test below.

Exception: a serious bug (data loss, the app does not start, a security issue) gets a PATCH release right away, even if it is the only change.

Everything ships from `main`; there are no release branches while there is a single maintainer.

## How to release

`main` only accepts squash-merged pull requests with green CI, so the release commit goes through a PR too.

1. Branch off `main`:

   ```bash
   git switch main && git pull
   git switch -c release/vx.y.z
   ```

2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD` and add a new empty `## [Unreleased]` above it. If the release changes the database schema, say so.
3. Bump the version:

   ```bash
   pnpm release:bump x.y.z
   ```

4. Commit, push and open the PR. Squash-merge it once CI is green:

   ```bash
   git commit -am "chore(release): vx.y.z"
   git push -u origin release/vx.y.z
   gh pr create --fill
   ```

5. Tag the merged commit on `main` with a signed tag and push only the tag:

   ```bash
   git switch main && git pull
   git tag -s vx.y.z -m "Nodal x.y.z"
   git push origin vx.y.z
   ```

6. The tag starts the [Release workflow](.github/workflows/release.yml). It checks that the tag matches `package.json` and that the changelog has the section, builds a universal `.dmg` (Apple Silicon and Intel) with its SHA-256 checksum and creates a **draft** release with the changelog section and install instructions as notes.
7. Download the `.dmg` from the draft and run the smoke test.
8. If it passes, publish the draft. If not, delete the draft first and then the tag, fix on `main` through a PR and start again from step 5:

   ```bash
   gh release delete vx.y.z --yes
   git push --delete origin vx.y.z && git tag -d vx.y.z
   ```

Nothing is published automatically: the last step is always manual. Release tags (`v*`) can't be moved or deleted except by a repository admin.

## Smoke test

On a Mac, with the downloaded `.dmg`:

- [ ] Install it, run `xattr -dr com.apple.quarantine /Applications/Nodal.app` and open the app.
- [ ] Nodal → About Nodal shows the release version.
- [ ] Data from the previous version is still there (projects, tasks, runs).
- [ ] Create a task in a test repo and run it to completion.
- [ ] If Linear is connected: the task's status reaches Linear.

## Signing

Releases are **not** signed with an Apple Developer ID nor notarized: the app is ad-hoc signed, which lets it run on Apple Silicon, and macOS blocks the first launch until the quarantine flag is removed. The release notes explain how. Adding Developer ID signing and notarization later only needs the certificates as repository secrets; the workflow and this process stay the same.
