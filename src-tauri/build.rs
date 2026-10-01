fn main() {
    // `cfg(windows)` inside a build script describes the HOST, not the target,
    // so the manifest has to be attached based on CARGO_CFG_TARGET_OS.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let mut attributes = tauri_build::Attributes::new();
    if target_os == "windows" {
        // Replaces Tauri's default manifest, so it must keep the
        // Common-Controls dependency that the default provides — without it
        // native dialogs lose their v6 theming.
        attributes = attributes.windows_attributes(
            tauri_build::WindowsAttributes::new()
                .app_manifest(include_str!("windows-app.manifest")),
        );
    }

    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
