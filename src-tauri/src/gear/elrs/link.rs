//! The serial side of ExpressLRS (design 6.4): hand a host's USB port over to the ELRS device
//! behind it (the radio's internal module, or the receiver on a flight controller UART), then
//! speak CRSF to it.
//!
//! - **Radio.** The EdgeTX CLI (USB serial set to CLI) stops the pulses and starts a serial
//!   passthrough to the internal module (`serialpassthrough rfmod 0 <baud>`). To flash, it
//!   first holds the module's boot pin while the module powers up.
//! - **Flight controller.** The Betaflight CLI names the UART with the serial receiver
//!   (function bit 64), checks that the receiver protocol is CRSF and the line is not inverted
//!   or half duplex, and starts `serialpassthrough <uart> <baud>`.
//!
//! Both hosts stay in passthrough until the person unplugs the USB cable (the FC) or restarts
//! the radio, so every caller says so in its report.
//!
//! QuadCam learned the command sequences from the ExpressLRS documentation and observation,
//! and copies no ExpressLRS code. The sequences have not run on a real device yet.

use super::crsf::{self, DeviceInfo, Frame, FrameParser, Param};
use crate::gear::edgetx::cli::at_prompt;
use crate::gear::serial::{read_until, Ports, SerialLink, Wait};
use anyhow::{anyhow, bail, Context, Result};
use std::time::{Duration, Instant};

/// What the radio's CLI and the FC's CLI use before the passthrough starts.
pub const CLI_BAUD: u32 = 115_200;
/// The module UART speed of a radio's internal ELRS module.
pub const MODULE_BAUD: u32 = 400_000;
/// The receiver UART speed behind an FC.
pub const RECEIVER_BAUD: u32 = 420_000;
/// The speed `esptool` talks to a module or receiver in its bootloader.
pub const FLASH_BAUD: u32 = 460_800;

/// The Betaflight prompt.
const BF_PROMPT: &[u8] = b"\r\n# ";

/// How long each step waits.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// A prompt counts once the line has been quiet this long.
    pub quiet: Duration,
    /// A CLI line's answer.
    pub command: Duration,
    /// A CRSF reply.
    pub reply: Duration,
    /// How long a ping collects device answers.
    pub ping: Duration,
    /// Pause between module power steps.
    pub settle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            quiet: Duration::from_millis(150),
            command: Duration::from_secs(2),
            reply: Duration::from_millis(500),
            ping: Duration::from_millis(800),
            settle: Duration::from_millis(500),
        }
    }
}

impl Timing {
    pub fn fast() -> Self {
        Self {
            quiet: Duration::from_millis(5),
            command: Duration::from_millis(300),
            reply: Duration::from_millis(100),
            ping: Duration::from_millis(60),
            settle: Duration::ZERO,
        }
    }
}

fn wait(t: &Timing, timeout: Duration) -> Wait {
    Wait {
        timeout,
        quiet: t.quiet,
        extend: false,
    }
}

fn cli_line(
    link: &mut dyn SerialLink,
    t: &Timing,
    line: &str,
    done: impl Fn(&[u8]) -> bool,
) -> Result<String> {
    link.write_all(format!("{line}\n").as_bytes())?;
    let raw = read_until(link, wait(t, t.command), done)
        .with_context(|| format!("No prompt after {line:?}"))?;
    Ok(String::from_utf8_lossy(&raw)
        .replace("\r\n", "\n")
        .replace('\r', "\n"))
}

/// How the radio's module is started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleStart {
    /// Normal firmware: read its parameters.
    Run,
    /// The boot pin held while the module powers up: its serial bootloader answers.
    Bootloader,
}

