//! USB serial ports: the one way Gear talks to an FC or a radio's serial port.
//!
//! - `SerialLink` is an open port: write, read what arrives, and the "port gone" error
//!   (`PortGone`) when the device is unplugged or reboots. Dropping it closes the port.
//! - `Ports` lists ports and opens one. `RealPorts` wraps the `serialport` crate;
//!   `FakePorts` serves scripted links (`ScriptLink`) or any `SerialLink` a test makes, such
//!   as the Betaflight simulator.
//! - Every open takes a lock file per port (`<cache>/locks/`), so the app and the CLI never
//!   open the same port; the second gets the refusal `port_busy`.
//! - Fail-safe: a process started by cargo gets `NoPorts` from `system` and an empty list
//!   from `real_ports`, unless `QUADCAM_SERIAL=real`. A test cannot reach a real device even
//!   when it forgets to pass a fake.

use super::model::{Refusal, RefusalCode};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A USB serial port as the system lists it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct PortInfo {
    /// `/dev/cu.usbmodem...`
    pub port: String,
    pub vid: u16,
    pub pid: u16,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
}

/// The port went away: unplugged, or the device is rebooting. Callers that expect a
/// reboot (`save`, `exit`) wait for the port to come back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortGone(pub String);

impl std::fmt::Display for PortGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The port {} is gone (unplugged or rebooting).", self.0)
    }
}

impl std::error::Error for PortGone {}

/// True when `e` is a `PortGone`.
pub fn is_gone(e: &anyhow::Error) -> bool {
    e.downcast_ref::<PortGone>().is_some()
}

/// An open serial port. Dropping it closes the port and releases its lock.
pub trait SerialLink: Send {
    /// The port's path.
    fn port(&self) -> &str;
    /// Writes every byte.
    fn write_all(&mut self, data: &[u8]) -> Result<()>;
    /// The bytes that arrive within `timeout`; empty when none did. `PortGone` when the
    /// port went away.
    fn read_some(&mut self, timeout: Duration) -> Result<Vec<u8>>;
}

/// Lists and opens ports.
pub trait Ports: Send + Sync {
    fn list(&self) -> Vec<PortInfo>;
    fn open(&self, port: &str, baud: u32) -> Result<Box<dyn SerialLink>>;
}

/// How long `read_until` waits.
#[derive(Debug, Clone, Copy)]
pub struct Wait {
    /// The longest wait for `done`.
    pub timeout: Duration,
    /// After `done` holds, no new byte for this long (the Betaflight prompt rule: 300 ms).
    pub quiet: Duration,
    /// Each byte that arrives starts the timeout again (a long `dump all`).
    pub extend: bool,
}

impl Wait {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            quiet: Duration::ZERO,
            extend: false,
        }
    }
}

/// Reads until `done(buffer)` holds and the line is quiet for `wait.quiet`. Fails with the
/// bytes so far after the timeout, and with `PortGone` when the port goes away.
pub fn read_until(
    link: &mut dyn SerialLink,
    wait: Wait,
    done: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut deadline = Instant::now() + wait.timeout;
    let mut last_byte = Instant::now();
    loop {
        let now = Instant::now();
        if done(&buf) && now.duration_since(last_byte) >= wait.quiet {
            return Ok(buf);
        }
        if now >= deadline {
            return Err(anyhow!(
                "No answer on {} within {} s ({} bytes read).",
                link.port(),
                wait.timeout.as_secs_f32(),
                buf.len()
            ));
        }
        let slice = if done(&buf) {
            wait.quiet.saturating_sub(now.duration_since(last_byte))
        } else {
            deadline - now
        }
        .min(Duration::from_millis(100));
        let got = link.read_some(slice)?;
        if !got.is_empty() {
            buf.extend_from_slice(&got);
            last_byte = Instant::now();
            if wait.extend {
                deadline = last_byte + wait.timeout;
            }
        }
    }
}

/// Whether to reach real serial ports: `QUADCAM_SERIAL=real` forces them on and any other
/// value off; unset, a process started by cargo gets none.
pub fn serial_enabled(setting: Option<&str>, under_cargo: bool) -> bool {
    match setting {
        Some("real") => true,
        Some(_) => false,
        None => !under_cargo,
    }
}

fn enabled_here() -> bool {
    serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    )
}

/// The system's USB serial ports, or none in a process started by cargo (see the module
/// docs).
pub fn real_ports() -> Vec<PortInfo> {
    if !enabled_here() {
        return Vec::new();
    }
    RealPorts::list_system()
}

