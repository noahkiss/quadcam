//! Simulators for tests and the mock core: an ExpressLRS device that answers CRSF, and the
//! host (an EdgeTX radio or a Betaflight FC) whose CLI hands its port over to it.
//!
//! A `FakeHost` is one device on one port. Its CLI answers the few commands the passthrough
//! needs; after `serialpassthrough` the port speaks CRSF to the `FakeElrs` behind it. Clones
//! share the state, so a test reads what was written after the job. An FC host built `with_msp`
//! answers the MSP identity through a `bf::fake::FakeFc`, as a real FC does before its CLI.

use super::crsf::{self, Frame, FrameParser, Param, Value};
use crate::gear::bf::fake::FakeFc;
use crate::gear::serial::{FakePorts, PortInfo, SerialLink};
use anyhow::Result;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What a simulated device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tx,
    Rx,
}

impl Role {
    pub fn address(self) -> u8 {
        match self {
            Role::Tx => crsf::ADDR_TX,
            Role::Rx => crsf::ADDR_RX,
        }
    }
}

/// The bytes of an entry as a device sends them (the inverse of `crsf::parse_param`).
pub fn encode_param(p: &Param) -> Vec<u8> {
    let (code, tail): (u8, Vec<u8>) = match &p.value {
        Value::Number {
            value,
            min,
            max,
            size,
            signed,
        } => {
            let code = match (size, signed) {
                (1, false) => 0,
                (1, true) => 1,
                (_, false) => 2,
                (_, true) => 3,
            };
            let mut t = Vec::new();
            for n in [value, min, max] {
                if *size == 1 {
                    t.push(*n as u8);
                } else {
                    t.extend_from_slice(&(*n as i16).to_be_bytes());
                }
            }
            (code, t)
        }
        Value::Select { options, index } => {
            let mut t = options.join(";").into_bytes();
            t.push(0);
            t.extend_from_slice(&[*index, 0, options.len().saturating_sub(1) as u8, 0, 0]);
            (9, t)
        }
        Value::Text(s) => {
            let mut t = s.clone().into_bytes();
            t.push(0);
            (10, t)
        }
        Value::Info(s) => {
            let mut t = s.clone().into_bytes();
            t.push(0);
            (12, t)
        }
        Value::Folder => (11, vec![0xFF]),
        Value::Command => (13, vec![0, 0, 0]),
        Value::Unread(c) => (*c, Vec::new()),
    };
    let mut out = vec![p.parent, code | if p.hidden { 0x80 } else { 0 }];
    out.extend_from_slice(p.name.as_bytes());
    out.push(0);
    out.extend_from_slice(&tail);
    out
}

fn select(id: u8, parent: u8, name: &str, options: &[&str], index: u8) -> Param {
    Param {
        id,
        parent,
        hidden: false,
        name: name.into(),
        value: Value::Select {
            options: options.iter().map(|s| s.to_string()).collect(),
            index,
        },
    }
}

fn folder(id: u8, name: &str) -> Param {
    Param {
        id,
        parent: 0,
        hidden: false,
        name: name.into(),
        value: Value::Folder,
    }
}

fn info(id: u8, parent: u8, name: &str, text: &str) -> Param {
    Param {
        id,
        parent,
        hidden: false,
        name: name.into(),
        value: Value::Info(text.into()),
    }
}

struct DeviceState {
    role: Role,
    name: String,
    params: Vec<Param>,
    /// How many bytes of an entry one chunk carries.
    chunk: usize,
    /// Every write: field id and the value bytes.
    writes: Vec<(u8, Vec<u8>)>,
    /// A parameter that never answers (a dead device).
    mute: bool,
    bootloader_requests: u32,
    /// What the bootloader prints when asked (the receiver's target name).
    bootloader_text: String,
    parser: FrameParser,
}

/// An ExpressLRS device.
#[derive(Clone)]
pub struct FakeElrs {
    st: Arc<Mutex<DeviceState>>,
}

