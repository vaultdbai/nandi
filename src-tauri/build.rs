fn main() {
    write_trial_key();

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

/// Writes `$OUT_DIR/trial_key.rs` (`services::llm_trial`): the free-trial
/// DeepSeek key from `NANDI_TRIAL_DEEPSEEK_KEY`, XORed with a pad made for
/// this build so the key never sits in the binary as plain text. Unset or
/// empty: both arrays are empty and the build has no trial.
fn write_trial_key() {
    use std::hash::{BuildHasher, Hasher};

    println!("cargo:rerun-if-env-changed=NANDI_TRIAL_DEEPSEEK_KEY");
    // Email sign-in client IDs, read with `option_env!` (services::email::provider)
    for name in [
        "NANDI_MICROSOFT_CLIENT_ID",
        "NANDI_GOOGLE_CLIENT_ID",
        "NANDI_GOOGLE_CLIENT_SECRET",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let key = std::env::var("NANDI_TRIAL_DEEPSEEK_KEY").unwrap_or_default();
    let key = key.trim().as_bytes();
    let random = std::collections::hash_map::RandomState::new();
    let pad: Vec<u8> = (0..key.len())
        .map(|i| {
            let mut hasher = random.build_hasher();
            hasher.write_usize(i);
            hasher.finish() as u8
        })
        .collect();
    let masked: Vec<u8> = key.iter().zip(&pad).map(|(k, p)| k ^ p).collect();
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("trial_key.rs");
    std::fs::write(
        out,
        format!(
            "const TRIAL_KEY_MASKED: &[u8] = &{masked:?};
const TRIAL_KEY_PAD: &[u8] = &{pad:?};
"
        ),
    )
    .expect("failed to write trial_key.rs");
}