/// The ports this process may use: the real ones, or `NoPorts` under cargo.
pub fn system(lock_dir: PathBuf) -> Arc<dyn Ports> {
    if enabled_here() {
        Arc::new(RealPorts { lock_dir })
    } else {
        Arc::new(NoPorts)
    }
}

/// No ports, and every open refused: the fail-safe for tests.
pub struct NoPorts;

impl Ports for NoPorts {
    fn list(&self) -> Vec<PortInfo> {
        Vec::new()
    }
    fn open(&self, port: &str, _baud: u32) -> Result<Box<dyn SerialLink>> {
        Err(Refusal::new(
            RefusalCode::Disabled,
            format!(
                "Serial ports are off in this process ({port}); set QUADCAM_SERIAL=real to use them."
            ),
        )
        .into())
    }
}

/// The lock file of a port: `<lock_dir>/serial-<port name>.lock`.
pub fn lock_path(lock_dir: &Path, port: &str) -> PathBuf {
    let name = port.rsplit('/').next().unwrap_or(port);
    lock_dir.join(format!("serial-{}.lock", super::store::safe(name)))
}

/// Takes the port's lock without waiting. Held for as long as the returned file lives.
pub fn lock_port(lock_dir: &Path, port: &str) -> Result<File> {
    std::fs::create_dir_all(lock_dir)
        .with_context(|| format!("creating {}", lock_dir.display()))?;
    let path = lock_path(lock_dir, port);
    let f = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    match f.try_lock() {
        Ok(()) => Ok(f),
        Err(std::fs::TryLockError::WouldBlock) => Err(Refusal::new(
            RefusalCode::PortBusy,
            format!("{port} is open in another QuadCam window or the CLI."),
        )
        .into()),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("locking {}", path.display()))
        }
    }
}

/// A link that holds its port's lock.
struct Locked {
    inner: Box<dyn SerialLink>,
    _lock: File,
}

impl SerialLink for Locked {
    fn port(&self) -> &str {
        self.inner.port()
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.inner.write_all(data)
    }
    fn read_some(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        self.inner.read_some(timeout)
    }
}

/// Takes the lock, then opens.
fn open_locked(
    lock_dir: &Path,
    port: &str,
    open: impl FnOnce() -> Result<Box<dyn SerialLink>>,
) -> Result<Box<dyn SerialLink>> {
    let lock = lock_port(lock_dir, port)?;
    Ok(Box::new(Locked {
        inner: open()?,
        _lock: lock,
    }))
}

/// The system's ports, through the `serialport` crate.
pub struct RealPorts {
    lock_dir: PathBuf,
}

impl RealPorts {
    /// USB serial ports, one per device: the `/dev/cu.*` node on macOS.
    fn list_system() -> Vec<PortInfo> {
        let Ok(ports) = serialport::available_ports() else {
            return Vec::new();
        };
        let mut out: Vec<PortInfo> = ports
            .into_iter()
            .filter(|p| !p.port_name.starts_with("/dev/tty."))
            .filter_map(|p| match p.port_type {
                serialport::SerialPortType::UsbPort(u) => Some(PortInfo {
                    port: p.port_name,
                    vid: u.vid,
                    pid: u.pid,
                    serial_number: u.serial_number,
                    manufacturer: u.manufacturer,
                    product: u.product,
                }),
                _ => None,
            })
            .collect();
        out.sort_by(|a, b| a.port.cmp(&b.port));
        out
    }
}

impl Ports for RealPorts {
    fn list(&self) -> Vec<PortInfo> {
        Self::list_system()
    }
    fn open(&self, port: &str, baud: u32) -> Result<Box<dyn SerialLink>> {
        open_locked(&self.lock_dir, port, || {
            let p = serialport::new(port, baud)
                .timeout(Duration::from_millis(100))
                .open()
                .with_context(|| format!("opening {port}"))?;
            Ok(Box::new(RealLink {
                port: port.to_string(),
                inner: p,
            }) as Box<dyn SerialLink>)
        })
    }
}

struct RealLink {
    port: String,
    inner: Box<dyn serialport::SerialPort>,
}

impl RealLink {
    fn gone_or(&self, e: std::io::Error) -> anyhow::Error {
        use std::io::ErrorKind::*;
        match e.kind() {
            TimedOut | Interrupted | WouldBlock => anyhow!(e),
            _ => PortGone(self.port.clone()).into(),
        }
    }
}

impl SerialLink for RealLink {
    fn port(&self) -> &str {
        &self.port
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        use std::io::Write;
        match self.inner.write_all(data).and_then(|_| self.inner.flush()) {
            Ok(()) => Ok(()),
            Err(e) => Err(self.gone_or(e)),
        }
    }
    fn read_some(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        use std::io::Read;
        let _ = self
            .inner
            .set_timeout(timeout.max(Duration::from_millis(1)));
        let mut buf = [0u8; 4096];
        match self.inner.read(&mut buf) {
            Ok(0) => Err(PortGone(self.port.clone()).into()),
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut) => Ok(Vec::new()),
            Err(e) => Err(self.gone_or(e)),
        }
    }
}

