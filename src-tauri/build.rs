fn main() {
    let mut attributes = tauri_build::Attributes::new();
    // tauri-build embeds the Common Controls v6 manifest only in the app binary, so test
    // binaries die on Windows with STATUS_ENTRYPOINT_NOT_FOUND. Embed it at link time for
    // every target (app, nodal-mcp, tests) instead.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        attributes = attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = std::env::current_dir().unwrap().join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attributes).expect("tauri-build failed");
}