/// Hands a radio's USB serial port to its internal ELRS module at `baud`. The radio's CLI
/// must be on (`serialPort: VCP` set to CLI in the radio) and the radio plugged in as USB
/// Serial.
pub fn radio_passthrough(
    ports: &dyn Ports,
    port: &str,
    baud: u32,
    start: ModuleStart,
    t: &Timing,
) -> Result<Box<dyn SerialLink>> {
    let mut link = ports.open(port, CLI_BAUD)?;
    link.write_all(b"\n")?;
    read_until(link.as_mut(), wait(t, t.command), at_prompt)
        .map_err(|_| anyhow!(crate::gear::edgetx::cli::NO_PROMPT))?;
    let mut lines = vec!["set pulses 0", "set rfmod 0 power off"];
    if start == ModuleStart::Bootloader {
        lines.push("set rfmod 0 bootpin 1");
    }
    lines.push("set rfmod 0 power on");
    if start == ModuleStart::Bootloader {
        lines.push("set rfmod 0 bootpin 0");
    }
    for l in lines {
        let reply = cli_line(link.as_mut(), t, l, at_prompt)?;
        if reply.to_ascii_lowercase().contains("unknown") || reply.contains("not found") {
            bail!("The radio's CLI does not know {l:?}: {}", reply.trim());
        }
        std::thread::sleep(t.settle);
    }
    link.write_all(format!("serialpassthrough rfmod 0 {baud}\n").as_bytes())?;
    std::thread::sleep(t.settle);
    drop(link);
    ports.open(port, baud)
}

/// What the FC's settings say about the receiver, found while entering passthrough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcReceiver {
    /// The `serial` identifier of the UART with the serial receiver.
    pub uart: String,
}

/// The UART with the serial receiver in `serial` output: a line `serial <id> <functions> ...`
/// whose function mask has bit 64.
pub fn receiver_uart(serial_output: &str) -> Option<String> {
    serial_output.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        (w.next()? == "serial").then_some(())?;
        let id = w.next()?;
        let mask: u32 = w.next()?.parse().ok()?;
        (mask & 64 != 0).then(|| id.to_string())
    })
}

fn bf_value<'a>(reply: &'a str, name: &str) -> Option<&'a str> {
    reply
        .lines()
        .find_map(|l| l.trim().strip_prefix(&format!("{name} = ")))
        .map(|v| v.split_whitespace().next().unwrap_or(""))
}

/// Hands an FC's USB port to the receiver on its serial-receiver UART at `baud`.
pub fn fc_passthrough(
    ports: &dyn Ports,
    port: &str,
    baud: u32,
    t: &Timing,
) -> Result<(Box<dyn SerialLink>, FcReceiver)> {
    let mut link = ports.open(port, CLI_BAUD)?;
    link.write_all(b"#")?;
    read_until(link.as_mut(), wait(t, t.command), |b| {
        b.ends_with(BF_PROMPT)
    })
    .context("No CLI prompt after '#': is this a Betaflight FC?")?;
    let done = |b: &[u8]| b.ends_with(BF_PROMPT);
    let mut problems = Vec::new();
    for (name, ok, hint) in [
        (
            "serialrx_provider",
            &["CRSF", "ELRS"][..],
            "set serialrx_provider = CRSF",
        ),
        (
            "serialrx_inverted",
            &["OFF"][..],
            "set serialrx_inverted = OFF",
        ),
        (
            "serialrx_halfduplex",
            &["OFF", "AUTO"][..],
            "set serialrx_halfduplex = OFF",
        ),
    ] {
        let reply = cli_line(link.as_mut(), t, &format!("get {name}"), done)?;
        let v = bf_value(&reply, name).unwrap_or("?");
        if !ok.contains(&v) {
            problems.push(format!("{name} is {v}; {hint}"));
        }
    }
    if !problems.is_empty() {
        bail!(
            "The FC's receiver settings do not allow a passthrough: {}. Fix them, save, and try again.",
            problems.join("; ")
        );
    }
    let reply = cli_line(link.as_mut(), t, "serial", done)?;
    let uart = receiver_uart(&reply)
        .ok_or_else(|| anyhow!("The FC has no UART with the serial receiver (function 64)."))?;
    link.write_all(format!("serialpassthrough {uart} {baud}\n").as_bytes())?;
    std::thread::sleep(t.settle);
    drop(link);
    Ok((ports.open(port, baud)?, FcReceiver { uart }))
}

/// A CRSF conversation with the ELRS device behind a passthrough.
pub struct CrsfLink {
    link: Box<dyn SerialLink>,
    parser: FrameParser,
    timing: Timing,
}