// ----- fakes -----

/// One step of a scripted link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The next write must be exactly these bytes.
    Expect(Vec<u8>),
    /// These bytes arrive to be read.
    Reply(Vec<u8>),
    /// The port goes away (a reboot or an unplug).
    Gone,
}

impl Step {
    pub fn expect(s: impl AsRef<[u8]>) -> Self {
        Step::Expect(s.as_ref().to_vec())
    }
    pub fn reply(s: impl AsRef<[u8]>) -> Self {
        Step::Reply(s.as_ref().to_vec())
    }
}

/// A link that replays a script and records what was written. A write the script does not
/// expect fails, naming both.
pub struct ScriptLink {
    port: String,
    steps: VecDeque<Step>,
    pending: Vec<u8>,
    gone: bool,
    /// Every byte written, for the test to read.
    pub written: Arc<Mutex<Vec<u8>>>,
}

impl ScriptLink {
    pub fn new(port: &str, steps: Vec<Step>) -> Self {
        let mut l = Self {
            port: port.to_string(),
            steps: steps.into(),
            pending: Vec::new(),
            gone: false,
            written: Arc::default(),
        };
        l.queue_replies();
        l
    }

    fn queue_replies(&mut self) {
        while let Some(Step::Reply(r)) = self.steps.front() {
            self.pending.extend_from_slice(r);
            self.steps.pop_front();
        }
    }

    /// True when every step ran.
    pub fn finished(&self) -> bool {
        self.steps.is_empty() && self.pending.is_empty()
    }
}

impl SerialLink for ScriptLink {
    fn port(&self) -> &str {
        &self.port
    }
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        if self.gone {
            return Err(PortGone(self.port.clone()).into());
        }
        self.written.lock().unwrap().extend_from_slice(data);
        match self.steps.front() {
            Some(Step::Expect(e)) if e == data => {
                self.steps.pop_front();
                self.queue_replies();
                Ok(())
            }
            Some(Step::Gone) => {
                self.gone = true;
                self.steps.pop_front();
                Err(PortGone(self.port.clone()).into())
            }
            other => Err(anyhow!(
                "fake {}: wrote {:?}, the script expected {:?}",
                self.port,
                String::from_utf8_lossy(data),
                other
            )),
        }
    }
    fn read_some(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
        if !self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.pending));
        }
        if self.gone {
            return Err(PortGone(self.port.clone()).into());
        }
        if let Some(Step::Gone) = self.steps.front() {
            self.steps.pop_front();
            self.gone = true;
            return Err(PortGone(self.port.clone()).into());
        }
        // Nothing arrives: a timeout, without the wait.
        std::thread::yield_now();
        Ok(Vec::new())
    }
}

type Opener = Box<dyn Fn(&str, u32) -> Result<Box<dyn SerialLink>> + Send + Sync>;

/// Ports for tests and the mock core: a fixed list, and links from scripts or an opener.
pub struct FakePorts {
    ports: Vec<PortInfo>,
    scripts: Mutex<HashMap<String, Vec<Step>>>,
    opener: Option<Opener>,
    lock_dir: Option<PathBuf>,
}

impl FakePorts {
    pub fn new(ports: Vec<PortInfo>) -> Self {
        Self {
            ports,
            scripts: Mutex::default(),
            opener: None,
            lock_dir: None,
        }
    }

    /// The next open of `port` replays these steps.
    pub fn with_script(self, port: &str, steps: Vec<Step>) -> Self {
        self.scripts.lock().unwrap().insert(port.to_string(), steps);
        self
    }

    /// Opens ports without a script through `f` (a device simulator).
    pub fn with_opener(
        mut self,
        f: impl Fn(&str, u32) -> Result<Box<dyn SerialLink>> + Send + Sync + 'static,
    ) -> Self {
        self.opener = Some(Box::new(f));
        self
    }

    /// Takes the per-port lock in this folder on every open, as the real ports do.
    pub fn with_locks(mut self, dir: PathBuf) -> Self {
        self.lock_dir = Some(dir);
        self
    }
}

