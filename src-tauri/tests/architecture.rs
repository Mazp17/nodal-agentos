//! Enforces the workspace's per-crate dependency whitelist (plan §2) and the absence of
//! internal dependency cycles, straight from `cargo metadata`, so a stray `use` or
//! manifest edit that widens a crate's dependency surface fails CI instead of drifting.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;

const WORKSPACE_CRATES: &[&str] = &[
    "nodal",
    "nodal-app",
    "nodal-domain",
    "nodal-host",
    "nodal-linear",
    "nodal-mcp-proto",
    "nodal-store",
];

/// Per-crate whitelist of normal (non-dev, non-build) dependencies: `(crate, internal, external)`.
const WHITELIST: &[(&str, &[&str], &[&str])] = &[
    ("nodal-domain", &[], &["serde", "serde_json", "thiserror"]),
    ("nodal-mcp-proto", &[], &["serde", "serde_json"]),
    (
        "nodal-store",
        &["nodal-domain"],
        &["rusqlite", "serde", "serde_json", "thiserror", "tokio"],
    ),
    (
        "nodal-host",
        &["nodal-domain"],
        &[
            "tokio",
            "keyring",
            "portable-pty",
            "libc",
            "serde",
            "serde_json",
            "thiserror",
        ],
    ),
    (
        "nodal-linear",
        &["nodal-domain"],
        &["reqwest", "serde", "serde_json", "thiserror"],
    ),
    (
        "nodal-app",
        &["nodal-domain", "nodal-store", "nodal-mcp-proto"],
        &["tokio", "serde", "serde_json", "thiserror"],
    ),
    (
        "nodal",
        &[
            "nodal-domain",
            "nodal-store",
            "nodal-host",
            "nodal-linear",
            "nodal-mcp-proto",
            "nodal-app",
        ],
        &[
            "tauri",
            "tauri-plugin-opener",
            "tauri-plugin-dialog",
            "tauri-plugin-updater",
            "tauri-plugin-notification",
            "serde",
            "serde_json",
            "tokio",
        ],
    ),
];

/// External crates confined to the workspace crate(s) that own them.
const EXCLUSIVE_EXTERNAL: &[(&str, &[&str])] = &[
    ("tauri", &["nodal"]),
    ("rusqlite", &["nodal-store"]),
    ("reqwest", &["nodal-linear"]),
    ("keyring", &["nodal-host"]),
    ("portable-pty", &["nodal-host"]),
];

struct Dep {
    name: String,
    kind: Option<String>,
    is_internal: bool,
}

fn cargo_metadata() -> Value {
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            manifest_path,
        ])
        .output()
        .expect("failed to run `cargo metadata`");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata did not print valid JSON")
}

fn packages(metadata: &Value) -> BTreeMap<String, Vec<Dep>> {
    let mut out = BTreeMap::new();
    for pkg in metadata["packages"].as_array().expect("packages array") {
        let name = pkg["name"].as_str().expect("package name").to_string();
        let deps = pkg["dependencies"]
            .as_array()
            .expect("dependencies array")
            .iter()
            .map(|dep| {
                let dep_name = dep["name"].as_str().expect("dep name").to_string();
                let kind = dep["kind"].as_str().map(str::to_string);
                let is_internal = WORKSPACE_CRATES.contains(&dep_name.as_str());
                Dep {
                    name: dep_name,
                    kind,
                    is_internal,
                }
            })
            .collect();
        out.insert(name, deps);
    }
    out
}

#[test]
fn workspace_matches_dependency_whitelist() {
    let metadata = cargo_metadata();
    let packages = packages(&metadata);

    let actual_members: BTreeSet<&str> = packages.keys().map(String::as_str).collect();
    let expected_members: BTreeSet<&str> = WORKSPACE_CRATES.iter().copied().collect();
    assert_eq!(
        actual_members, expected_members,
        "workspace membership changed; update WORKSPACE_CRATES and the whitelist in this test"
    );

    for (crate_name, internal, external) in WHITELIST {
        let deps = &packages[*crate_name];

        let expected: BTreeSet<&str> = internal.iter().chain(external.iter()).copied().collect();
        let actual: BTreeSet<&str> = deps
            .iter()
            .filter(|d| d.kind.is_none())
            .map(|d| d.name.as_str())
            .collect();
        assert_eq!(
            actual, expected,
            "{crate_name}'s normal dependencies drifted from the §2 whitelist (left = actual, right = expected)"
        );

        for dep in deps.iter().filter(|d| d.kind.is_none()) {
            let should_be_internal = internal.contains(&dep.name.as_str());
            assert_eq!(
                dep.is_internal, should_be_internal,
                "{crate_name} -> {} internal/external classification mismatch",
                dep.name
            );
        }
    }
}

#[test]
fn exclusive_external_deps_stay_confined() {
    let metadata = cargo_metadata();
    let packages = packages(&metadata);

    for (dep_name, owners) in EXCLUSIVE_EXTERNAL {
        for (crate_name, deps) in &packages {
            let depends_on_it = deps.iter().any(|d| d.kind.is_none() && d.name == *dep_name);
            assert!(
                !depends_on_it || owners.contains(&crate_name.as_str()),
                "{crate_name} depends on {dep_name} directly, but only {owners:?} may"
            );
        }
    }
}

#[test]
fn no_internal_dependency_cycles() {
    let metadata = cargo_metadata();
    let packages = packages(&metadata);

    // Normal and dev edges both count: Cargo already forbids a normal-dependency
    // cycle, but a dev-dependency cycle (e.g. a test-support feature) compiles fine
    // and would still be an architecture violation.
    let mut edges: BTreeMap<&str, BTreeSet<&str>> = WORKSPACE_CRATES
        .iter()
        .map(|&c| (c, BTreeSet::new()))
        .collect();
    for (crate_name, deps) in &packages {
        for dep in deps {
            if dep.is_internal && dep.kind.as_deref() != Some("build") {
                edges
                    .get_mut(crate_name.as_str())
                    .unwrap()
                    .insert(dep.name.as_str());
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    fn visit<'a>(
        node: &'a str,
        edges: &BTreeMap<&'a str, BTreeSet<&'a str>>,
        colors: &mut BTreeMap<&'a str, Color>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<&'a str>> {
        colors.insert(node, Color::Gray);
        stack.push(node);
        for &next in &edges[node] {
            match colors.get(next).copied().unwrap_or(Color::White) {
                Color::White => {
                    if let Some(cycle) = visit(next, edges, colors, stack) {
                        return Some(cycle);
                    }
                }
                Color::Gray => {
                    let start = stack.iter().position(|&n| n == next).unwrap();
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(next);
                    return Some(cycle);
                }
                Color::Black => {}
            }
        }
        stack.pop();
        colors.insert(node, Color::Black);
        None
    }

    let mut colors: BTreeMap<&str, Color> = BTreeMap::new();
    for &crate_name in WORKSPACE_CRATES {
        if colors.get(crate_name).copied().unwrap_or(Color::White) == Color::White {
            let mut stack = Vec::new();
            if let Some(cycle) = visit(crate_name, &edges, &mut colors, &mut stack) {
                panic!("internal dependency cycle detected: {}", cycle.join(" -> "));
            }
        }
    }
}
