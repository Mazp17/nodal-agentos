use super::*;

#[test]
fn slugs() {
    assert_eq!(
        task_slug("PAY", 1, "New logo in the header!"),
        "pay-1-new-logo-in-the-header"
    );
    assert_eq!(
        task_slug("WEB", 12, "Crème brûlée for the piñata"),
        "web-12-creme-brulee-for-the-pinata"
    );
    assert_eq!(task_slug("A1", 3, "  ¿¿??  "), "a1-3");
    let long = task_slug("PAY", 1, &"word ".repeat(20));
    assert!(long.len() <= SLUG_MAX, "{long}");
    assert!(!long.ends_with('-'));
    assert_eq!(branch_for("pay-1-x"), "nodal/pay-1-x");
    assert_eq!(
        dir_for(Path::new("/w"), "My Repo", "pay-1"),
        PathBuf::from("/w/my-repo/pay-1")
    );
}

#[test]
fn run_id_validation() {
    assert!(is_valid_run_id("ddb91222"));
    assert!(is_valid_run_id("abc-123"));
    assert!(!is_valid_run_id("-rf"));
    assert!(!is_valid_run_id("ab"));
    assert!(!is_valid_run_id("abcd\"; rm -rf ~"));
    assert!(!is_valid_run_id("abcd efgh"));
    assert!(!is_valid_run_id("abcd'"));
}
