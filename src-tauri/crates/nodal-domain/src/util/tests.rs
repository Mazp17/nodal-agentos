use super::*;

#[test]
fn ids_are_unique_sortable_and_path_safe() {
    let a = new_id('t', 1_700_000_000_000);
    let b = new_id('t', 1_700_000_000_000);
    let c = new_id('t', 1_700_000_000_001);
    assert_ne!(a, b);
    assert!(a < c && b < c, "{a} {b} {c}");
    assert_eq!(a.len(), 19);
    assert!(is_valid_id(&a), "{a}");
    assert!(new_id('r', 1).starts_with('r'));
    assert!(is_valid_id("legacy-task_1"));
    assert!(!is_valid_id("../etc"));
    assert!(!is_valid_id("t/../x"));
    assert!(!is_valid_id("TABC"));
    assert!(!is_valid_id("-x"));
    assert!(!is_valid_id(""));
}

#[test]
fn clip() {
    assert_eq!(clip_chars("hello", 10), "hello");
    assert_eq!(clip_chars("naïveté", 4), "naï…");
}
