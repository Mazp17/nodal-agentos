use nodal_host::paths::nodal_home;
use nodal_mcp_proto::paths::dev_sibling;

use super::DEV;

#[test]
fn dev_data_lives_next_to_the_release_data() {
    use std::path::Path;

    let dir = Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal");
    assert_eq!(
        dev_sibling(dir),
        Path::new("/Users/me/Library/Application Support/io.github.mazp17.nodal.dev")
    );
    // `cargo test` is a debug build unless run with --release.
    assert_eq!(nodal_home().unwrap().ends_with(".nodal-dev"), DEV);
}
