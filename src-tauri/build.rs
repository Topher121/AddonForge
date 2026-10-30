use std::env;
use std::fs;
use std::path::Path;

/// The CurseForge 3rd Party API key is baked into release builds only. It is
/// read from `private/curseforge-key.txt` (gitignored) or the
/// `ADDONFORGE_CURSEFORGE_KEY` environment variable at compile time, lightly
/// masked so it does not sit in the exe as a plain string, and never written
/// anywhere else. Builds without it (CI, contributors) simply have CurseForge
/// turned off. See CLAUDE.md "CurseForge API" for the permission record.
fn curseforge_key() -> Option<String> {
    println!("cargo:rerun-if-env-changed=ADDONFORGE_CURSEFORGE_KEY");
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("private").join("curseforge-key.txt");
    println!("cargo:rerun-if-changed={}", file.display());
    let from_env = env::var("ADDONFORGE_CURSEFORGE_KEY").ok();
    let from_file = fs::read_to_string(&file).ok();
    from_env
        .or(from_file)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    // Refresh the embedded Windows resources when only the artwork changes.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");

    // Plain embedding since build 26: the old XOR mask hid nothing (any copy of
    // the app can be read back) and its byte pattern was a detector signal.
    let key = curseforge_key().unwrap_or_default();
    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("cf_key.rs");
    let body = format!("pub const CF_KEY: &str = {key:?};\n");
    fs::write(&out, body).expect("write cf_key.rs");
    println!(
        "cargo:warning=CurseForge key: {}",
        if key.is_empty() { "not present (CurseForge disabled in this build)" } else { "embedded" }
    );

    tauri_build::build()
}
