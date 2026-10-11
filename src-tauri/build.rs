fn main() {
    // The commit the app is built from, for Settings and About: `QUADCAM_BUILD` when set,
    // else `git rev-parse --short HEAD`, else "unknown".
    println!("cargo:rerun-if-env-changed=QUADCAM_BUILD");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let build = std::env::var("QUADCAM_BUILD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| git(&["rev-parse", "--short=9", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    // Build again when HEAD moves (a commit, a checkout).
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/logs/HEAD");
    }
    println!("cargo:rustc-env=QUADCAM_BUILD={build}");
    intel();
    tauri_build::build()
}

/// The Swift helper `quadcam-intel` (`../intel`), which `tauri.conf.json` bundles as the
/// external binary `binaries/quadcam-intel`: tauri-build copies it beside the app's binaries
/// (`target/<profile>/`) and the bundler into `Contents/MacOS`. It needs Xcode 26 or newer.
/// Without one, a stand-in script that answers "unavailable" takes its place, so the app
/// builds and reports the speech check as unavailable; `QUADCAM_INTEL_REQUIRED=1` (CI and
/// releases) makes that a build error instead.
fn intel() {
    use std::path::{Path, PathBuf};
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let package = manifest.join("../intel");
    println!("cargo:rerun-if-changed=../intel/Package.swift");
    println!("cargo:rerun-if-changed=../intel/Sources");
    println!("cargo:rerun-if-env-changed=QUADCAM_INTEL_REQUIRED");
    let triple = std::env::var("TARGET").unwrap();
    let dest = manifest
        .join("binaries")
        .join(format!("quadcam-intel-{triple}"));
    // target/<profile>/build/<crate>/out, with an optional <triple> level: two levels above
    // `build` is the target folder, shared by every profile.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target = out
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest.join("target"));
    let scratch = target.join("intel-swift");
    let arch = if triple.starts_with("aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let swift = |extra: &[&str]| {
        std::process::Command::new("xcrun")
            .args(["swift", "build", "-c", "release", "--arch", arch])
            .arg("--package-path")
            .arg(&package)
            .arg("--scratch-path")
            .arg(&scratch)
            .args(extra)
            // Cargo's own variables mean nothing to SwiftPM; the package names its macOS floor.
            .env_remove("MACOSX_DEPLOYMENT_TARGET")
            .output()
    };
    let built = swift(&[]);
    let product = swift(&["--show-bin-path"])
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()).join("quadcam-intel"))
        .unwrap_or_default();
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    match built {
        Ok(o) if o.status.success() && product.is_file() => {
            let _ = std::fs::remove_file(&dest);
            std::fs::copy(&product, &dest).expect("copying quadcam-intel");
        }
        other => {
            let why = match other {
                Ok(o) => String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .filter(|l| l.contains("error"))
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("; "),
                Err(e) => e.to_string(),
            };
            if std::env::var("QUADCAM_INTEL_REQUIRED").as_deref() == Ok("1") {
                panic!("quadcam-intel did not build (it needs Xcode 26 or newer): {why}");
            }
            println!("cargo:warning=quadcam-intel did not build (it needs Xcode 26 or newer), so the speech check is unavailable in this build: {why}");
            std::fs::write(&dest, STAND_IN).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// What a build without Xcode 26 bundles in the helper's place.
const STAND_IN: &str = "#!/bin/sh\necho '{\"kind\":\"error\",\"code\":\"unavailable\",\"message\":\"This build of QuadCam has no speech check.\"}'\nexit 3\n";
