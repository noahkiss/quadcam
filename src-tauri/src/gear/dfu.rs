//! USB DFU 1.1 with the STM32 DfuSe extensions (design 6.5): erase, write, read back and
//! leave, for a radio in its ROM bootloader (`0483:df11`).
//!
//! Written from the USB DFU specification and ST's published DfuSe notes (the memory layout
//! string, the "set address pointer" and "erase" commands, block numbers from 2). There is
//! no official macOS `dfu-util` binary to download, and DFU is a USB class, so QuadCam
//! speaks it itself.
//!
//! - `Usb` is the transport: two control transfers and the alt-0 layout string. `NusbUsb`
//!   is the real one (`nusb`); `FakeDfu` is a DfuSe device in memory, with faults, and is
//!   what every test flashes.
//! - Nothing here flashes by itself. `Flasher` (in `firmware/mod.rs`) hands out a transport,
//!   and a process started by cargo gets the fake unless `QUADCAM_FLASH=real`.
//! - `read_verified` reads the whole flash twice through `ReadOnly`, a transport that cannot
//!   erase, write or leave, and returns a `VerifiedCopy`. `flash` takes one as an argument,
//!   so no code path erases without a verified copy of what it replaces.
//! - `flash` verifies every 16 KB segment as it writes it and reads the whole image back
//!   before it leaves DFU. A mismatch stays in DFU, so the
//!   person can try again: the bootloader is in ROM and cannot be overwritten from here.

use super::detect::{DfuInfo, STM32_DFU};
use anyhow::{anyhow, bail, Context, Result};
use std::time::Duration;

const DFU_DNLOAD: u8 = 1;
const DFU_UPLOAD: u8 = 2;
const DFU_GETSTATUS: u8 = 3;
const DFU_CLRSTATUS: u8 = 4;
const DFU_ABORT: u8 = 6;

const DFUSE_SET_ADDRESS: u8 = 0x21;
const DFUSE_ERASE: u8 = 0x41;

/// DFU states (DFU 1.1, table 6.1.2).
pub mod state {
    pub const DFU_IDLE: u8 = 2;
    pub const DNLOAD_SYNC: u8 = 3;
    pub const DNBUSY: u8 = 4;
    pub const DNLOAD_IDLE: u8 = 5;
    pub const MANIFEST_SYNC: u8 = 6;
    pub const MANIFEST: u8 = 7;
    pub const UPLOAD_IDLE: u8 = 9;
    pub const ERROR: u8 = 10;
}

/// The bytes moved per control transfer. STM32 bootloaders report 2,048 in their DFU
/// functional descriptor.
pub const TRANSFER: usize = 2048;

/// Where the STM32 internal flash starts.
pub const FLASH_BASE: u32 = 0x0800_0000;

/// One control transfer path to a DFU interface (interface 0, class requests).
pub trait Usb {
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()>;
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>>;
    /// The alt setting 0 string, `@Internal Flash /0x08000000/04*016Kg,...`.
    fn layout(&mut self) -> Result<String>;
    /// `wTransferSize` of the DFU functional descriptor, when the transport can read it.
    /// DfuSe addresses a block as `pointer + (block - 2) * wTransferSize`, so a device with
    /// another size than `TRANSFER` must not be written with it.
    fn transfer_size(&mut self) -> Option<usize> {
        None
    }
}

/// The bytes written, then read back and compared, before the next segment.
pub const SEGMENT: usize = 16 * 1024;

/// A transport that can only read: it passes status polls, aborts, error clears, the
/// set-address command and uploads, and refuses everything else (erase, write blocks,
/// leave). The read-only DFU trial runs through it.
pub struct ReadOnly<'a> {
    inner: &'a mut dyn Usb,
}

impl<'a> ReadOnly<'a> {
    pub fn new(inner: &'a mut dyn Usb) -> ReadOnly<'a> {
        ReadOnly { inner }
    }
}

impl Usb for ReadOnly<'_> {
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        let set_address =
            request == DFU_DNLOAD && value == 0 && data.len() == 5 && data[0] == DFUSE_SET_ADDRESS;
        if set_address || request == DFU_CLRSTATUS || request == DFU_ABORT {
            self.inner.control_out(request, value, data)
        } else {
            bail!("Refused: the read-only DFU path cannot send request {request} (value {value}).")
        }
    }
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>> {
        if request == DFU_GETSTATUS || request == DFU_UPLOAD {
            self.inner.control_in(request, value, len)
        } else {
            bail!("Refused: the read-only DFU path cannot send request {request}.")
        }
    }
    fn layout(&mut self) -> Result<String> {
        self.inner.layout()
    }
    fn transfer_size(&mut self) -> Option<usize> {
        self.inner.transfer_size()
    }
}

/// The answer to DFU_GETSTATUS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub status: u8,
    pub poll_ms: u32,
    pub state: u8,
}

fn status_name(s: u8) -> &'static str {
    match s {
        0 => "OK",
        1 => "target file rejected",
        2 => "file rejected",
        3 => "cannot write memory",
        4 => "cannot erase memory",
        5 => "erase check failed",
        6 => "cannot program memory",
        7 => "verify failed",
        8 => "address out of range",
        9 => "not done",
        10 => "firmware corrupt",
        _ => "error",
    }
}

/// One sector of the flash layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sector {
    pub addr: u32,
    pub size: u32,
    pub erasable: bool,
}

