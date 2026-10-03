use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_media_bridge();
        build_icloud_bridge();
    }
    tauri_build::build()
}

/// Compiles the MediaRemote bridge into a universal dylib that is shipped as
/// a bundle resource (see tauri.macos.conf.json). It has to exist before
/// `tauri_build::build()` validates the resource list.
fn build_media_bridge() {
    let source = "native/macos/media_bridge.m";
    let output = Path::new("resources/libbangs_media.dylib");
    println!("cargo:rerun-if-changed={source}");

    std::fs::create_dir_all("resources").expect("create resources dir");
    let status = Command::new("xcrun")
        .args([
            "clang",
            "-dynamiclib",
            "-fobjc-arc",
            "-O2",
            "-arch",
            "arm64",
            "-arch",
            "x86_64",
            "-mmacosx-version-min=11.0",
            "-framework",
            "Foundation",
            "-framework",
            "AppKit",
            source,
            "-o",
        ])
        .arg(output)
        .status()
        .expect("failed to run clang for the media bridge");
    assert!(status.success(), "failed to compile {source}");
    sign(output);
}

/// Compiles the iCloud bridge for the watch (native/macos/icloud_bridge.m)
/// into a static library linked into the app itself: CloudKit checks the
/// entitlements of the process that calls it, which has to be Bangs.
fn build_icloud_bridge() {
    let source = "native/macos/icloud_bridge.m";
    println!("cargo:rerun-if-changed={source}");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("icloud_bridge.o");
    let library = out.join("libbangs_icloud.a");
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        _ => "x86_64",
    };
    let status = Command::new("xcrun")
        .args(["clang", "-c", "-fobjc-arc", "-O2", "-arch", arch, "-mmacosx-version-min=12.0", source, "-o"])
        .arg(&object)
        .status()
        .expect("failed to run clang for the iCloud bridge");
    assert!(status.success(), "failed to compile {source}");
    let status = Command::new("xcrun")
        .args(["libtool", "-static", "-o"])
        .arg(&library)
        .arg(&object)
        .status()
        .expect("failed to run libtool for the iCloud bridge");
    assert!(status.success(), "failed to archive {source}");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=bangs_icloud");
    println!("cargo:rustc-link-lib=framework=CloudKit");
    println!("cargo:rustc-link-lib=framework=Security");
    println!("cargo:rustc-link-lib=framework=Foundation");
}

/// A release build signs this dylib here, with the same identity the bundle
/// is signed with: it ships inside the app, and notarization refuses a bundle
/// that contains anything only ad-hoc signed.
fn sign(library: &Path) {
    println!("cargo:rerun-if-env-changed=APPLE_SIGNING_IDENTITY");
    let Ok(identity) = std::env::var("APPLE_SIGNING_IDENTITY") else {
        return;
    };
    let status = Command::new("codesign")
        .args(["--force", "--timestamp", "--options", "runtime", "--sign", &identity])
        .arg(library)
        .status()
        .expect("failed to run codesign for the media bridge");
    assert!(status.success(), "failed to sign {}", library.display());
}
