use super::*;

#[test]
fn path_ids() {
    assert!(is_valid_path_id("wf_2450b7a8-254"));
    assert!(is_valid_path_id("a1007a0f03db17270"));
    for bad in ["", "..", "a/b", "a\\b", "-rf", "wf_x.json", "a b"] {
        assert!(!is_valid_path_id(bad), "{bad}");
    }
}
