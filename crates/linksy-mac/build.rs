//! Build `Linksy.app` from `Sources/*.swift` with the system's `swiftc` (Xcode's command line
//! tools are enough: no Xcode, no Homebrew), into `target/<profile>/Linksy.app`. On other
//! systems there is nothing to build.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Info.plist");
    println!("cargo:rerun-if-changed=Sources");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        println!("cargo:warning=linksy-mac is the macOS menu bar app; nothing to build here");
        return;
    }
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut sources: Vec<PathBuf> = std::fs::read_dir(crate_dir.join("Sources"))
        .expect("Sources/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "swift"))
        .collect();
    sources.sort();
    for s in &sources {
        println!("cargo:rerun-if-changed={}", s.display());
    }

    // target/<profile>/Linksy.app: the bundle next to the other binaries, where the install
    // script expects it (OUT_DIR is target/<profile>/build/linksy-mac-<hash>/out).
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let profile_dir = out_dir.ancestors().nth(3).expect("target/<profile>").to_path_buf();
    let app = profile_dir.join("Linksy.app");
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    let _ = std::fs::remove_dir_all(&app);
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::create_dir_all(&resources).unwrap();

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let arch = match arch.as_str() {
        "aarch64" => "arm64",
        other => other,
    };
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let mut swiftc = Command::new("swiftc");
    swiftc
        .arg(if release { "-O" } else { "-Onone" })
        .args(["-swift-version", "5", "-target", &format!("{arch}-apple-macos13.0"), "-module-name", "Linksy"])
        .arg("-o")
        .arg(macos.join("linksy-mac"))
        .args(&sources);
    let status = swiftc.status().expect("swiftc (install Xcode's command line tools: xcode-select --install)");
    assert!(status.success(), "swiftc failed");

    let plist = std::fs::read_to_string(crate_dir.join("Info.plist")).unwrap().replace("@VERSION@", &std::env::var("CARGO_PKG_VERSION").unwrap());
    std::fs::write(contents.join("Info.plist"), plist).unwrap();
    std::fs::write(contents.join("PkgInfo"), "APPL????").unwrap();
    icon(&crate_dir.join("../../brand/logo/logo.png"), &resources, &out_dir);

    // An ad-hoc signature: macOS likes bundles to carry one (login items, Notification Center)
    let signed = Command::new("codesign").args(["--force", "--sign", "-"]).arg(&app).status().is_ok_and(|s| s.success());
    if !signed {
        println!("cargo:warning=codesign failed: Linksy.app is unsigned");
    }
}

/// The app's icon from the brand's PNG, with the system's own tools; skipped when they're
/// missing (the Finder then shows a generic icon).
fn icon(png: &Path, resources: &Path, scratch: &Path) {
    println!("cargo:rerun-if-changed={}", png.display());
    if !png.exists() {
        return;
    }
    let set = scratch.join("Linksy.iconset");
    let _ = std::fs::remove_dir_all(&set);
    std::fs::create_dir_all(&set).unwrap();
    for (name, px) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128), ("128x128@2x", 256), ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
        let ok = Command::new("sips").args(["-z", &px.to_string(), &px.to_string()]).arg(png).arg("--out").arg(set.join(format!("icon_{name}.png"))).stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !ok {
            return;
        }
    }
    let _ = Command::new("iconutil").args(["-c", "icns", "-o"]).arg(resources.join("Linksy.icns")).arg(&set).status();
}
