// SPDX-License-Identifier: GPL-3.0-or-later
// Embeds the BabelCraft icon and version info into the Windows executable.
// On Linux and macOS the icon is set at runtime (window icon) and by the
// packaging files in `packaging/`.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" && std::path::Path::new("assets/icon.ico").exists() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "BabelCraft");
        res.set("FileDescription", "BabelCraft - English to Japanese video translator");
        res.set("LegalCopyright", "Copyright (C) 2026 BabelCraft contributors. GNU GPL v3 or later.");
        if let Err(e) = res.compile() {
            // Never fail the build just because the resource compiler is missing.
            println!("cargo:warning=Could not embed Windows icon: {e}");
        }
    }
}
