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
    tauri_build::build()
}