impl Ports for FakePorts {
    fn list(&self) -> Vec<PortInfo> {
        self.ports.clone()
    }
    fn open(&self, port: &str, baud: u32) -> Result<Box<dyn SerialLink>> {
        if !self.ports.iter().any(|p| p.port == port) {
            return Err(PortGone(port.to_string()).into());
        }
        let open = || -> Result<Box<dyn SerialLink>> {
            if let Some(steps) = self.scripts.lock().unwrap().remove(port) {
                return Ok(Box::new(ScriptLink::new(port, steps)));
            }
            match &self.opener {
                Some(f) => f(port, baud),
                None => Err(anyhow!("fake {port}: no script and no opener")),
            }
        };
        match &self.lock_dir {
            Some(dir) => open_locked(dir, port, open),
            None => open(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(p: &str) -> PortInfo {
        PortInfo {
            port: p.into(),
            vid: 0x0483,
            pid: 0x5740,
            ..Default::default()
        }
    }

    #[test]
    fn fail_safe() {
        assert!(serial_enabled(None, false), "the app uses real ports");
        assert!(
            !serial_enabled(None, true),
            "anything cargo started gets none"
        );
        assert!(!serial_enabled(Some("off"), false));
        assert!(serial_enabled(Some("real"), true));
    }

    /// This test binary runs under cargo: no real port is listed or opened.
    #[test]
    fn under_cargo_there_are_no_real_ports() {
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        if std::env::var("QUADCAM_SERIAL").as_deref() != Ok("real") {
            assert!(real_ports().is_empty());
            let d = tempfile::tempdir().unwrap();
            let ports = system(d.path().to_path_buf());
            assert!(ports.list().is_empty());
            let e = ports.open("/dev/cu.usbmodem0", 115200).err().unwrap();
            assert_eq!(
                e.downcast_ref::<Refusal>().unwrap().code,
                RefusalCode::Disabled
            );
        }
    }

    #[test]
    fn a_script_replays_and_checks_writes() {
        let ports = FakePorts::new(vec![port("/dev/cu.fake1")]).with_script(
            "/dev/cu.fake1",
            vec![
                Step::expect("#"),
                Step::reply("\r\nEntering CLI Mode\r\n# "),
                Step::expect("save\n"),
                Step::reply("Saving\r\n"),
                Step::Gone,
            ],
        );
        let mut link = ports.open("/dev/cu.fake1", 115200).unwrap();
        link.write_all(b"#").unwrap();
        let wait = Wait {
            timeout: Duration::from_secs(1),
            quiet: Duration::ZERO,
            extend: false,
        };
        let got = read_until(link.as_mut(), wait, |b| b.ends_with(b"\r\n# ")).unwrap();
        assert!(got.ends_with(b"# "));
        link.write_all(b"save\n").unwrap();
        let e = read_until(link.as_mut(), wait, |_| false).unwrap_err();
        assert!(is_gone(&e), "{e:#}");
        assert!(is_gone(&link.write_all(b"#").unwrap_err()));
        // A write the script does not expect fails.
        let ports = FakePorts::new(vec![port("/dev/cu.fake2")])
            .with_script("/dev/cu.fake2", vec![Step::expect("version\n")]);
        let mut link = ports.open("/dev/cu.fake2", 115200).unwrap();
        assert!(link.write_all(b"status\n").is_err());
        // A port not in the list is gone.
        assert!(is_gone(&ports.open("/dev/cu.nope", 115200).err().unwrap()));
    }

    #[test]
    fn read_until_times_out_with_what_it_read() {
        let mut link = ScriptLink::new("/dev/cu.fake", vec![Step::reply("partial")]);
        let e = read_until(&mut link, Wait::new(Duration::from_millis(50)), |b| {
            b.ends_with(b"# ")
        })
        .unwrap_err();
        assert!(format!("{e:#}").contains("7 bytes read"), "{e:#}");
    }

    #[test]
    fn one_open_per_port() {
        let d = tempfile::tempdir().unwrap();
        let ports = FakePorts::new(vec![port("/dev/cu.fake1"), port("/dev/cu.fake2")])
            .with_opener(|p, _| Ok(Box::new(ScriptLink::new(p, vec![]))))
            .with_locks(d.path().to_path_buf());
        let first = ports.open("/dev/cu.fake1", 115200).unwrap();
        let e = ports.open("/dev/cu.fake1", 115200).err().unwrap();
        assert_eq!(
            e.downcast_ref::<Refusal>().unwrap().code,
            RefusalCode::PortBusy
        );
        // Another port is fine, and closing frees the first.
        let _other = ports.open("/dev/cu.fake2", 115200).unwrap();
        drop(first);
        ports.open("/dev/cu.fake1", 115200).unwrap();
        assert_eq!(
            lock_path(d.path(), "/dev/cu.usbmodem1"),
            d.path().join("serial-cu.usbmodem1.lock")
        );
    }
}
