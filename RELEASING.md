# Releasing Nodal

This is how a new version of Nodal gets out, from deciding it's time to publishing the `.dmg`.

## Version numbers

Versions follow [Semantic Versioning](https://semver.org/). Bump the patch number (`0.2.0` → `0.2.1`) when a release only fixes bugs, and the minor number (`0.2.0` → `0.3.0`) when it adds something new. Nodal hasn't reached 1.0 yet, so a minor release can also change or break existing behavior. When it does, the changelog says so.

We'll move to 1.0 once the app is stable enough to promise compatibility between versions.

Only versions with a suffix (`0.3.0-beta.1`) are marked as pre-releases on GitHub. Installed apps look for updates in the `latest.json` of the latest release, and GitHub skips pre-releases there, so a plain `x.y.z` reaches every user once it's published and a suffixed one reaches nobody until a plain version follows it.

The version is set in `package.json` and `src-tauri/Cargo.toml`. Don't edit them by hand: `pnpm release:bump x.y.z` updates both, along with `Cargo.lock`.

## When to release

There's no fixed schedule. We release when there's something worth shipping, which in practice means:

- the `[Unreleased]` section of `CHANGELOG.md` has changes users will notice,
- CI is green on `main`,
- there are no known bugs that get in the way of normal use,
- and the build passes the smoke test described below.

A serious bug, like data loss, the app not starting or a security problem, is the exception: fix it and ship a patch release right away, even if nothing else has changed.

All releases come from `main`. We don't keep release branches.

## Cutting a release

Since `main` only takes pull requests, the version bump goes through one too.

Start a branch from an up-to-date `main`:

```bash
git switch main && git pull
git switch -c release/vx.y.z
```

In `CHANGELOG.md`, rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD` and add an empty `## [Unreleased]` above it. If the release changes the database schema, mention it there. Then bump the version and open the PR:

```bash
pnpm release:bump x.y.z
git commit -am "chore(release): vx.y.z"
git push -u origin release/vx.y.z
gh pr create --fill
```

Once CI passes, squash-merge it. Then tag the merge commit on `main` with a signed tag and push the tag:

```bash
git switch main && git pull
git tag -s vx.y.z -m "Nodal x.y.z"
git push origin vx.y.z
```

Pushing the tag starts the [release workflow](.github/workflows/release.yml). It makes sure the tag matches the version in `package.json` and that the changelog has a section for it, then builds a universal `.dmg` for Apple Silicon and Intel, computes its SHA-256 and creates a draft release. The draft's notes are that version's changelog section plus the install instructions.

The draft also carries what the in-app updater needs: the update bundle (`Nodal_x.y.z_universal.app.tar.gz`), its signature (`.sig`) and `latest.json`, with the version, the changelog section as release notes, the date and the bundle's URL and signature. Installed apps only see it once the draft is published.

Download the `.dmg` from the draft and go through the smoke test. If everything works, publish the draft. If something is wrong, delete the draft and then the tag, fix the problem on `main` with a regular PR and tag again:

```bash
gh release delete vx.y.z --yes
git push --delete origin vx.y.z && git tag -d vx.y.z
```

Release tags are protected: only a repository admin can move or delete them.

## Smoke test

Before publishing, check the downloaded build on a Mac:

- [ ] It installs, and opens after running `xattr -dr com.apple.quarantine /Applications/Nodal.app`.
- [ ] Nodal → About Nodal shows the new version.
- [ ] Updating from the previous version works: with the previous release installed and the draft published, Nodal offers "Nodal x.y.z is available" on launch (or from Nodal → Check for Updates…), and Update installs it and restarts into the new version. If it doesn't, turn the release back into a draft right away so nobody else gets it.
- [ ] Projects, tasks and runs from the previous version are still there.
- [ ] A task in a test repo runs to completion.
- [ ] If Linear is connected, the task's status shows up in Linear.

## Update signing key

Updates are signed with a separate key, unrelated to Apple signing. The app only installs bundles signed with the private half of the public key in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). The release workflow signs with the `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repository secrets and fails early if the key is missing.

The key pair was made with `pnpm tauri signer generate`. Keep a backup of the private key and its password outside GitHub: if they're lost, installed apps can't be updated anymore and everyone has to download a build with a new public key by hand.

## About signing

The app isn't signed with an Apple Developer ID or notarized, because that requires a paid Apple developer account. It is ad-hoc signed, which is enough for it to run on Apple Silicon, but macOS still blocks the first launch until the quarantine flag is removed. The release notes and the README explain how to do that.

If the project gets a Developer ID later, signing and notarization only need the certificates added as repository secrets. The rest of this process stays the same.