/// The memory layout from the alt-0 string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub name: String,
    pub sectors: Vec<Sector>,
}

impl Layout {
    /// Parses `@Internal Flash /0x08000000/04*016Kg,01*064Kg,07*128Kg`.
    pub fn parse(s: &str) -> Result<Layout> {
        let s = s.trim().trim_start_matches('@');
        let mut parts = s.splitn(3, '/');
        let name = parts.next().unwrap_or_default().trim().to_string();
        let start = parts
            .next()
            .and_then(|a| u32::from_str_radix(a.trim().trim_start_matches("0x"), 16).ok())
            .ok_or_else(|| anyhow!("The memory layout has no start address: {s}"))?;
        let segs = parts
            .next()
            .ok_or_else(|| anyhow!("The memory layout has no sectors: {s}"))?;
        let mut addr = start;
        let mut sectors = Vec::new();
        for seg in segs.split(',') {
            let seg = seg.trim();
            let (count, rest) = seg
                .split_once('*')
                .ok_or_else(|| anyhow!("Unreadable sector group `{seg}`"))?;
            let count: u32 = count.trim().parse().context("sector count")?;
            let kind = rest.chars().last().unwrap_or('a');
            let body = &rest[..rest.len() - kind.len_utf8()];
            let (digits, mult) = match body.chars().last() {
                Some('K') => (&body[..body.len() - 1], 1024),
                Some('M') => (&body[..body.len() - 1], 1024 * 1024),
                Some('B') => (&body[..body.len() - 1], 1),
                _ => (body.trim(), 1),
            };
            let size: u32 = digits.trim().parse::<u32>().context("sector size")? * mult;
            let erasable = (kind as u8).wrapping_sub(b'a').wrapping_add(1) & 2 != 0;
            if size == 0 || count == 0 || count > 4096 {
                bail!("Unreadable sector group `{seg}`");
            }
            for _ in 0..count {
                sectors.push(Sector {
                    addr,
                    size,
                    erasable,
                });
                addr += size;
            }
        }
        Ok(Layout { name, sectors })
    }

    pub fn start(&self) -> u32 {
        self.sectors.first().map_or(0, |s| s.addr)
    }

    /// One past the last byte.
    pub fn end(&self) -> u32 {
        self.sectors.last().map_or(0, |s| s.addr + s.size)
    }

    /// The sectors that hold any of `addr .. addr + len`.
    pub fn covering(&self, addr: u32, len: u32) -> Vec<Sector> {
        let end = addr.saturating_add(len);
        self.sectors
            .iter()
            .filter(|s| s.addr < end && s.addr + s.size > addr)
            .copied()
            .collect()
    }
}

/// A DfuSe device over a transport.
pub struct Dfu<'a> {
    usb: &'a mut dyn Usb,
    transfer: usize,
    /// Honour the device's poll timeout. Off for the fake.
    sleep: bool,
}

