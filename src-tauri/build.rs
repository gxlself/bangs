use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_media_bridge();
        if std::env::var_os("CARGO_FEATURE_ICLOUD").is_some() {
            link_cloud_bridge();
        }
    }
    tauri_build::build()
}

/// Builds packages/BangsCloud with SwiftPM and links it in: the CloudKit side
/// of syncing with the iPhone app (docs/sync.md). Only with `--features icloud`,
/// so everyone else needs neither a Swift toolchain nor the iCloud signing.
fn link_cloud_bridge() {
    println!("cargo:rerun-if-changed=../packages/BangsCloud/Package.swift");
    println!("cargo:rerun-if-changed=../packages/BangsCloud/Sources");
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("no Swift build for the architecture {other:?}"),
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let scratch = out.join("swift");
    let status = Command::new("swift")
        .args(["build", "-c", "release", "--product", "BangsCloudBridge", "--triple"])
        .arg(format!("{arch}-apple-macosx12.0"))
        .arg("--package-path")
        .arg("../packages/BangsCloud")
        .arg("--scratch-path")
        .arg(&scratch)
        .status()
        .expect("failed to run `swift build`; the icloud feature needs Xcode or its command line tools");
    assert!(status.success(), "swift build failed for packages/BangsCloud");

    // One archive of every object SwiftPM made, whatever it decides a static
    // product contains: the package's own targets are all that is needed.
    let mut objects = Vec::new();
    collect_objects(&scratch, false, &mut objects);
    assert!(!objects.is_empty(), "swift build produced no objects under {}", scratch.display());
    let library = out.join("libbangs_cloud.a");
    let status = Command::new("xcrun")
        .args(["libtool", "-static", "-o"])
        .arg(&library)
        .args(&objects)
        .status()
        .expect("failed to run libtool");
    assert!(status.success(), "failed to archive the Swift objects");

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=bangs_cloud");
    for framework in ["CloudKit", "Foundation", "Security"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    // The Swift runtime ships with the OS; the linker still has to find the
    // stubs and the back-deployment libraries the objects ask for.
    for dir in swift_library_dirs() {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

/// Every `.o` inside a `*.build` directory. Symlinks are not followed, so the
/// `release` alias SwiftPM leaves next to the real directory is not counted twice.
fn collect_objects(dir: &Path, inside_build: bool, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            let is_build = path.extension().is_some_and(|ext| ext == "build");
            collect_objects(&path, inside_build || is_build, found);
        } else if kind.is_file() && inside_build && path.extension().is_some_and(|ext| ext == "o") {
            found.push(path);
        }
    }
}

fn swift_library_dirs() -> Vec<String> {
    let mut dirs = vec!["/usr/lib/swift".to_string()];
    if let Some(sdk) = command_output("xcrun", &["--show-sdk-path"]) {
        dirs.push(format!("{sdk}/usr/lib/swift"));
    }
    if let Some(developer) = command_output("xcode-select", &["-p"]) {
        for candidate in [
            format!("{developer}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx"),
            format!("{developer}/usr/lib/swift/macosx"),
        ] {
            if Path::new(&candidate).exists() {
                dirs.push(candidate);
            }
        }
    }
    dirs
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
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
