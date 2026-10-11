//! Shared test helpers: synthetic DVR-like clips and disk images.
#![allow(dead_code)]

pub mod server;

use quadcam_lib::media::{self, Tools};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

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

/// A DJI-like MP4: H.264 at 30000/1001 fps, a cover picture (MJPEG, attached picture), the
/// format tags `comment` (EIS and FOV, as DJI writes them) and `creation_time`, and `moov`
/// last. DJI's own data streams (`djmd`, `dbgi`) are not reproduced: ffmpeg's MP4 muxer
/// cannot write them. Small (640x360) so the tests stay fast.
pub fn make_dji_clip(path: &Path, secs: u64, creation_time: &str) {
    let cover = path.with_extension("cover.jpg");
    let st = Command::new(tools().ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x180",
        ])
        .args(["-frames:v", "1"])
        .arg(&cover)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg failed making the cover");
    let st = Command::new(tools().ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=size=640x360:rate=30000/1001")
        .arg("-i")
        .arg(&cover)
        .args(["-map", "0:v", "-map", "1:v", "-t", &secs.to_string()])
        .args([
            "-c:v:0",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:v:1", "copy", "-disposition:v:1", "attached_pic"])
        .args(["-metadata", "comment=EIS:RS;FOV:Linear;"])
        .args(["-metadata", &format!("creation_time={creation_time}")])
        .args(["-f", "mp4"])
        .arg(path)
        .status()
        .unwrap();
    std::fs::remove_file(&cover).unwrap();
    assert!(st.success(), "ffmpeg failed making {}", path.display());
}

/// A hand-truncated copy, standing in for a power-off mid-record.
pub fn truncate_copy(src: &Path, dst: &Path, fraction: f64) {
    let data = std::fs::read(src).unwrap();
    let n = (data.len() as f64 * fraction) as usize;
    std::fs::write(dst, &data[..n]).unwrap();
}
/// The clip name an O4 air unit writes.
pub const DJI_NAME: &str = "DJI_20260105120000_0001_D.MP4";

/// A card like an O4 air unit shows over USB: `DCIM/DJI_001/` with one `secs`-long clip and
/// its `.SRT`, and DJI's `MISC/` housekeeping. Returns the clip.
pub fn dji_card(root: &Path, secs: u64) -> PathBuf {
    let dir = root.join("DCIM/DJI_001");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(root.join("MISC/THM")).unwrap();
    std::fs::write(root.join("MISC/FC8770.db"), b"housekeeping").unwrap();
    let clip = dir.join(DJI_NAME);
    make_dji_clip(&clip, secs, "2026-01-05T17:00:00Z");
    std::fs::write(
        clip.with_extension("SRT"),
        "1\n00:00:00,000 --> 00:00:01,000\nsubtitle\n",
    )
    .unwrap();
    clip
}

/// A FAT32 disk image attached at a private mount point (never under /Volumes, so it can
/// not collide with a real card). Detaches on drop.
pub struct Image {
    pub mount: PathBuf,
    pub disk: String,
    pub file: PathBuf,
    _dir: tempfile::TempDir,
}

/// Disk-image tests take turns machine-wide, across processes and worktrees: hdiutil and
/// diskutil share one daemon, and a detached image's disk number goes to the next attach, so
/// another test could mount or erase this image by a stale number. The first image in a
/// process takes a lock file and holds it until the process exits; the first image in a test
/// thread (plain `cargo test` runs a file's tests side by side) holds a mutex until the thread
/// ends. A test that makes several images takes each lock once.
fn disk_image_lock() {
    static FILE: OnceLock<std::fs::File> = OnceLock::new();
    static THREADS: Mutex<()> = Mutex::new(());
    thread_local! {
        static HELD: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
    }
    FILE.get_or_init(|| {
        let path = std::env::temp_dir().join("quadcam-disk-image-tests.lock");
        let f = std::fs::File::create(&path).unwrap();
        f.lock().unwrap();
        f
    });
    HELD.with(|h| {
        h.borrow_mut()
            .get_or_insert_with(|| THREADS.lock().unwrap_or_else(|e| e.into_inner()));
    });
}

impl Image {
    pub fn create(size: &str, label: &str, sparse: bool) -> Image {
        disk_image_lock();
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("card");
        let file = if sparse {
            base.with_extension("sparseimage")
        } else {
            base.with_extension("dmg")
        };
        // `hdiutil create` attaches the new image to format it, so it can fail on a loaded
        // machine as an attach does: a failed create removes what it left and tries again,
        // up to 3 times.
        let mut tries = 0;
        let out = loop {
            tries += 1;
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
            let out = c.arg(&base).output().unwrap();
            if out.status.success() || tries == 3 {
                break out;
            }
            eprintln!(
                "hdiutil create, try {tries}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            let _ = std::fs::remove_file(&file);
            std::thread::sleep(std::time::Duration::from_secs(2));
        };
        assert!(
            out.status.success(),
            "hdiutil create: {}",
            String::from_utf8_lossy(&out.stderr)
        );
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

    /// Attaches at our mount point. On a loaded machine the file system probe can time out
    /// ("no mountable file systems"): that one error detaches and tries again, up to 3 times.
    pub fn attach(&mut self) {
        let mut tries = 0;
        let out = loop {
            tries += 1;
            let out = Command::new("/usr/bin/hdiutil")
                .args(["attach", "-nobrowse", "-mountpoint"])
                .arg(&self.mount)
                .arg(&self.file)
                .output()
                .unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            if out.status.success() || tries == 3 || !err.contains("no mountable file systems") {
                break out;
            }
            eprintln!("hdiutil attach, try {tries}: {}", err.trim());
            self.detach();
            std::thread::sleep(std::time::Duration::from_secs(2));
        };
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

    /// The safety check before any erase or unmount: our disk number is a disk image, it is
    /// the one our image file is attached as now, and a volume mounted at our mount point
    /// sits on it.
    pub fn assert_is_test_image(&self) {
        let whole = quadcam_lib::disk::info(&self.disk).unwrap();
        assert_eq!(
            whole.bus_protocol.as_deref(),
            Some("Disk Image"),
            "refusing to touch a non-image disk"
        );
        assert_eq!(
            self.attached_disk().as_deref(),
            Some(self.disk.as_str()),
            "{} is not our image",
            self.disk
        );
        if let Ok(vol) = quadcam_lib::disk::info(&self.mount.to_string_lossy()) {
            if vol.mount_point.is_some() {
                assert_eq!(
                    quadcam_lib::disk::whole_disk_of(&vol.parent_whole_disk),
                    self.disk
                );
            }
        }
    }

    pub fn is_attached(&self) -> bool {
        self.attached_disk().is_some()
    }

    /// Our image is attached and its volume is mounted. After "safe to remove"
    /// (`diskutil unmountDisk`) the image stays attached but nothing is mounted.
    pub fn is_mounted(&self) -> bool {
        self.attached_disk().is_some_and(|d| {
            quadcam_lib::disk::info(&format!("{d}s1")).is_ok_and(|i| i.mount_point.is_some())
        })
    }

    /// Mounts our image again after an unmount: detach, then attach at the same mount point.
    pub fn remount(&mut self) {
        self.detach();
        self.attach();
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
