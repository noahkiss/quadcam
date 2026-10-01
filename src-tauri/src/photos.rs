//! Share to Photos: add verified outputs to the macOS Photos library through PhotoKit,
//! optionally into an album. `PhotosLibrary` is a trait so tests can use a mock and never
//! touch a real library.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Album the app suggests. An empty album setting adds to the library only.
pub const DEFAULT_ALBUM: &str = "Drone";

pub trait PhotosLibrary: Send + Sync {
    /// Adds every file as a video asset in one change, and into `album` when given
    /// (created if missing). All or nothing: an Err means none were added.
    fn add_videos(&self, files: &[PathBuf], album: Option<&str>) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShareReport {
    pub added: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub album: Option<String>,
}

fn check_file(p: &Path) -> Result<()> {
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "mp4" | "mov") {
        bail!("not an MP4 or MOV file");
    }
    if !p.is_file() {
        bail!("file is missing");
    }
    Ok(())
}

/// Checks each file, then hands the good ones to the library in one change.
pub fn share(
    lib: &dyn PhotosLibrary,
    files: &[PathBuf],
    album: Option<&str>,
) -> Result<ShareReport> {
    if files.is_empty() {
        bail!("Nothing to add: no verified clips.");
    }
    let album = album
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string);
    let mut report = ShareReport {
        added: Vec::new(),
        failed: Vec::new(),
        album: album.clone(),
    };
    let mut good = Vec::new();
    for f in files {
        match check_file(f) {
            Ok(()) => good.push(f.clone()),
            Err(e) => report.failed.push((f.clone(), e.to_string())),
        }
    }
    if good.is_empty() {
        return Ok(report);
    }
    match lib.add_videos(&good, album.as_deref()) {
        Ok(()) => report.added = good,
        Err(e) => report
            .failed
            .extend(good.into_iter().map(|f| (f, format!("{e:#}")))),
    }
    Ok(report)
}

#[cfg(target_os = "macos")]
pub use photokit::PhotoKit;

#[cfg(target_os = "macos")]
mod photokit {
    use super::PhotosLibrary;
    use anyhow::{anyhow, bail, Result};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_foundation::{NSArray, NSString, NSURL};
    use objc2_photos::{
        PHAccessLevel, PHAssetChangeRequest, PHAssetCollection, PHAssetCollectionChangeRequest,
        PHAssetCollectionSubtype, PHAssetCollectionType, PHAuthorizationStatus,
        PHObjectPlaceholder, PHPhotoLibrary,
    };
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The real library. Needs `NSPhotoLibraryAddUsageDescription` (and, for an album,
    /// `NSPhotoLibraryUsageDescription`) in Info.plist. Call it off the main thread.
    pub struct PhotoKit;

    const SETTINGS_HINT: &str =
        "Allow it in System Settings > Privacy & Security > Photos, then try again.";

    /// Asks for access when macOS has not asked yet, and waits for the answer.
    fn ensure_access(level: PHAccessLevel) -> Result<()> {
        let mut status = unsafe { PHPhotoLibrary::authorizationStatusForAccessLevel(level) };
        if status == PHAuthorizationStatus::NotDetermined {
            let (tx, rx) = mpsc::channel();
            let handler = RcBlock::new(move |s: PHAuthorizationStatus| {
                let _ = tx.send(s);
            });
            unsafe { PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(level, &handler) };
            status = rx
                .recv_timeout(Duration::from_secs(300))
                .map_err(|_| anyhow!("No answer to the Photos permission prompt."))?;
        }
        match status {
            PHAuthorizationStatus::Authorized | PHAuthorizationStatus::Limited => Ok(()),
            PHAuthorizationStatus::Denied => {
                bail!("quadcam is not allowed to add to Photos. {SETTINGS_HINT}")
            }
            PHAuthorizationStatus::Restricted => {
                bail!("Photos access is restricted on this Mac (parental controls or a profile).")
            }
            _ => bail!("Photos access was not granted. {SETTINGS_HINT}"),
        }
    }

    fn find_album(title: &str) -> Option<Retained<PHAssetCollection>> {
        let found = unsafe {
            PHAssetCollection::fetchAssetCollectionsWithType_subtype_options(
                PHAssetCollectionType::Album,
                PHAssetCollectionSubtype::AlbumRegular,
                None,
            )
        };
        (0..unsafe { found.count() })
            .map(|i| unsafe { found.objectAtIndex(i) })
            .find(|c| unsafe { c.localizedTitle() }.is_some_and(|t| t.to_string() == title))
    }

    impl PhotosLibrary for PhotoKit {
        fn add_videos(&self, files: &[PathBuf], album: Option<&str>) -> Result<()> {
            // Add-only access is enough for the library; an album needs read-write to find it.
            let level = if album.is_some() {
                PHAccessLevel::ReadWrite
            } else {
                PHAccessLevel::AddOnly
            };
            ensure_access(level)?;

            let urls: Vec<Retained<NSURL>> = files
                .iter()
                .map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())))
                .collect();
            let existing = album.and_then(find_album);
            let album = album.map(str::to_string);
            let change = RcBlock::new(move || unsafe {
                let mut placeholders: Vec<Retained<PHObjectPlaceholder>> = Vec::new();
                for url in &urls {
                    if let Some(req) =
                        PHAssetChangeRequest::creationRequestForAssetFromVideoAtFileURL(url)
                    {
                        if let Some(ph) = req.placeholderForCreatedAsset() {
                            placeholders.push(ph);
                        }
                    }
                }
                if let Some(title) = &album {
                    let req = match &existing {
                        Some(c) => PHAssetCollectionChangeRequest::changeRequestForAssetCollection(c),
                        None => Some(PHAssetCollectionChangeRequest::creationRequestForAssetCollectionWithTitle(
                            &NSString::from_str(title),
                        )),
                    };
                    if let Some(req) = req {
                        let arr = NSArray::from_retained_slice(&placeholders);
                        req.addAssets(ProtocolObject::from_ref(&*arr));
                    }
                }
            });
            let lib = unsafe { PHPhotoLibrary::sharedPhotoLibrary() };
            unsafe { lib.performChangesAndWait_error(RcBlock::as_ptr(&change)) }
                .map_err(|e| anyhow!("Photos refused the import: {}", e.localizedDescription()))
        }
    }
}