impl<'a> Dfu<'a> {
    pub fn new(usb: &'a mut dyn Usb) -> Dfu<'a> {
        let transfer = usb.transfer_size().unwrap_or(TRANSFER);
        Dfu {
            usb,
            transfer,
            sleep: true,
        }
    }

    /// For tests: no sleeping on the device's poll timeouts.
    pub fn quick(usb: &'a mut dyn Usb) -> Dfu<'a> {
        let transfer = usb.transfer_size().unwrap_or(TRANSFER);
        Dfu {
            usb,
            transfer,
            sleep: false,
        }
    }

    pub fn layout(&mut self) -> Result<Layout> {
        Layout::parse(&self.usb.layout()?)
    }

    pub fn status(&mut self) -> Result<Status> {
        let b = self.usb.control_in(DFU_GETSTATUS, 0, 6)?;
        if b.len() < 6 {
            bail!("The DFU status answer is {} bytes, not 6.", b.len());
        }
        Ok(Status {
            status: b[0],
            poll_ms: u32::from_le_bytes([b[1], b[2], b[3], 0]),
            state: b[4],
        })
    }

    /// Brings the device to `dfuIDLE`: clears an error, aborts a half-done transfer.
    pub fn to_idle(&mut self) -> Result<()> {
        for _ in 0..4 {
            let s = self.status()?;
            match s.state {
                state::DFU_IDLE => return Ok(()),
                state::ERROR => self.usb.control_out(DFU_CLRSTATUS, 0, &[])?,
                _ => self.usb.control_out(DFU_ABORT, 0, &[])?,
            }
        }
        bail!("The DFU device would not go idle.");
    }

    /// Sends a download block, then polls until the device is done with it.
    fn download(&mut self, value: u16, data: &[u8]) -> Result<()> {
        self.usb.control_out(DFU_DNLOAD, value, data)?;
        // First status: dfuDNBUSY with the time to wait; then poll until it is not busy.
        for _ in 0..2000 {
            let s = self.status()?;
            if s.status != 0 {
                let _ = self.usb.control_out(DFU_CLRSTATUS, 0, &[]);
                bail!("The DFU device refused: {}.", status_name(s.status));
            }
            if self.sleep && s.poll_ms > 0 {
                std::thread::sleep(Duration::from_millis(s.poll_ms as u64));
            }
            if s.state != state::DNBUSY && s.state != state::DNLOAD_SYNC {
                return Ok(());
            }
        }
        bail!("The DFU device stayed busy.");
    }

    fn command(&mut self, cmd: u8, addr: u32) -> Result<()> {
        let mut d = vec![cmd];
        d.extend_from_slice(&addr.to_le_bytes());
        self.download(0, &d)
    }

    pub fn set_address(&mut self, addr: u32) -> Result<()> {
        self.command(DFUSE_SET_ADDRESS, addr)
    }

    /// Erases the sector that holds `addr`.
    pub fn erase_sector(&mut self, addr: u32) -> Result<()> {
        self.command(DFUSE_ERASE, addr)
    }

    /// Writes `data` from `addr` (which must be erased). The last block is padded with
    /// 0xFF to a multiple of 4.
    pub fn write(&mut self, addr: u32, data: &[u8], progress: &mut dyn FnMut(usize)) -> Result<()> {
        self.set_address(addr)?;
        for (n, chunk) in data.chunks(self.transfer).enumerate() {
            let mut block = chunk.to_vec();
            while block.len() % 4 != 0 {
                block.push(0xFF);
            }
            self.download(2 + n as u16, &block)?;
            progress((n + 1) * self.transfer);
        }
        Ok(())
    }

    /// Reads `len` bytes from `addr`.
    pub fn read(
        &mut self,
        addr: u32,
        len: usize,
        progress: &mut dyn FnMut(usize),
    ) -> Result<Vec<u8>> {
        self.set_address(addr)?;
        self.to_idle()?;
        let mut out = Vec::with_capacity(len);
        let mut n = 0u16;
        while out.len() < len {
            let want = self.transfer.min(len - out.len()).max(1);
            let b = self.usb.control_in(DFU_UPLOAD, 2 + n, self.transfer)?;
            if b.is_empty() {
                bail!(
                    "The DFU device sent no data at {:#010x}.",
                    addr as usize + out.len()
                );
            }
            out.extend_from_slice(&b[..b.len().min(want)]);
            n += 1;
            progress(out.len());
        }
        self.to_idle()?;
        Ok(out)
    }

    /// Leaves DFU and starts the program at `addr` (the same operation as
    /// `dfu-util -s <addr>:leave`). The device resets; its last answer may be an error.
    pub fn leave(&mut self, addr: u32) -> Result<()> {
        self.set_address(addr)?;
        self.usb.control_out(DFU_DNLOAD, 0, &[])?;
        let _ = self.status();
        let _ = self.status();
        Ok(())
    }
}

/// What a flash did, for the apply report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashOutcome {
    pub erased: usize,
    pub written: usize,
    pub verified: usize,
}

/// The steps of a flash, for progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Erase,
    Write,
    ReadBack,
    Leave,
}

/// A copy of the device's flash read twice, with both reads equal. Only `read_verified`
/// makes one. `flash` needs one that covers the range it will erase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCopy {
    base: u32,
    bytes: Vec<u8>,
}

impl VerifiedCopy {
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Every byte read, to the end of the flash.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// True when every byte reads 0xFF: there is no firmware to keep.
    pub fn is_blank(&self) -> bool {
        self.bytes.iter().all(|b| *b == 0xFF)
    }

    /// The bytes without the erased tail (0xFF), at least one byte.
    pub fn trimmed(&self) -> Vec<u8> {
        let mut v = self.bytes.clone();
        while v.len() > 1 && v.last() == Some(&0xFF) {
            v.pop();
        }
        v
    }

    fn covers(&self, addr: u32, len: usize) -> bool {
        addr >= self.base
            && (addr as u64 + len as u64) <= self.base as u64 + self.bytes.len() as u64
    }
}

