fn main() {
    // Compile-time only. No deployed binary inspects Git or source directories.
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
    };
    let revision = git(&["rev-parse", "HEAD"])
        .map(|s| s.trim().to_owned())
        .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=normal"])
        .map(|s| if s.is_empty() { "false" } else { "true" })
        .unwrap_or("unknown");
    println!("cargo:rustc-env=FLUXDL_CODE_REVISION={revision}");
    println!("cargo:rustc-env=FLUXDL_CODE_DIRTY={dirty}");
    for file in [
        ".git/HEAD",
        ".git/index",
        ".git/refs/heads/main",
        "build.rs",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            if file == "build.rs" {
                std::path::PathBuf::from(file)
            } else {
                root.join(file)
            }
            .display()
        );
    }
    if let Some(files) = git(&["ls-files", "-z"]) {
        for file in files.split('\0').filter(|s| !s.is_empty()) {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
    }
    // Tauri normally puts MANIFEST resource #1 in the main binary's resource.lib.
    // Supply it once through the linker instead so examples and unit tests get
    // the same Common-Controls v6 dependency without duplicate resource #1.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("Tauri build metadata generation failed");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("windows.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
