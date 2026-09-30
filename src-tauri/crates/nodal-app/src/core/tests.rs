//! P18: with `panic = "unwind"` in release, a panic inside `spawn_blocking` surfaces as a
//! `JoinError` `blocking` already converts to a plain error string (not a crash) — this only
//! actually exercises that path when the process's panic strategy is unwind.

use super::blocking;

#[test]
fn a_panic_inside_blocking_becomes_a_join_error_not_a_crash() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let res: Result<(), String> = rt.block_on(blocking(|| -> Result<(), String> { panic!("boom") }));
    let err = res.expect_err("a panicking closure must not be observed as Ok");
    assert!(err.contains("Internal error:"), "{err}");

    // The runtime (and the blocking pool) is still usable after that panic.
    assert_eq!(rt.block_on(blocking(|| Ok::<_, String>(2))).unwrap(), 2);
}