impl FakeElrs {
    /// A transmitter module with the usual Lua parameters.
    pub fn tx(name: &str, version: &str) -> FakeElrs {
        let params = vec![
            select(
                1,
                0,
                "Packet Rate",
                &[
                    "50Hz(-115dBm)",
                    "150Hz(-112dBm)",
                    "250Hz(-108dBm)",
                    "500Hz(-105dBm)",
                ],
                1,
            ),
            select(
                2,
                0,
                "Telem Ratio",
                &[
                    "Std", "Off", "1:128", "1:64", "1:32", "1:16", "1:8", "1:4", "1:2",
                ],
                0,
            ),
            select(3, 0, "Switch Mode", &["Hybrid", "Wide"], 1),
            select(4, 0, "Model Match", &["Off", "On"], 0),
            folder(5, "TX Power"),
            select(
                6,
                5,
                "Max Power",
                &["10", "25", "50", "100", "250", "500", "1000"],
                4,
            ),
            select(
                7,
                5,
                "Dynamic",
                &["Off", "On", "AUX9", "AUX10", "AUX11", "AUX12"],
                0,
            ),
            Param {
                id: 8,
                parent: 0,
                hidden: false,
                name: "Bind".into(),
                value: Value::Command,
            },
            info(9, 0, "Version", &format!("ELRS {version} (fixture)")),
        ];
        Self::build(Role::Tx, name, params)
    }

    /// A receiver with the usual Lua parameters.
    pub fn rx(name: &str, version: &str) -> FakeElrs {
        let params = vec![
            select(1, 0, "Protocol", &["CRSF", "Inverted CRSF", "SBUS"], 0),
            select(2, 0, "Model Match", &["Off", "On"], 0),
            Param {
                id: 3,
                parent: 0,
                hidden: false,
                name: "Bind".into(),
                value: Value::Command,
            },
            info(4, 0, "Version", &format!("ELRS {version} (fixture)")),
        ];
        Self::build(Role::Rx, name, params)
    }

    fn build(role: Role, name: &str, params: Vec<Param>) -> FakeElrs {
        FakeElrs {
            st: Arc::new(Mutex::new(DeviceState {
                role,
                name: name.into(),
                params,
                chunk: 24,
                writes: Vec::new(),
                mute: false,
                bootloader_requests: 0,
                bootloader_text: name.to_ascii_uppercase(),
                parser: FrameParser::new(),
            })),
        }
    }

    /// The device answers nothing.
    pub fn mute(self) -> FakeElrs {
        self.st.lock().unwrap().mute = true;
        self
    }

    /// Replaces a parameter list.
    pub fn with_params(self, params: Vec<Param>) -> FakeElrs {
        self.st.lock().unwrap().params = params;
        self
    }

    /// What the receiver prints after the bootloader request.
    pub fn with_bootloader_text(self, text: &str) -> FakeElrs {
        self.st.lock().unwrap().bootloader_text = text.into();
        self
    }

    /// The parameter named `name`, as it is now.
    pub fn param(&self, name: &str) -> Option<Param> {
        self.st
            .lock()
            .unwrap()
            .params
            .iter()
            .find(|p| p.name == name)
            .cloned()
    }

    /// Moves a choice parameter, as the person would on the device.
    pub fn set_index(&self, id: u8, to: u8) {
        let mut s = self.st.lock().unwrap();
        if let Some(Param {
            value: Value::Select { index, .. },
            ..
        }) = s.params.iter_mut().find(|p| p.id == id)
        {
            *index = to;
        }
    }

    /// Every write received: field id and bytes.
    pub fn writes(&self) -> Vec<(u8, Vec<u8>)> {
        self.st.lock().unwrap().writes.clone()
    }

    pub fn bootloader_requests(&self) -> u32 {
        self.st.lock().unwrap().bootloader_requests
    }

