//! The programs QuadCam runs: a macOS system tool by its full path, or a tool the module
//! manager provides (ffmpeg, ffprobe, esptool) or the person set. Nothing comes from the
//! user's PATH or from Homebrew except the ffmpeg fallback and optional exiftool
//! (`docs/modules.md`, "What QuadCam needs"). A new program fails this test until it is
//! added here and in that table.

use std::path::{Path, PathBuf};

/// macOS's own tools, by the path QuadCam calls. Apple ships them; QuadCam bundles nothing.
const SYSTEM: &[&str] = &[
    "/bin/df",
    "/bin/ls",
    "/bin/ps",
    "/bin/sleep",
    "/bin/stty",
    "/usr/bin/afplay",
    "/usr/bin/codesign",
    "/usr/bin/curl",
    "/usr/bin/ditto",
    "/usr/bin/env",
    "/usr/bin/osascript",
    "/usr/bin/say",
    "/usr/bin/tar",
    "/usr/bin/true",
    "/usr/bin/unzip",
    "/usr/bin/xattr",
    "/usr/bin/zip",
    "/usr/sbin/diskutil",
    "/usr/sbin/ioreg",
    "/usr/sbin/system_profiler",
];

/// What `homebrew_tool` may look up.
const HOMEBREW: &[&str] = &["exiftool", "ffmpeg", "ffprobe"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every string literal that follows `needle` in the source, up to the closing quote.
fn literals_after(src: &str, needle: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(needle) {
        rest = &rest[i + needle.len()..];
        if let Some(end) = rest.find('"') {
            out.push(rest[..end].to_string());
        }
    }
    out
}

#[test]
fn quadcam_runs_only_system_tools_and_modules() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut seen = Vec::new();
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        // Drop `#[cfg(test)]` modules: test helpers may call anything on the test machine.
        let src = src.split("#[cfg(test)]").next().unwrap().to_string();
        for needle in [
            "Command::new(\"",
            "const DISKUTIL: &str = \"",
            "const PROGRAM: &str = \"",
        ] {
            for lit in literals_after(&src, needle) {
                assert!(
                    SYSTEM.contains(&lit.as_str()),
                    "{} runs {lit:?}, which is not a listed macOS system tool. Use a module, or add it to this list and to docs/modules.md.",
                    f.display()
                );
                seen.push(lit);
            }
        }
        for lit in literals_after(&src, "homebrew_tool(\"") {
            assert!(
                HOMEBREW.contains(&lit.as_str()),
                "{} looks up {lit:?} on Homebrew. Only ffmpeg, ffprobe and exiftool may come from there.",
                f.display()
            );
        }
    }
    // The scan itself works: it saw the tools the app is known to call.
    for must in ["/usr/sbin/diskutil", "/usr/bin/curl"] {
        assert!(seen.iter().any(|s| s == must), "the scan never saw {must}");
    }
}
