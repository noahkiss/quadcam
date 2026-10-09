//! Firmware (design 6.4, 6.5, 7.5): the version check, and the flash path for an EdgeTX
//! radio. Everything that reaches the network goes through `Fetch`, and everything that
//! writes a device goes through `Flasher`; a process started by cargo gets an offline
//! fetcher and a recorder, so no test downloads or flashes.
//!
//! - `check`: newest releases (EdgeTX, Betaflight, ExpressLRS) against what each saved device
//!   reports, cached under `<cache>/firmware/latest.json`.
//! - `edgetx`: a release's board binary, its checks and the splash patch, as an apply plan.
//!
//! QuadCam bundles no firmware. Images come from the upstream release at run time, on the
//! person's action, and sit in `<cache>/firmware/<product>/<version>/` with their SHA-256.

pub mod check;
pub mod edgetx;

use super::dfu::{self, FakeDfu, Usb};
use crate::modules::fetch::Fetch;
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The way to a DFU device. Real flashing is reachable only through here.
pub trait Flasher: Send + Sync {
    /// Opens the one DFU device with these ids.
    fn open_dfu(&self, vid: u16, pid: u16, serial: Option<&str>) -> Result<Box<dyn Usb>>;
    /// True for a fake: no sleeping on the device's poll timeouts.
    fn quick(&self) -> bool {
        false
    }
}

/// The real USB path (`nusb`).
pub struct RealFlasher;

impl Flasher for RealFlasher {
    fn open_dfu(&self, vid: u16, pid: u16, serial: Option<&str>) -> Result<Box<dyn Usb>> {
        Ok(Box::new(dfu::NusbUsb::open(vid, pid, serial)?))
    }
}

/// A DFU device in memory, shared with the test that made it.
struct Shared(Arc<Mutex<FakeDfu>>);

impl Usb for Shared {
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.0.lock().unwrap().control_out(request, value, data)
    }
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>> {
        self.0.lock().unwrap().control_in(request, value, len)
    }
    fn layout(&mut self) -> Result<String> {
        self.0.lock().unwrap().layout()
    }
}

/// Flashes a `FakeDfu`. Tests keep `device` to inspect it afterwards.
pub struct Recorder {
    pub device: Arc<Mutex<FakeDfu>>,
    /// Each open, as `vid:pid`.
    pub opened: Mutex<Vec<String>>,
}

impl Recorder {
    pub fn new(device: FakeDfu) -> Recorder {
        Recorder {
            device: Arc::new(Mutex::new(device)),
            opened: Mutex::new(Vec::new()),
        }
    }

    /// A device with an empty flash.
    pub fn empty() -> Recorder {
        Recorder::new(FakeDfu::with_firmware(&[]))
    }
}

impl Flasher for Recorder {
    fn open_dfu(&self, vid: u16, pid: u16, _serial: Option<&str>) -> Result<Box<dyn Usb>> {
        self.opened
            .lock()
            .unwrap()
            .push(format!("{vid:04x}:{pid:04x}"));
        Ok(Box::new(Shared(self.device.clone())))
    }
    fn quick(&self) -> bool {
        true
    }
}

/// Whether to flash real devices: `QUADCAM_FLASH=real` forces it on and any other value off;
/// unset, a process started by cargo gets the recorder.
pub fn flash_enabled(setting: Option<&str>, under_cargo: bool) -> bool {
    match setting {
        Some("real") => true,
        Some(_) => false,
        None => !under_cargo,
    }
}

/// The flasher this process may use (see the module docs).
pub fn system_flasher() -> Arc<dyn Flasher> {
    if flash_enabled(
        std::env::var("QUADCAM_FLASH").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        Arc::new(RealFlasher)
    } else {
        Arc::new(Recorder::empty())
    }
}

/// What the firmware code reaches outside the process: the network and the USB path.
#[derive(Clone)]
pub struct FwEnv {
    pub fetch: Arc<dyn Fetch>,
    pub flasher: Arc<dyn Flasher>,
}

impl FwEnv {
    /// The real fetcher (offline under cargo) and `system_flasher`.
    pub fn system() -> FwEnv {
        FwEnv {
            fetch: Arc::new(crate::modules::fetch::real_fetch()),
            flasher: system_flasher(),
        }
    }
}

/// A fetcher that serves files from memory, by URL. For tests and fixtures.
#[derive(Default)]
pub struct FixtureFetch {
    files: Mutex<HashMap<String, Vec<u8>>>,
    /// Every URL asked for, in order.
    pub requested: Mutex<Vec<String>>,
}

impl FixtureFetch {
    pub fn new() -> FixtureFetch {
        FixtureFetch::default()
    }

    pub fn serve(&self, url: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(url.to_string(), bytes);
    }

    pub fn count(&self, url: &str) -> usize {
        self.requested
            .lock()
            .unwrap()
            .iter()
            .filter(|u| *u == url)
            .count()
    }
}

impl Fetch for FixtureFetch {
    fn download(&self, url: &str, dest: &Path) -> Result<()> {
        self.requested.lock().unwrap().push(url.to_string());
        let files = self.files.lock().unwrap();
        let bytes = files
            .get(url)
            .ok_or_else(|| anyhow!("The download failed: no fixture for {url}"))?;
        std::fs::write(dest, bytes)?;
        Ok(())
    }
}

/// Downloads `url` to a scratch file next to `dest` and returns its bytes.
pub fn fetch_bytes(fetch: &dyn Fetch, url: &str, scratch: &Path) -> Result<Vec<u8>> {
    std::fs::create_dir_all(scratch)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = scratch.join(format!(
        ".download-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let r = fetch.download(url, &tmp);
    let bytes = std::fs::read(&tmp);
    let _ = std::fs::remove_file(&tmp);
    r?;
    match bytes {
        Ok(b) => Ok(b),
        Err(e) => bail!("The download failed: {e}"),
    }
}