impl CrsfLink {
    pub fn new(link: Box<dyn SerialLink>, timing: Timing) -> Self {
        Self {
            link,
            parser: FrameParser::new(),
            timing,
        }
    }

    pub fn port(&self) -> &str {
        self.link.port()
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.link.write_all(bytes)
    }

    /// Reads until `f` returns Some or `timeout` passes. Frames `f` ignores are dropped (a live
    /// link streams channel and link-statistics frames all the time).
    fn until<T>(
        &mut self,
        timeout: Duration,
        mut f: impl FnMut(&Frame) -> Option<T>,
    ) -> Result<Option<T>> {
        let end = Instant::now() + timeout;
        loop {
            while let Some(fr) = self.parser.take() {
                if let Some(v) = f(&fr) {
                    return Ok(Some(v));
                }
            }
            let now = Instant::now();
            if now >= end {
                return Ok(None);
            }
            let got = self
                .link
                .read_some((end - now).min(Duration::from_millis(50)))?;
            self.parser.push(&got);
        }
    }

    /// Pings and collects every device that answers within the ping window.
    pub fn ping(&mut self) -> Result<Vec<DeviceInfo>> {
        self.send(&crsf::ping())?;
        let mut found: Vec<DeviceInfo> = Vec::new();
        let _ = self.until(self.timing.ping, |f| {
            if f.kind == crsf::T_DEVICE_INFO {
                if let Ok(i) = crsf::parse_device_info(f) {
                    if !found.iter().any(|d| d.origin == i.origin) {
                        found.push(i);
                    }
                }
            }
            None::<()>
        })?;
        Ok(found)
    }

    /// One parameter entry, its chunks joined.
    pub fn read_entry(&mut self, dest: u8, id: u8) -> Result<Vec<u8>> {
        let mut data: Vec<u8> = Vec::new();
        let mut chunk = 0u8;
        loop {
            let mut got: Option<(u8, Vec<u8>)> = None;
            for _try in 0..3 {
                self.send(&crsf::read_request(dest, id, chunk))?;
                got = self.until(self.timing.reply, |f| {
                    let e = crsf::ext(f).filter(|_| f.kind == crsf::T_ENTRY)?;
                    (e.origin == dest && e.body.len() >= 2 && e.body[0] == id)
                        .then(|| (e.body[1], e.body[2..].to_vec()))
                })?;
                if got.is_some() {
                    break;
                }
            }
            let (left, bytes) =
                got.ok_or_else(|| anyhow!("Parameter {id} did not answer (chunk {chunk})."))?;
            data.extend_from_slice(&bytes);
            if left == 0 {
                return Ok(data);
            }
            chunk = chunk
                .checked_add(1)
                .context("parameter has too many chunks")?;
        }
    }

    /// Every parameter of `info`'s device, in id order. A parameter that does not parse is
    /// kept as `Unread` so the list stays whole.
    pub fn read_params(&mut self, info: &DeviceInfo) -> Result<Vec<Param>> {
        let mut out = Vec::new();
        for id in 1..=info.params {
            let data = self.read_entry(info.origin, id)?;
            out.push(crsf::parse_param(id, &data).unwrap_or(Param {
                id,
                parent: 0,
                hidden: true,
                name: format!("(parameter {id})"),
                value: crsf::Value::Unread(255),
            }));
        }
        Ok(out)
    }

    /// Writes an encoded value into a parameter.
    pub fn write_value(&mut self, dest: u8, id: u8, value: &[u8]) -> Result<()> {
        self.send(&crsf::write_request(dest, id, value))
    }

    /// Asks a receiver to restart into its bootloader, and returns what it printed.
    pub fn enter_bootloader(&mut self) -> Result<String> {
        self.send(&[0x07, 0x07, 0x12, 0x20])?;
        self.send(&[0x55; 32])?;
        std::thread::sleep(self.timing.settle);
        self.send(&crsf::bootloader_request())?;
        let mut text = Vec::new();
        let end = Instant::now() + self.timing.reply;
        while Instant::now() < end {
            let got = self.link.read_some(Duration::from_millis(30))?;
            text.extend_from_slice(&got);
        }
        Ok(String::from_utf8_lossy(&text).trim().to_string())
    }
}
