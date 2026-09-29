//! Probing a repo folder for things the app wants to know about it: which test commands it
//! looks like it has, and where `nodal-mcp` lives next to the running binary.

use std::path::{Path, PathBuf};

/// Test commands of the repo in `cwd`, detected by its files (fixed list: never text from
/// the repo). They go as `Bash(<cmd>:*)` in the reviewer's allowed tools.
pub fn test_commands(cwd: &Path) -> Vec<String> {
    let has = |f: &str| cwd.join(f).is_file();
    let mut out: Vec<&str> = Vec::new();
    let npm_test = std::fs::read_to_string(cwd.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["scripts"]["test"].is_string());
    if npm_test {
        out.push(if has("pnpm-lock.yaml") {
            "pnpm test"
        } else if has("yarn.lock") {
            "yarn test"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun run test"
        } else {
            "npm test"
        });
    }
    if has("Cargo.toml") {
        out.push("cargo test");
    }
    if has("go.mod") {
        out.push("go test");
    }
    if has("pytest.ini") || has("pyproject.toml") || has("setup.cfg") || has("tox.ini") {
        out.extend(["pytest", "python -m pytest"]);
    }
    if has("mix.exs") {
        out.push("mix test");
    }
    let make_test = std::fs::read_to_string(cwd.join("Makefile"))
        .is_ok_and(|t| t.lines().any(|l| l.starts_with("test:")));
    if make_test {
        out.push("make test");
    }
    out.into_iter().map(String::from).collect()
}

/// `nodal-mcp` next to the app's executable: `Contents/MacOS` in the bundle,
/// `target/<profile>` from source (only if it was built: `cargo build --bin nodal-mcp`).
pub fn nodal_mcp_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("nodal-mcp")).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests;
