use std::path::PathBuf;

fn main() {
    // On Windows tauri-build embeds the app manifest (Common Controls v6) only
    // into the app binary, so `cargo test` binaries crash with
    // STATUS_ENTRYPOINT_NOT_FOUND. Embed our own manifest into every target.
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut windows = tauri_build::WindowsAttributes::new();
    if msvc {
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
        embed_manifest();
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .unwrap_or_else(|e| panic!("tauri-build failed: {e:#}"));
}

/// Worktrees share one target dir and the package hash does not depend on the
/// checkout path, so a cached build-script run may outlive the worktree it ran
/// in. Hence: the source path is read at run time (not `env!`), the rerun hint
/// is relative to the package, and the linker gets a copy in `OUT_DIR`, which
/// always exists as long as the cached link args are in use.
fn embed_manifest() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map(PathBuf::from);
    let out_dir = std::env::var("OUT_DIR").map(PathBuf::from);
    let (Ok(manifest_dir), Ok(out_dir)) = (manifest_dir, out_dir) else {
        panic!("CARGO_MANIFEST_DIR / OUT_DIR not set by cargo");
    };
    let src = manifest_dir.join("windows-app-manifest.xml");
    let dst = out_dir.join("windows-app-manifest.xml");
    std::fs::copy(&src, &dst)
        .unwrap_or_else(|e| panic!("copy {} -> {}: {e}", src.display(), dst.display()));
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", dst.display());
}
