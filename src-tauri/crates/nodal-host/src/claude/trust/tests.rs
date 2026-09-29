use super::*;
use crate::testutil::{git_available, init_repo, TempDir};
use serde_json::json;

fn cfg(entries: &[(&str, bool)]) -> Value {
    let projects: serde_json::Map<String, Value> = entries
        .iter()
        .map(|(k, v)| (k.to_string(), json!({"hasTrustDialogAccepted": v})))
        .collect();
    json!({ "projects": projects })
}

#[test]
fn rule_exact_parent_and_bound() {
    let repo = Path::new("/w/acme");
    let sub = Path::new("/w/acme/packages/web");
    // Canonical root.
    let t = trust_of(&cfg(&[("/w/acme", true)]), sub, repo, Some(repo));
    assert_eq!(
        (t.trusted, t.source, t.matched_path.as_deref()),
        (Some(true), TrustSource::Repo, Some("/w/acme"))
    );
    // Parent folder inside the repo.
    let t = trust_of(&cfg(&[("/w/acme/packages", true)]), sub, repo, Some(repo));
    assert_eq!((t.trusted, t.source), (Some(true), TrustSource::Parent));
    // A parent OUTSIDE the repo doesn't count; outside git, it does.
    let t = trust_of(&cfg(&[("/w", true)]), sub, repo, Some(repo));
    assert_eq!(
        (t.trusted, t.source),
        (Some(false), TrustSource::NotTrusted)
    );
    let t = trust_of(&cfg(&[("/w", true)]), sub, sub, None);
    assert_eq!(t.matched_path.as_deref(), Some("/w"));
    // Explicit `false` or missing: not trusted.
    let t = trust_of(&cfg(&[("/w/acme", false)]), repo, repo, Some(repo));
    assert_eq!(t.trusted, Some(false));
    assert_eq!(
        trust_of(&json!({}), repo, repo, Some(repo)).trusted,
        Some(false)
    );
    // Worktree: the canonical root is the main repo even though it's outside the bound.
    let wt = Path::new("/wt/acme/pay-1");
    let t = trust_of(&cfg(&[("/w/acme", true)]), wt, repo, Some(wt));
    assert_eq!(t.source, TrustSource::Repo);
}

#[test]
fn reads_config_and_resolves_worktrees() {
    if !git_available() {
        eprintln!("git not available: skipping");
        return;
    }
    let t = TempDir::new("trust");
    let repo = t.0.join("repo");
    init_repo(&repo);
    let wt = t.0.join("wt");
    git::ok(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "nodal/x",
            wt.to_str().unwrap(),
        ],
    )
    .unwrap();
    let file = t.0.join(".claude.json");
    std::fs::write(&file, cfg(&[(repo.to_str().unwrap(), true)]).to_string()).unwrap();

    let r = repo_trust_blocking(&wt, Some(&file)).unwrap();
    assert_eq!((r.trusted, r.source), (Some(true), TrustSource::Repo));
    let sub = repo.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    assert_eq!(
        repo_trust_blocking(&sub, Some(&file)).unwrap().trusted,
        Some(true)
    );

    std::fs::write(&file, "{}").unwrap();
    assert_eq!(
        repo_trust_blocking(&repo, Some(&file)).unwrap().trusted,
        Some(false)
    );
    std::fs::write(&file, "not json").unwrap();
    assert_eq!(
        repo_trust_blocking(&repo, Some(&file)).unwrap().source,
        TrustSource::Unknown
    );
    assert_eq!(
        repo_trust_blocking(&repo, Some(&t.0.join("missing.json")))
            .unwrap()
            .trusted,
        None
    );
    assert!(repo_trust_blocking(Path::new("relative"), Some(&file)).is_err());

    // Symlink to the folder: resolved to compare with what git returns.
    let link = t.0.join("link");
    std::os::unix::fs::symlink(&repo, &link).unwrap();
    std::fs::write(
        &file,
        cfg(&[(repo.join("src").to_str().unwrap(), true)]).to_string(),
    )
    .unwrap();
    let r = repo_trust_blocking(&link.join("src"), Some(&file)).unwrap();
    assert_eq!((r.trusted, r.source), (Some(true), TrustSource::Parent));
    // And an entry saved with the unresolved path counts too.
    std::fs::write(&file, cfg(&[(link.to_str().unwrap(), true)]).to_string()).unwrap();
    assert_eq!(
        repo_trust_blocking(&link.join("src"), Some(&file))
            .unwrap()
            .trusted,
        Some(true)
    );
}
