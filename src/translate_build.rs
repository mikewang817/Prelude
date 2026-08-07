//! Building the Apple translation helper.
//!
//! Apple's `Translation` framework is only vended through a SwiftUI view
//! modifier, and it refuses to serve a bare CLI binary — the XPC call to
//! `translationd` hangs forever with no error. It works as soon as the caller
//! has real app-bundle identity, so this compiles the Swift helper and wraps
//! it in a minimal ad-hoc-signed .app. No developer account needed.
//!
//! The Swift source is embedded in this binary rather than read from disk, so
//! a single distributed executable can rebuild the helper anywhere.

use crate::ansi::*;
use crate::exec::which;
use crate::paths;
use std::time::Duration;

const SWIFT_SRC: &str = include_str!("../translate.swift");

const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>PreludeTranslate</string>
<key>CFBundleIdentifier</key><string>local.prelude.translate</string>
<key>CFBundleName</key><string>PreludeTranslate</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>LSMinimumSystemVersion</key><string>15.0</string>
<key>LSUIElement</key><true/>
</dict></plist>
"#;

pub fn build() -> i32 {
    if which("swiftc").is_none() {
        eprintln!("prelude: swiftc not found — install Xcode or the Command Line Tools");
        return 2;
    }
    let app = paths::data().join("PreludeTranslate.app");
    let macos = app.join("Contents/MacOS");
    println!("building {} ...", app.display());
    if std::fs::create_dir_all(&macos).is_err() {
        eprintln!("prelude: could not create {}", macos.display());
        return 2;
    }

    let src = std::env::temp_dir().join("prelude-translate.swift");
    if std::fs::write(&src, SWIFT_SRC).is_err() {
        eprintln!("prelude: could not write {}", src.display());
        return 2;
    }
    let bin = macos.join("PreludeTranslate");
    let out = std::process::Command::new("swiftc")
        .args(["-O", &src.to_string_lossy(), "-o", &bin.to_string_lossy()])
        .output();
    let _ = std::fs::remove_file(&src);
    match out {
        Ok(o) if !o.status.success() => {
            eprintln!("prelude: swiftc failed:\n{}", String::from_utf8_lossy(&o.stderr));
            return 2;
        }
        Err(e) => {
            eprintln!("prelude: swiftc failed: {e}");
            return 2;
        }
        _ => {}
    }

    if std::fs::write(app.join("Contents/Info.plist"), PLIST).is_err() {
        eprintln!("prelude: could not write Info.plist");
        return 2;
    }
    crate::exec::run(
        &["codesign", "--force", "--sign", "-", &app.to_string_lossy()],
        Duration::from_secs(120),
    );

    match crate::compute::translate("hello", "zh-Hans") {
        Ok(v) => {
            println!("  {GREEN}✓{RESET} built and working  (hello -> {v})");
            0
        }
        Err(e) => {
            println!("  built, but a test translation failed: {e}");
            1
        }
    }
}
