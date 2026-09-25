## What and why

<!-- One or two lines. Link the issue: "Closes #123". -->

## How I tested it

<!-- Steps, and screenshots for UI changes. -->

## Checklist

- [ ] `pnpm tsc --noEmit` and `pnpm build` pass
- [ ] `cargo test --lib` and `cargo clippy --all-targets -- -D warnings` pass (in `src-tauri/`)
- [ ] Rust commands/types and `src/domain/api.ts` / `types.ts` are in sync
- [ ] Database changes are an additive migration with a test
- [ ] No real data in fixtures, tests or screenshots; no API keys
- [ ] The title follows Conventional Commits and reads well as a changelog entry