    fn feed(&self, data: &[u8]) -> Vec<u8> {
        let mut s = self.st.lock().unwrap();
        s.parser.push(data);
        let mut out = Vec::new();
        while let Some(f) = s.parser.take() {
            out.extend(Self::answer(&mut s, &f));
        }
        out
    }

    fn answer(s: &mut DeviceState, f: &Frame) -> Vec<u8> {
        if f.kind == crsf::T_COMMAND {
            if f.payload == b"bl" {
                s.bootloader_requests += 1;
                let mut t = s.bootloader_text.clone().into_bytes();
                t.push(b'\n');
                return t;
            }
            return Vec::new();
        }
        if s.mute {
            return Vec::new();
        }
        let origin = s.role.address();
        let Some(e) = crsf::ext(f) else {
            return Vec::new();
        };
        match f.kind {
            crsf::T_PING => {
                let mut body = s.name.clone().into_bytes();
                body.push(0);
                body.extend_from_slice(b"ELRS");
                body.extend_from_slice(&0u32.to_be_bytes());
                body.extend_from_slice(&0x0004_0100u32.to_be_bytes());
                body.push(s.params.len() as u8);
                body.push(0);
                crsf::encode_ext(
                    crsf::ADDR_RADIO,
                    crsf::T_DEVICE_INFO,
                    crsf::ADDR_RADIO,
                    origin,
                    &body,
                )
            }
            crsf::T_READ if e.body.len() >= 2 && (e.dest == origin || e.dest == 0) => {
                let (id, chunk) = (e.body[0], e.body[1] as usize);
                let Some(p) = s.params.iter().find(|p| p.id == id) else {
                    return Vec::new();
                };
                let data = encode_param(p);
                let parts: Vec<&[u8]> = data.chunks(s.chunk).collect();
                let Some(part) = parts.get(chunk) else {
                    return Vec::new();
                };
                let left = (parts.len() - 1 - chunk) as u8;
                let mut body = vec![id, left];
                body.extend_from_slice(part);
                crsf::encode_ext(
                    crsf::ADDR_RADIO,
                    crsf::T_ENTRY,
                    crsf::ADDR_RADIO,
                    origin,
                    &body,
                )
            }
            crsf::T_WRITE if e.body.len() >= 2 && e.dest == origin => {
                let id = e.body[0];
                let val = e.body[1..].to_vec();
                s.writes.push((id, val.clone()));
                if let Some(p) = s.params.iter_mut().find(|p| p.id == id) {
                    match &mut p.value {
                        Value::Select { index, options } if (val[0] as usize) < options.len() => {
                            *index = val[0]
                        }
                        Value::Number {
                            value,
                            size,
                            signed,
                            ..
                        } => {
                            *value = match (*size, *signed, val.as_slice()) {
                                (1, false, [a, ..]) => *a as i64,
                                (1, true, [a, ..]) => *a as i8 as i64,
                                (_, _, [a, b, ..]) => i16::from_be_bytes([*a, *b]) as i64,
                                _ => *value,
                            }
                        }
                        _ => {}
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// Which CLI the host speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    Radio,
    Fc,
}

struct HostState {
    kind: HostKind,
    device: FakeElrs,
    /// The FC's `serial` listing and receiver settings.
    serialrx_provider: String,
    inverted: String,
    halfduplex: String,
    uart_line: Option<String>,
    /// Set once `serialpassthrough` ran, with its baud.
    passthrough: Option<u32>,
    /// Every CLI line received.
    log: Vec<String>,
    opens: Vec<u32>,
    /// The radio's module is in bootloader mode (boot pin held at power-on).
    bootpin: bool,
    boot_mode: bool,
    /// The board the radio's `ver` names.
    board: String,
    /// What answers MSP on the FC's port before its CLI starts.
    msp: Option<FakeFc>,
}

/// A radio or an FC on a serial port, with an ELRS device behind it.
#[derive(Clone)]
pub struct FakeHost {
    st: Arc<Mutex<HostState>>,
}

impl FakeHost {
    pub fn radio(device: FakeElrs) -> FakeHost {
        Self::new(HostKind::Radio, device)
    }

    /// An FC whose UART 2 carries the serial receiver.
    pub fn fc(device: FakeElrs) -> FakeHost {
        let h = Self::new(HostKind::Fc, device);
        h.st.lock().unwrap().uart_line = Some("serial 2 64 115200 57600 0 115200".into());
        h
    }

    fn new(kind: HostKind, device: FakeElrs) -> FakeHost {
        FakeHost {
            st: Arc::new(Mutex::new(HostState {
                kind,
                device,
                serialrx_provider: "CRSF".into(),
                inverted: "OFF".into(),
                halfduplex: "OFF".into(),
                uart_line: None,
                passthrough: None,
                log: Vec::new(),
                opens: Vec::new(),
                bootpin: false,
                boot_mode: false,
                board: "pocket".into(),
                msp: None,
            })),
        }
    }

    pub fn with_receiver_settings(
        self,
        provider: &str,
        inverted: &str,
        halfduplex: &str,
    ) -> FakeHost {
        {
            let mut s = self.st.lock().unwrap();
            s.serialrx_provider = provider.into();
            s.inverted = inverted.into();
            s.halfduplex = halfduplex.into();
        }
        self
    }

    /// The FC answers the MSP identity of `fc` (its MCU id) until a passthrough starts.
    pub fn with_msp(self, fc: FakeFc) -> FakeHost {
        self.st.lock().unwrap().msp = Some(fc);
        self
    }

    /// The board the radio's `ver` names (default `pocket`).
    pub fn with_board(self, board: &str) -> FakeHost {
        self.st.lock().unwrap().board = board.into();
        self
    }

    pub fn without_receiver_uart(self) -> FakeHost {
        self.st.lock().unwrap().uart_line = None;
        self
    }

    /// The person restarts the radio, or unplugs the FC and plugs it in: the CLI is back.
    pub fn restart(&self) {
        let mut s = self.st.lock().unwrap();
        s.passthrough = None;
        s.bootpin = false;
        s.boot_mode = false;
    }

    pub fn device(&self) -> FakeElrs {
        self.st.lock().unwrap().device.clone()
    }

    /// Puts another ELRS device behind the host (a receiver swapped on the quad).
    pub fn replace_device(&self, device: FakeElrs) {
        self.st.lock().unwrap().device = device;
    }

    pub fn log(&self) -> Vec<String> {
        self.st.lock().unwrap().log.clone()
    }

    /// The baud of each open, in order.
    pub fn opens(&self) -> Vec<u32> {
        self.st.lock().unwrap().opens.clone()
    }

    pub fn passthrough_baud(&self) -> Option<u32> {
        self.st.lock().unwrap().passthrough
    }

    /// True when the radio's module was started with the boot pin held.
    pub fn module_in_bootloader(&self) -> bool {
        self.st.lock().unwrap().boot_mode
    }

    /// Ports with this host on `port`.
    pub fn ports(&self, port: &str) -> FakePorts {
        let host = self.clone();
        let (vid, pid, manufacturer, product) = match self.st.lock().unwrap().kind {
            HostKind::Radio => (0x0483, 0x5740, "OpenTX", "Pocket Serial Port"),
            HostKind::Fc => (0x0483, 0x5740, "Betaflight", "STM32 Virtual COM Port"),
        };
        let info = PortInfo {
            port: port.into(),
            vid,
            pid,
            serial_number: Some("00000000FAKE".into()),
            manufacturer: Some(manufacturer.into()),
            product: Some(product.into()),
        };
        FakePorts::new(vec![info]).with_opener(move |port, baud| {
            host.st.lock().unwrap().opens.push(baud);
            Ok(Box::new(HostLink {
                port: port.to_string(),
                host: host.clone(),
                line: Vec::new(),
                out: VecDeque::new(),
                msp: None,
            }))
        })
    }

    fn cli(&self, line: &str) -> String {
        let mut s = self.st.lock().unwrap();
        s.log.push(line.to_string());
        let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
        match s.kind {
            HostKind::Radio => match verb {
                "set" => {
                    match arg {
                        "rfmod 0 bootpin 1" => s.bootpin = true,
                        "rfmod 0 bootpin 0" => s.bootpin = false,
                        "rfmod 0 power on" => s.boot_mode = s.bootpin,
                        _ => {}
                    }
                    format!("{line}\r\nset: {arg}\r\n> ")
                }
                "serialpassthrough" => {
                    let baud = arg.split_whitespace().last().and_then(|b| b.parse().ok());
                    s.passthrough = baud;
                    String::new()
                }
                "ver" => format!(
                    "{line}\r\nboard: {b}\r\nvers: edgetx-{b}-2.11.0\r\n> ",
                    b = s.board
                ),
                _ => format!("{line}\r\nUnknown command: {verb}\r\n> "),
            },
            HostKind::Fc => match verb {
                "get" => {
                    let v = match arg {
                        "serialrx_provider" => s.serialrx_provider.clone(),
                        "serialrx_inverted" => s.inverted.clone(),
                        "serialrx_halfduplex" => s.halfduplex.clone(),
                        _ => "?".into(),
                    };
                    format!("{line}\r\n{arg} = {v}\r\n\r\n# ")
                }
                "serial" => {
                    let l = s.uart_line.clone().unwrap_or_default();
                    format!("{line}\r\n{l}\r\n\r\n# ")
                }
                "serialpassthrough" => {
                    let baud = arg.split_whitespace().last().and_then(|b| b.parse().ok());
                    s.passthrough = baud;
                    String::new()
                }
                _ => format!("{line}\r\n###ERROR IN CLI### Invalid\r\n\r\n# "),
            },
        }
    }
}

struct HostLink {
    port: String,
    host: FakeHost,
    line: Vec<u8>,
    out: VecDeque<u8>,
    /// The `FakeFc` link MSP frames go to, opened at the first one.
    msp: Option<Box<dyn SerialLink>>,
}

impl SerialLink for HostLink {
    fn port(&self) -> &str {
        &self.port
    }

    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        let passthrough = self.host.st.lock().unwrap().passthrough.is_some();
        if passthrough {
            let device = self.host.device();
            self.out.extend(device.feed(data));
            return Ok(());
        }
        // An MSP frame (`$M<`, `$X<`) on an FC that answers MSP.
        if data.first() == Some(&b'$') && self.line.is_empty() {
            let fc = self.host.st.lock().unwrap().msp.clone();
            if let Some(fc) = fc {
                if self.msp.is_none() {
                    self.msp = Some(fc.open(&self.port)?);
                }
                return self.msp.as_mut().expect("opened above").write_all(data);
            }
        }
        for &b in data {
            match b {
                b'\n' => {
                    let text = String::from_utf8_lossy(&self.line).trim().to_string();
                    self.line.clear();
                    let reply = if text.is_empty() {
                        match self.host.st.lock().unwrap().kind {
                            HostKind::Radio => "\r\n> ".to_string(),
                            HostKind::Fc => "\r\n# ".to_string(),
                        }
                    } else {
                        self.host.cli(&text)
                    };
                    self.out.extend(reply.into_bytes());
                }
                b'#' if self.line.is_empty() => {
                    let kind = self.host.st.lock().unwrap().kind;
                    if kind == HostKind::Fc {
                        self.out.extend(b"\r\n# ".iter());
                    }
                }
                b'\r' => {}
                b => self.line.push(b),
            }
        }
        Ok(())
    }

    fn read_some(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        if let Some(m) = self.msp.as_mut() {
            let got = m.read_some(timeout)?;
            if !got.is_empty() {
                return Ok(got);
            }
        }
        if self.out.is_empty() {
            std::thread::yield_now();
            return Ok(Vec::new());
        }
        Ok(self.out.drain(..).collect())
    }
}
