use super::*;

#[test]
fn write_plan_only_when_changed() {
    let dir = std::env::temp_dir().join(format!("nodal-plan-{}", crate::util::new_id('x', 1)));
    let path = plan_path(&dir, "t1");
    assert!(write_plan(&path, "a").unwrap());
    assert!(!write_plan(&path, "a").unwrap());
    assert!(write_plan(&path, "b").unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
    std::fs::remove_dir_all(dir).unwrap();
}
