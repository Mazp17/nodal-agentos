use super::*;
use crate::testutil::TempDir;

#[test]
fn detects_test_commands() {
    let t = TempDir::new("test-cmds");
    assert!(test_commands(&t.0).is_empty());
    std::fs::write(t.0.join("package.json"), r#"{"scripts":{"test":"vitest"}}"#).unwrap();
    std::fs::write(t.0.join("pnpm-lock.yaml"), "").unwrap();
    std::fs::write(t.0.join("Cargo.toml"), "").unwrap();
    std::fs::write(t.0.join("Makefile"), "build:\n\tx\ntest:\n\ty\n").unwrap();
    assert_eq!(
        test_commands(&t.0),
        ["pnpm test", "cargo test", "make test"]
    );
    std::fs::write(t.0.join("package.json"), r#"{"scripts":{"build":"x"}}"#).unwrap();
    assert_eq!(test_commands(&t.0), ["cargo test", "make test"]);
}