/// Reads all of the device's flash twice, over a transport that cannot write, and returns
/// the copy when both reads are equal. Refuses a flash that reads as all zeros (a
/// read-protected part), because that copy would restore nothing. A blank flash (all 0xFF,
/// as after an interrupted flash) is a copy: `is_blank` says so, and the caller decides.
/// `progress` gets
/// (bytes done, bytes in both reads).
pub fn read_verified(
    usb: &mut dyn Usb,
    quick: bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<VerifiedCopy> {
    let mut ro = ReadOnly::new(usb);
    let mut dfu = if quick {
        Dfu::quick(&mut ro)
    } else {
        Dfu::new(&mut ro)
    };
    let layout = dfu.layout()?;
    let (base, len) = (layout.start(), (layout.end() - layout.start()) as usize);
    dfu.to_idle()?;
    let total = len * 2;
    let first = dfu.read(base, len, &mut |n| progress(n, total))?;
    let second = dfu.read(base, len, &mut |n| progress(len + n, total))?;
    if let Some(i) = first.iter().zip(&second).position(|(a, b)| a != b) {
        bail!(
            "Two reads of the radio's flash differ (first difference at {:#010x}). The USB link is not reliable; try another cable or port.",
            base as usize + i
        );
    }
    if first.iter().all(|b| *b == 0) {
        bail!("The radio's flash reads as all zeros; it may be read-protected. The copy would restore nothing.");
    }
    Ok(VerifiedCopy { base, bytes: first })
}

/// Erases the pages `image` covers, writes it at `base` one segment at a time (each read
/// back and compared before the next), reads the whole image back and compares, then leaves
/// DFU. A failed compare returns an error and does not leave.
///
/// `copy` is a verified copy of the flash, which must cover the range. It is the argument
/// that makes an erase without a copy impossible to write.
pub fn flash(
    usb: &mut dyn Usb,
    base: u32,
    image: &[u8],
    copy: &VerifiedCopy,
    quick: bool,
    step: &mut dyn FnMut(Step),
) -> Result<FlashOutcome> {
    if image.is_empty() {
        bail!("There is nothing to flash.");
    }
    if !copy.covers(base, image.len()) {
        bail!(
            "Refused: the verified copy of the radio's firmware does not cover the range to erase."
        );
    }
    let mut dfu = if quick {
        Dfu::quick(usb)
    } else {
        Dfu::new(usb)
    };
    if dfu.transfer != TRANSFER {
        bail!(
            "The device moves {} bytes per transfer; QuadCam writes {TRANSFER}. Nothing was written.",
            dfu.transfer
        );
    }
    let layout = dfu.layout()?;
    let end = base as u64 + image.len() as u64;
    if base < layout.start() || end > layout.end() as u64 {
        bail!(
            "The image ({} bytes at {base:#010x}) does not fit the device's flash ({:#010x} to {:#010x}).",
            image.len(),
            layout.start(),
            layout.end()
        );
    }
    let sectors = layout.covering(base, image.len() as u32);
    if let Some(s) = sectors.iter().find(|s| !s.erasable) {
        bail!("The sector at {:#010x} cannot be erased.", s.addr);
    }
    dfu.to_idle()?;
    step(Step::Erase);
    for s in &sectors {
        dfu.erase_sector(s.addr)?;
    }
    step(Step::Write);
    for (n, chunk) in image.chunks(SEGMENT).enumerate() {
        let at = base + (n * SEGMENT) as u32;
        dfu.write(at, chunk, &mut |_| {})?;
        let back = dfu.read(at, chunk.len(), &mut |_| {})?;
        if let Some(i) = back.iter().zip(chunk).position(|(a, b)| a != b) {
            bail!(
                "The device holds different bytes than were written (first difference at {:#010x}, found while writing). It stays in DFU mode; nothing was started.",
                at as usize + i
            );
        }
    }
    step(Step::ReadBack);
    let back = dfu.read(base, image.len(), &mut |_| {})?;
    if let Some(i) = back.iter().zip(image).position(|(a, b)| a != b) {
        bail!(
            "The device holds different bytes than were written (first difference at {:#010x}). It stays in DFU mode; nothing was started.",
            base as usize + i
        );
    }
    step(Step::Leave);
    dfu.leave(base)?;
    Ok(FlashOutcome {
        erased: sectors.len(),
        written: image.len(),
        verified: back.len(),
    })
}

/// The size of the device's flash, from its memory layout.
pub fn flash_size(usb: &mut dyn Usb, _quick: bool) -> Result<usize> {
    let l = Layout::parse(&usb.layout()?)?;
    Ok((l.end() - l.start()) as usize)
}

// ---- the real transport ----

/// DFU devices plugged in now: the STM32 ROM bootloader and any other DFU-class device by
/// its ids. Reads only. None in a process started by cargo unless `QUADCAM_SERIAL=real`,
/// like the serial ports.
pub fn list() -> Vec<DfuInfo> {
    use nusb::MaybeFuture;
    if !super::serial::serial_enabled(
        std::env::var("QUADCAM_SERIAL").ok().as_deref(),
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
    ) {
        return Vec::new();
    }
    let Ok(devs) = nusb::list_devices().wait() else {
        return Vec::new();
    };
    devs.filter(|d| (d.vendor_id(), d.product_id()) == STM32_DFU)
        .map(|d| DfuInfo {
            vid: d.vendor_id(),
            pid: d.product_id(),
            serial: d.serial_number().map(str::to_string),
        })
        .collect()
}

/// The real transport, over `nusb`. Opening a device claims interface 0.
pub struct NusbUsb {
    iface: nusb::Interface,
    layout: String,
    transfer: Option<usize>,
}

impl NusbUsb {
    /// Opens the one DFU device with these ids (and this serial, when given).
    pub fn open(vid: u16, pid: u16, serial: Option<&str>) -> Result<NusbUsb> {
        use nusb::MaybeFuture;
        let mut found = nusb::list_devices()
            .wait()
            .map_err(|e| anyhow!("Listing USB devices failed: {e}"))?
            .filter(|d| {
                d.vendor_id() == vid
                    && d.product_id() == pid
                    && serial.is_none_or(|s| d.serial_number() == Some(s))
            });
        let info = found
            .next()
            .ok_or_else(|| anyhow!("No DFU device {vid:04x}:{pid:04x} is plugged in."))?;
        if found.next().is_some() {
            bail!("Two DFU devices {vid:04x}:{pid:04x} are plugged in; unplug one.");
        }
        let dev = info
            .open()
            .wait()
            .map_err(|e| anyhow!("Opening the DFU device failed: {e}"))?;
        let iface = dev
            .claim_interface(0)
            .wait()
            .map_err(|e| anyhow!("Claiming the DFU interface failed: {e}"))?;
        let alt0 = iface
            .descriptors()
            .find(|d| d.alternate_setting() == 0)
            .ok_or_else(|| anyhow!("The DFU device has no alt setting 0."))?;
        // The DFU functional descriptor (type 0x21): wTransferSize at bytes 5 and 6.
        let transfer = alt0
            .descriptors()
            .find(|d| d.descriptor_type() == 0x21 && d.len() >= 7)
            .map(|d| u16::from_le_bytes([d[5], d[6]]) as usize);
        let idx = alt0
            .string_index()
            .ok_or_else(|| anyhow!("The DFU device names no memory layout."))?;
        let layout = dev
            .get_string_descriptor(
                idx,
                nusb::descriptors::language_id::US_ENGLISH,
                Duration::from_secs(2),
            )
            .wait()
            .map_err(|e| anyhow!("Reading the memory layout failed: {e}"))?;
        Ok(NusbUsb {
            iface,
            layout,
            transfer,
        })
    }
}

impl Usb for NusbUsb {
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        use nusb::{
            transfer::{ControlOut, ControlType, Recipient},
            MaybeFuture,
        };
        self.iface
            .control_out(
                ControlOut {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: 0,
                    data,
                },
                Duration::from_secs(5),
            )
            .wait()
            .map_err(|e| anyhow!("USB transfer failed: {e}"))
    }

    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>> {
        use nusb::{
            transfer::{ControlIn, ControlType, Recipient},
            MaybeFuture,
        };
        self.iface
            .control_in(
                ControlIn {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: 0,
                    length: len as u16,
                },
                Duration::from_secs(5),
            )
            .wait()
            .map_err(|e| anyhow!("USB transfer failed: {e}"))
    }

    fn layout(&mut self) -> Result<String> {
        Ok(self.layout.clone())
    }

    fn transfer_size(&mut self) -> Option<usize> {
        self.transfer
    }
}

