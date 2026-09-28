//! Helpers moved to `nodal_host::testutil`; re-exported so current uses don't break.

pub use nodal_host::testutil::*;

#[test]
fn dev_data_lives_next_to_the_release_data() {
    use std::path::Path;

    let dir = Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal");
    assert_eq!(
        super::dev_sibling(dir),
        Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal.dev")
    );
    // `cargo test` is a debug build unless run with --release.
    assert_eq!(super::nodal_home().unwrap().ends_with(".nodal-dev"), super::DEV);
}