/// A library that records what it was asked to add and adds nothing. Tests use it, and the
/// CLI uses it for `--dry-run`, so nothing ever reaches a real Photos library.
#[derive(Default)]
pub struct Recorder {
    pub calls: std::sync::Mutex<Vec<(Vec<PathBuf>, Option<String>)>>,
    pub fail: bool,
}

impl PhotosLibrary for Recorder {
    fn add_videos(&self, files: &[PathBuf], album: Option<&str>) -> Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push((files.to_vec(), album.map(str::to_string)));
        if self.fail {
            bail!("Photos refused the import: mock failure");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_checks_files_and_batches_one_change() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("2026-09-30_flight.mp4");
        let b = d.path().join("2026-09-30_flight-2.mov");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let gone = d.path().join("gone.mp4");
        let avi = d.path().join("PICT0001.AVI");
        std::fs::write(&avi, b"x").unwrap();

        let m = Recorder::default();
        let r = share(
            &m,
            &[a.clone(), gone.clone(), b.clone(), avi.clone()],
            Some("  Drone "),
        )
        .unwrap();
        assert_eq!(r.added, vec![a.clone(), b.clone()]);
        assert_eq!(r.failed.len(), 2);
        assert_eq!(r.album.as_deref(), Some("Drone"));
        let calls = m.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "one change for the batch");
        assert_eq!(calls[0], (vec![a, b], Some("Drone".to_string())));
    }

    #[test]
    fn share_reports_library_failure_and_empty_album() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("x.mp4");
        std::fs::write(&a, b"x").unwrap();
        let m = Recorder {
            fail: true,
            ..Default::default()
        };
        let r = share(&m, std::slice::from_ref(&a), Some("")).unwrap();
        assert!(r.added.is_empty());
        assert!(r.failed[0].1.contains("mock failure"));
        assert_eq!(
            m.calls.lock().unwrap()[0].1,
            None,
            "an empty album means library only"
        );
        assert!(share(&m, &[], None).is_err());
    }
}