// ---- the fake ----

/// What a `FakeDfu` can get wrong.
#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// Programming ignores the block with this index (the bytes stay erased).
    pub drop_write_block: Option<usize>,
    /// Refuses the erase command with a status error.
    pub fail_erase: bool,
    /// Every upload flips the low bit of byte 0.
    pub corrupt_read: bool,
    /// After this many uploads, every upload flips the low bit of byte 0 (a link that went
    /// bad part way).
    pub flip_after_reads: Option<usize>,
}

enum Pending {
    SetAddress(u32),
    Erase(u32),
    Write { block: usize, data: Vec<u8> },
    Leave,
}

/// A DfuSe device in memory: the STM32F4 1 MB layout, a state machine as the specification
/// says, and a log of what it was asked. Programming clears bits only (flash cannot set a
/// bit), so a write to unerased flash reads back wrong, as on the real part.
pub struct FakeDfu {
    pub flash: Vec<u8>,
    pub layout: String,
    pub base: u32,
    pub state: u8,
    pub status: u8,
    pub pointer: u32,
    pub left: bool,
    pub erased: Vec<u32>,
    pub faults: Faults,
    pub log: Vec<String>,
    /// Data blocks programmed or dropped so far (what `drop_write_block` counts).
    pub blocks: usize,
    /// Uploads answered so far.
    pub reads: usize,
    /// Commands that changed the flash: erases and data blocks (a read-only path leaves 0).
    pub mutations: usize,
    pending: Option<Pending>,
}

/// The layout string of a 1 MB STM32F4 (the Pocket's).
pub const F4_1MB: &str = "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg";

impl FakeDfu {
    /// A device holding `firmware` at the start of its flash, the rest erased.
    pub fn with_firmware(firmware: &[u8]) -> FakeDfu {
        let layout = Layout::parse(F4_1MB).expect("the F4 layout parses");
        let mut flash = vec![0xFFu8; (layout.end() - layout.start()) as usize];
        flash[..firmware.len()].copy_from_slice(firmware);
        FakeDfu {
            flash,
            layout: F4_1MB.into(),
            base: layout.start(),
            state: state::DFU_IDLE,
            status: 0,
            pointer: layout.start(),
            left: false,
            erased: Vec::new(),
            faults: Faults::default(),
            log: Vec::new(),
            blocks: 0,
            reads: 0,
            mutations: 0,
            pending: None,
        }
    }

    fn sectors(&self) -> Vec<Sector> {
        Layout::parse(&self.layout)
            .map(|l| l.sectors)
            .unwrap_or_default()
    }

    fn fail(&mut self, status: u8) {
        self.status = status;
        self.state = state::ERROR;
    }

    fn run(&mut self, p: Pending) {
        match p {
            Pending::SetAddress(a) => {
                self.log.push(format!("set_address {a:#010x}"));
                self.pointer = a;
            }
            Pending::Erase(a) => {
                let Some(s) = self
                    .sectors()
                    .into_iter()
                    .find(|s| a >= s.addr && a < s.addr + s.size)
                else {
                    return self.fail(8);
                };
                if self.faults.fail_erase {
                    return self.fail(4);
                }
                self.mutations += 1;
                self.log.push(format!("erase {:#010x}", s.addr));
                self.erased.push(s.addr);
                let from = (s.addr - self.base) as usize;
                self.flash[from..from + s.size as usize].fill(0xFF);
            }
            Pending::Write { block, data } => {
                let at = self.pointer as u64 + (block * TRANSFER) as u64;
                if at < self.base as u64
                    || at + data.len() as u64 > self.base as u64 + self.flash.len() as u64
                {
                    return self.fail(8);
                }
                self.log
                    .push(format!("write {at:#010x} {} bytes", data.len()));
                self.mutations += 1;
                self.blocks += 1;
                if self.faults.drop_write_block == Some(self.blocks - 1) {
                    return;
                }
                let from = (at - self.base as u64) as usize;
                for (i, b) in data.iter().enumerate() {
                    self.flash[from + i] &= b;
                }
            }
            Pending::Leave => {
                self.mutations += 1;
                self.log.push(format!("leave {:#010x}", self.pointer));
                self.left = true;
            }
        }
    }
}

