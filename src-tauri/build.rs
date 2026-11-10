fn main() {
    watch_build_env();

    // Windows: embed the common-controls v6 manifest via the linker for
    // *every* target, not just the app binary. tauri-build's default only
    // covers the bin, so test binaries load comctl32 v5 and crash at startup
    // with STATUS_ENTRYPOINT_NOT_FOUND (TaskDialogIndirect is v6-only).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let embed_manifest = target_os == "windows" && target_env == "msvc";

    let mut attributes = tauri_build::Attributes::new();
    if embed_manifest {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    }
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

/// Rebuilds when a client ID read with `option_env!` changes (email
/// sign-in, `services::email::provider`)
fn watch_build_env() {
    for name in [
        "NANDI_MICROSOFT_CLIENT_ID",
        "NANDI_GOOGLE_CLIENT_ID",
        "NANDI_GOOGLE_CLIENT_SECRET",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
}
