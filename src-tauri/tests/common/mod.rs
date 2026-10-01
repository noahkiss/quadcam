//! Shared test helpers: synthetic DVR-like clips and disk images.
#![allow(dead_code)]

use quadcam_lib::media::{self, Tools};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn tools() -> Tools {
    media::find_tools().expect("tests need ffmpeg and ffprobe (brew install ffmpeg)")
}

pub const FPS: u64 = 30;

/// An MJPEG + PCM AVI like an analog DVR writes: 720x480, 30 fps, mono 32 kHz audio.
pub fn make_clip(path: &Path, secs: u64, audio: bool) {
    let mut c = Command::new(tools().ffmpeg);
    c.args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc=size=720x480:rate={FPS}"));
    if audio {
        c.args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=32000"]);
    }
    c.args([
        "-t",
        &secs.to_string(),
        "-c:v",
        "mjpeg",
        "-q:v",
        "5",
        "-pix_fmt",
        "yuvj420p",
    ]);
    if audio {
        c.args(["-c:a", "pcm_s16le", "-ac", "1"]);
    }
    c.args(["-f", "avi"]).arg(path);
    let st = c.status().unwrap();
    assert!(st.success(), "ffmpeg failed making {}", path.display());
}

/// A hand-truncated copy, standing in for a power-off mid-record.
pub fn truncate_copy(src: &Path, dst: &Path, fraction: f64) {
    let data = std::fs::read(src).unwrap();
    let n = (data.len() as f64 * fraction) as usize;
    std::fs::write(dst, &data[..n]).unwrap();
}

/// A FAT32 disk image attached at a private mount point (never under /Volumes, so it can
/// not collide with a real card). Detaches on drop.
pub struct Image {
    pub mount: PathBuf,
    pub disk: String,
    pub file: PathBuf,
    _dir: tempfile::TempDir,
}

impl Image {
    pub fn create(size: &str, label: &str, sparse: bool) -> Image {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("card");
        let mut c = Command::new("/usr/bin/hdiutil");
        c.args([
            "create",
            "-quiet",
            "-size",
            size,
            "-fs",
            "MS-DOS FAT32",
            "-layout",
            "MBRSPUD",
            "-volname",
            label,
        ]);
        if sparse {
            c.args(["-type", "SPARSE"]);
        }
        assert!(c.arg(&base).status().unwrap().success(), "hdiutil create");
        let file = if sparse {
            base.with_extension("sparseimage")
        } else {
            base.with_extension("dmg")
        };
        let mount = dir.path().join("mnt");
        std::fs::create_dir(&mount).unwrap();
        let mut img = Image {
            mount,
            disk: String::new(),
            file,
            _dir: dir,
        };
        img.attach();
        img
    }

    pub fn attach(&mut self) {
        let out = Command::new("/usr/bin/hdiutil")
            .args(["attach", "-nobrowse", "-mountpoint"])
            .arg(&self.mount)
            .arg(&self.file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "hdiutil attach: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        self.disk = text
            .split_whitespace()
            .next()
            .unwrap()
            .trim_start_matches("/dev/")
            .to_string();
    }

    /// The whole disk our image file is attached as right now, from `hdiutil info`. Disk
    /// numbers get reused as soon as an image detaches, so never trust a stored number alone.
    pub fn attached_disk(&self) -> Option<String> {
        let want = std::fs::canonicalize(&self.file).ok()?;
        let out = Command::new("/usr/bin/hdiutil").arg("info").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        for block in text.split("================================================") {
            let path = block.lines().find_map(|l| {
                l.strip_prefix("image-path")
                    .map(|r| r.trim_start().trim_start_matches(':').trim().to_string())
            });
            let same = path
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p == want);
            if same {
                return block
                    .lines()
                    .filter_map(|l| l.split_whitespace().next())
                    .find(|d| d.starts_with("/dev/disk") && !d[9..].contains('s'))
                    .map(|d| d.trim_start_matches("/dev/").to_string());
            }
        }
        None
    }

    /// Detaches our image only, found by its file, never by a remembered disk number.
    pub fn detach(&self) {
        if let Some(d) = self.attached_disk() {
            let _ = Command::new("/usr/bin/hdiutil")
                .args(["detach", "-quiet", "-force"])
                .arg(format!("/dev/{d}"))
                .status();
        }
    }

    pub fn is_attached(&self) -> bool {
        self.attached_disk().is_some()
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        self.detach();
    }
}

/// ffprobe format tags as a JSON value.
pub fn format_tags(path: &Path) -> serde_json::Value {
    let out = Command::new(tools().ffprobe)
        .args(["-v", "error", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["format"]["tags"].clone()
}

/// exiftool tags, or None when exiftool is not installed.
pub fn exif(path: &Path) -> Option<serde_json::Value> {
    let bin = ["/opt/homebrew/bin/exiftool", "/usr/local/bin/exiftool"]
        .into_iter()
        .find(|p| Path::new(p).exists())?;
    let out = Command::new(bin)
        .args([
            "-j",
            "-s",
            "-Title",
            "-Comment",
            "-CreateDate",
            "-Description",
        ])
        .arg(path)
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(v[0].clone())
}