impl Usb for FakeDfu {
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        match request {
            DFU_DNLOAD => {
                if self.state != state::DFU_IDLE && self.state != state::DNLOAD_IDLE {
                    self.fail(9);
                    bail!("stalled: DNLOAD in state {}", self.state);
                }
                let p = if value == 0 && data.is_empty() {
                    Pending::Leave
                } else if value == 0 {
                    let addr = data
                        .get(1..5)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    match (data[0], addr) {
                        (DFUSE_SET_ADDRESS, Some(a)) => Pending::SetAddress(a),
                        (DFUSE_ERASE, Some(a)) => Pending::Erase(a),
                        _ => {
                            self.fail(3);
                            return Ok(());
                        }
                    }
                } else {
                    Pending::Write {
                        block: value as usize - 2,
                        data: data.to_vec(),
                    }
                };
                self.pending = Some(p);
                self.state = state::DNBUSY;
                Ok(())
            }
            DFU_CLRSTATUS => {
                self.status = 0;
                self.state = state::DFU_IDLE;
                Ok(())
            }
            DFU_ABORT => {
                self.pending = None;
                self.state = state::DFU_IDLE;
                Ok(())
            }
            _ => bail!("stalled: request {request}"),
        }
    }

    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>> {
        match request {
            DFU_GETSTATUS => {
                let report = (self.status, self.state);
                if self.state == state::DNBUSY {
                    let leave = matches!(self.pending, Some(Pending::Leave));
                    if let Some(p) = self.pending.take() {
                        self.run(p);
                    }
                    if self.state == state::DNBUSY {
                        self.state = if leave {
                            state::MANIFEST
                        } else {
                            state::DNLOAD_IDLE
                        };
                    }
                    // This answer says busy; the next says what it became.
                    return Ok(vec![report.0, 0, 0, 0, state::DNBUSY, 0]);
                }
                Ok(vec![report.0, 0, 0, 0, report.1, 0])
            }
            DFU_UPLOAD => {
                if value < 2 {
                    bail!("stalled: upload block {value}");
                }
                let at = self.pointer as u64 + ((value as usize - 2) * TRANSFER) as u64;
                let from = at.saturating_sub(self.base as u64) as usize;
                if from >= self.flash.len() {
                    return Ok(Vec::new());
                }
                let to = (from + len.min(TRANSFER)).min(self.flash.len());
                let mut b = self.flash[from..to].to_vec();
                self.log
                    .push(format!("upload {at:#010x} {} bytes", b.len()));
                self.reads += 1;
                let flaky = self.faults.flip_after_reads.is_some_and(|n| self.reads > n);
                if (self.faults.corrupt_read || flaky) && !b.is_empty() {
                    b[0] ^= 1;
                }
                self.state = state::UPLOAD_IDLE;
                Ok(b)
            }
            _ => bail!("stalled: request {request}"),
        }
    }

    fn layout(&mut self) -> Result<String> {
        Ok(self.layout.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `read_verified` would return for the device as it stands, for a test whose device
    /// is blank (a blank flash is refused as a copy).
    fn copy(dev: &FakeDfu) -> VerifiedCopy {
        VerifiedCopy {
            base: FLASH_BASE,
            bytes: dev.flash.clone(),
        }
    }

    fn image(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn the_layout_string_parses() {
        let l = Layout::parse(F4_1MB).unwrap();
        assert_eq!(l.name, "Internal Flash");
        assert_eq!(l.sectors.len(), 12);
        assert_eq!(l.start(), 0x0800_0000);
        assert_eq!(l.end(), 0x0810_0000);
        assert_eq!(l.sectors[0].size, 16 * 1024);
        assert_eq!(l.sectors[4].size, 64 * 1024);
        assert_eq!(l.sectors[5].addr, 0x0801_0000 + 0x1_0000);
        assert!(l.sectors.iter().all(|s| s.erasable));
        // 20 KB spans two 16 KB sectors; 16 KB exactly one.
        assert_eq!(l.covering(FLASH_BASE, 20 * 1024).len(), 2);
        assert_eq!(l.covering(FLASH_BASE, 16 * 1024).len(), 1);
        assert!(Layout::parse("@Internal Flash").is_err());
        assert!(Layout::parse("@x /0x08000000/zz").is_err());
        // A read-only sector type is not erasable.
        let ro = Layout::parse("@Option /0x1FFFC000/01*016Ka").unwrap();
        assert!(!ro.sectors[0].erasable);
    }

    #[test]
    fn flash_erases_writes_reads_back_compares_and_leaves() {
        let old = image(300 * 1024);
        let new = image(500 * 1024).into_iter().rev().collect::<Vec<u8>>();
        let mut dev = FakeDfu::with_firmware(&old);
        let c = copy(&dev);
        let mut steps = Vec::new();
        let out = flash(&mut dev, FLASH_BASE, &new, &c, true, &mut |s| steps.push(s)).unwrap();
        assert_eq!(
            steps,
            [Step::Erase, Step::Write, Step::ReadBack, Step::Leave]
        );
        assert_eq!(&dev.flash[..new.len()], &new[..]);
        assert!(dev.left, "the device left DFU");
        assert_eq!(out.written, new.len());
        assert_eq!(out.verified, new.len());
        // 500 KB: 16*4 + 64 + 128 * 3 = 512 KB, so 8 sectors.
        assert_eq!(out.erased, 8);
        assert_eq!(dev.erased.len(), 8);
        assert_eq!(dev.log.last().unwrap(), "leave 0x08000000");
        // Bytes past the image stay as they were (erased).
        assert!(dev.flash[new.len() + 12 * 1024..]
            .iter()
            .all(|b| *b == 0xFF));
    }

    #[test]
    fn every_segment_is_read_back_before_the_next_is_written() {
        let mut dev = FakeDfu::with_firmware(&image(1000));
        let c = copy(&dev);
        let img = image(40 * 1024);
        flash(&mut dev, FLASH_BASE, &img, &c, true, &mut |_| {}).unwrap();
        // The address and kind of each write and upload, in the order the device saw them.
        let ops: Vec<(&str, u32, usize)> = dev
            .log
            .iter()
            .filter_map(|l| {
                let mut w = l.split(' ');
                let kind = w.next()?;
                if kind != "write" && kind != "upload" {
                    return None;
                }
                let at = u32::from_str_radix(w.next()?.trim_start_matches("0x"), 16).ok()?;
                Some((kind, at, w.next()?.parse().ok()?))
            })
            .collect();
        let segments = img.len().div_ceil(SEGMENT);
        assert_eq!(segments, 3);
        let base = FLASH_BASE as usize;
        let seg = |at: u32| (at as usize - base) / SEGMENT;
        for n in 0..segments {
            let start = base + n * SEGMENT;
            let end = (start + SEGMENT).min(base + img.len());
            let writes: Vec<usize> = (0..ops.len())
                .filter(|&i| ops[i].0 == "write" && seg(ops[i].1) == n)
                .collect();
            assert_eq!(
                writes.len(),
                (end - start).div_ceil(TRANSFER),
                "segment {n}"
            );
            let last_write = *writes.last().unwrap();
            let next_write = (0..ops.len())
                .find(|&i| ops[i].0 == "write" && seg(ops[i].1) == n + 1)
                .unwrap_or(ops.len());
            // Between segment n's last write and segment n + 1's first, uploads cover every
            // byte of segment n.
            let mut covered = vec![false; end - start];
            for &(kind, at, len) in &ops[last_write + 1..next_write] {
                assert_eq!(kind, "upload", "segment {n}: a write before its read back");
                // After the last segment, the full read back covers the others too.
                let Some(from) = (at as usize)
                    .checked_sub(start)
                    .filter(|f| *f < end - start)
                else {
                    continue;
                };
                for c in covered.iter_mut().skip(from).take(len) {
                    *c = true;
                }
            }
            assert!(
                covered.iter().all(|c| *c),
                "segment {n} is read back in full before segment {} is written",
                n + 1
            );
        }
    }

    #[test]
    fn a_dropped_block_is_caught_while_writing_and_the_device_stays_in_dfu() {
        let mut dev = FakeDfu::with_firmware(&[1]);
        dev.faults.drop_write_block = Some(3);
        let c = copy(&dev);
        let err = flash(&mut dev, FLASH_BASE, &image(20_000), &c, true, &mut |_| {}).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("different bytes"), "{msg}");
        assert!(
            msg.contains("0x08001800"),
            "first difference is block 3: {msg}"
        );
        assert!(!dev.left, "must not start a half-written image");
        // The failure came before the next segment was written.
        assert!(!dev.log.iter().any(|l| l.starts_with("write 0x08004000")));
    }

    #[test]
    fn a_failed_erase_stops_before_any_write() {
        let mut dev = FakeDfu::with_firmware(&image(1000));
        dev.faults.fail_erase = true;
        let c = copy(&dev);
        let err = flash(&mut dev, FLASH_BASE, &image(5000), &c, true, &mut |_| {}).unwrap_err();
        assert!(format!("{err:#}").contains("cannot erase"), "{err:#}");
        assert!(!dev.log.iter().any(|l| l.starts_with("write")));
        assert!(!dev.left);
    }

    #[test]
    fn corrupt_uploads_fail_the_compare() {
        let mut dev = FakeDfu::with_firmware(&[1]);
        let c = copy(&dev);
        dev.faults.corrupt_read = true;
        assert!(flash(&mut dev, FLASH_BASE, &image(3000), &c, true, &mut |_| {}).is_err());
        assert!(!dev.left);
    }

    #[test]
    fn an_image_outside_the_flash_is_refused_before_any_command() {
        let mut dev = FakeDfu::with_firmware(&[1]);
        let c = copy(&dev);
        let err = flash(
            &mut dev,
            FLASH_BASE,
            &vec![0u8; 1024 * 1024 + 1],
            &c,
            true,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not cover"), "{err}");
        let err = flash(&mut dev, 0x2000_0000, &[1, 2, 3, 4], &c, true, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not cover"), "{err}");
        assert!(flash(&mut dev, FLASH_BASE, &[], &c, true, &mut |_| {}).is_err());
        assert!(dev.log.is_empty(), "nothing was sent: {:?}", dev.log);
        assert_eq!(dev.mutations, 0);
    }

    #[test]
    fn a_copy_that_does_not_cover_the_image_refuses_the_erase() {
        let mut dev = FakeDfu::with_firmware(&[1]);
        let short = VerifiedCopy {
            base: FLASH_BASE,
            bytes: vec![1; 1000],
        };
        let err = flash(
            &mut dev,
            FLASH_BASE,
            &image(5000),
            &short,
            true,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("does not cover"), "{err}");
        assert_eq!(dev.mutations, 0, "no erase without a covering copy");
    }

    #[test]
    fn a_device_with_another_transfer_size_is_not_written() {
        struct Odd(FakeDfu);
        impl Usb for Odd {
            fn control_out(&mut self, r: u8, v: u16, d: &[u8]) -> Result<()> {
                self.0.control_out(r, v, d)
            }
            fn control_in(&mut self, r: u8, v: u16, l: usize) -> Result<Vec<u8>> {
                self.0.control_in(r, v, l)
            }
            fn layout(&mut self) -> Result<String> {
                self.0.layout()
            }
            fn transfer_size(&mut self) -> Option<usize> {
                Some(1024)
            }
        }
        let inner = FakeDfu::with_firmware(&[1]);
        let c = copy(&inner);
        let mut dev = Odd(inner);
        let err = flash(&mut dev, FLASH_BASE, &image(5000), &c, true, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("1024"), "{err}");
        assert_eq!(dev.0.mutations, 0);
    }

    #[test]
    fn an_odd_length_image_is_padded_for_the_device_only() {
        let img = image(2049);
        let mut dev = FakeDfu::with_firmware(&[1]);
        let c = copy(&dev);
        flash(&mut dev, FLASH_BASE, &img, &c, true, &mut |_| {}).unwrap();
        assert_eq!(&dev.flash[..2049], &img[..]);
        assert_eq!(dev.flash[2049..2052], [0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn read_verified_reads_twice_through_a_path_that_cannot_write() {
        let fw = image(70_000);
        let mut dev = FakeDfu::with_firmware(&fw);
        let mut last = (0, 0);
        let c = read_verified(&mut dev, true, &mut |d, t| last = (d, t)).unwrap();
        assert_eq!(c.base(), FLASH_BASE);
        assert_eq!(&c.bytes()[..fw.len()], &fw[..]);
        assert_eq!(c.bytes().len(), 1024 * 1024);
        assert_eq!(c.trimmed().len(), fw.len(), "the erased tail is cut");
        assert_eq!(
            last,
            (2 * 1024 * 1024, 2 * 1024 * 1024),
            "both reads reported"
        );
        assert_eq!(dev.mutations, 0, "no erase, write or leave");
        assert!(!dev.left, "a read does not restart the radio");
        assert_eq!(dev.flash[..fw.len()], fw[..]);
    }

    #[test]
    fn the_read_only_path_refuses_erase_write_and_leave() {
        let mut dev = FakeDfu::with_firmware(&image(100));
        {
            let mut ro = ReadOnly::new(&mut dev);
            let mut erase = vec![DFUSE_ERASE];
            erase.extend_from_slice(&FLASH_BASE.to_le_bytes());
            assert!(ro.control_out(DFU_DNLOAD, 0, &erase).is_err());
            assert!(ro.control_out(DFU_DNLOAD, 2, &[0; 16]).is_err());
            assert!(ro.control_out(DFU_DNLOAD, 0, &[]).is_err(), "leave");
            assert!(ro.control_out(5, 0, &[]).is_err());
            assert!(ro.control_in(1, 0, 6).is_err());
            // What a read needs passes.
            let mut set = vec![DFUSE_SET_ADDRESS];
            set.extend_from_slice(&FLASH_BASE.to_le_bytes());
            ro.control_out(DFU_DNLOAD, 0, &set).unwrap();
            ro.control_in(DFU_GETSTATUS, 0, 6).unwrap();
            ro.control_out(DFU_ABORT, 0, &[]).unwrap();
        }
        assert_eq!(dev.mutations, 0);
    }

    #[test]
    fn a_zero_or_unstable_flash_is_not_a_copy() {
        let mut blank = FakeDfu::with_firmware(&[]);
        let c = read_verified(&mut blank, true, &mut |_, _| {}).unwrap();
        assert!(c.is_blank(), "a blank flash is a copy the caller can judge");
        let mut zero = FakeDfu::with_firmware(&[]);
        zero.flash.fill(0);
        let e = read_verified(&mut zero, true, &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("zeros"), "{e}");
        // Every upload flips a bit in its first byte, but the two reads agree with each other
        // there, so a stable corruption cannot be told from data: the unstable case is the
        // one a flaky link shows. Flip on the second pass only.
        let mut flaky = FakeDfu::with_firmware(&image(5000));
        flaky.faults.flip_after_reads = Some(512);
        let e = read_verified(&mut flaky, true, &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("differ"), "{e}");
    }

    #[test]
    fn a_device_in_error_is_cleared_first() {
        let mut dev = FakeDfu::with_firmware(&[1]);
        let c = copy(&dev);
        dev.status = 3;
        dev.state = state::ERROR;
        flash(&mut dev, FLASH_BASE, &image(100), &c, true, &mut |_| {}).unwrap();
        assert!(dev.left);
    }

    #[test]
    fn listing_is_empty_under_cargo() {
        if std::env::var("QUADCAM_SERIAL").as_deref() != Ok("real") {
            assert!(list().is_empty());
        }
    }
}
