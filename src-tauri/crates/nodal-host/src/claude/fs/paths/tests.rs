use super::*;
use nodal_domain::sessions::transcript::is_valid_session_id;

const SANDBOX: &str = "/Users/me/Code/nodal-sandbox";
const DONE_SESSION: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";

fn fixtures_projects() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/claude/fixtures/projects")
}

#[test]
fn slug_matches_real_project_dirs() {
    assert_eq!(project_slug(SANDBOX), "-Users-me-Code-nodal-sandbox");
    assert_eq!(
        project_slug("/Users/x/Code/repo/.claude/worktrees/acme-38"),
        "-Users-x-Code-repo--claude-worktrees-acme-38"
    );
    assert!(is_valid_session_id(DONE_SESSION));
    assert!(!is_valid_session_id("../etc"));
    assert!(!is_valid_session_id(""));
}

#[test]
fn finds_session_dir_by_slug_and_by_scan() {
    let projects = fixtures_projects();
    let dir = find_session_dir(&projects, SANDBOX, DONE_SESSION).unwrap();
    assert!(dir.ends_with(DONE_SESSION));
    // cwd that doesn't match the slug: falls back to the scan.
    assert_eq!(find_session_dir(&projects, "/somewhere/else", DONE_SESSION), Some(dir));
    assert_eq!(find_session_dir(&projects, SANDBOX, "no-such-session"), None);
}
