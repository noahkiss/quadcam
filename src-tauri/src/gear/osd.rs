//! Betaflight OSD layouts: read the element positions from a `dump all`, a `diff all` or a
//! file of CLI lines, draw each OSD profile on its grid, and check it (design 7.3).
//!
//! **Position value** (`set osd_<element>_pos = N`):
//!
//! | Bits | Holds |
//! |---|---|
//! | 0-4 | x, low 5 bits |
//! | 5-9 | y (0-31) |
//! | 10 | x, bit 5 (so x runs 0-63, for HD grids) |
//! | 11, 12, 13 | shown in OSD profile 1, 2, 3 |
//! | 14-15 | the element's variant |
//!
//! An element has ONE position, shared by every profile; a profile only turns it on or off.
//!
//! **Grids:** analog NTSC 30 x 13, PAL 30 x 16, HD (MSP DisplayPort) 53 x 20, or any W x H.
//!
//! **Element table:** QuadCam's own. Names come from the `osd_*_pos` settings a dump lists;
//! each element's sample text and width come from what the element shows on screen
//! (`B4.20V`, `L2:99`, `T00:00`), with letters standing in for the font's symbols. An element
//! the table does not know draws 5 wide and is listed in `unknown`.
//!
//! This module reads text only: it opens no port and no file. `Core::gear_osd` reads the
//! files.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, BTreeSet};

/// Profile flag bits: profile 1 is bit 11, 2 is bit 12, 3 is bit 13.
pub const PROFILE_BITS: [u16; 3] = [1 << 11, 1 << 12, 1 << 13];
/// The OSD profiles Betaflight has.
pub const PROFILES: [u8; 3] = [1, 2, 3];
/// The horizon line moves across columns x-4..x+4 ...
pub const HORIZON_HALF: i32 = 4;
/// ... and rows y..y+9 as the quad pitches; its level line sits on row y+4.
pub const HORIZON_ROWS: i32 = 10;
/// A stick overlay is 7 columns by 5 rows.
pub const STICK: (i32, i32) = (7, 5);
/// Width of an element the table does not know.
pub const UNKNOWN_WIDTH: i32 = 5;

/// A decoded position value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Pos {
    pub x: u8,
    pub y: u8,
    /// Bit 0 = profile 1, bit 1 = profile 2, bit 2 = profile 3.
    pub profiles: u8,
    pub variant: u8,
}

impl Pos {
    /// Reads a `_pos` value.
    pub fn decode(v: u16) -> Pos {
        Pos {
            x: ((v & 31) | (((v >> 10) & 1) << 5)) as u8,
            y: ((v >> 5) & 31) as u8,
            profiles: ((v >> 11) & 7) as u8,
            variant: ((v >> 14) & 3) as u8,
        }
    }

    /// The `_pos` value. x above 63, y above 31, profiles above 7 or a variant above 3
    /// do not fit and are cut to their bits.
    pub fn encode(&self) -> u16 {
        let x = self.x as u16;
        (x & 31)
            | (((x >> 5) & 1) << 10)
            | (((self.y as u16) & 31) << 5)
            | (((self.profiles as u16) & 7) << 11)
            | (((self.variant as u16) & 3) << 14)
    }

    /// True when the element shows in OSD profile `p` (1-3).
    pub fn in_profile(&self, p: u8) -> bool {
        (1..=3).contains(&p) && self.profiles & (1 << (p - 1)) != 0
    }

    /// The profiles (1-3) it shows in.
    pub fn profile_list(&self) -> Vec<u8> {
        PROFILES
            .into_iter()
            .filter(|p| self.in_profile(*p))
            .collect()
    }
}

/// How an element draws.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Draw {
    /// One row: this sample, padded to this width.
    Text(&'static str, i32),
    /// One row: a setting's value in capitals (the craft name), or this text when empty.
    Setting(&'static str, &'static str),
    /// The artificial horizon.
    Horizon,
    /// A stick overlay.
    Stick,
    /// The camera frame: a box sized by `osd_camera_frame_width` and `_height`.
    Frame,
    /// The horizon sidebars: drawn around the screen centre, not at the position. Listed,
    /// not drawn.
    Sidebars,
}

/// QuadCam's OSD element table: setting name (without `osd_` and `_pos`), what the element
/// shows, and how it draws.
const ELEMENTS: &[(&str, &str, Draw)] = &[
    ("vbat", "Battery voltage", Draw::Text("B4.20V", 6)),
    (
        "avg_cell_voltage",
        "Average cell voltage",
        Draw::Text("B4.20V", 6),
    ),
    ("rssi", "RSSI", Draw::Text("R99", 3)),
    ("link_quality", "Link quality", Draw::Text("L2:99", 5)),
    (
        "link_tx_power",
        "Transmitter power",
        Draw::Text("R250MW", 6),
    ),
    ("rssi_dbm", "RSSI (dBm)", Draw::Text("R-67", 5)),
    ("rsnr", "Signal-to-noise ratio", Draw::Text("R  6", 4)),
    ("tim_1", "Timer 1", Draw::Text("T00:00", 6)),
    ("tim_2", "Timer 2", Draw::Text("T00:00", 6)),
    (
        "remaining_time_estimate",
        "Flight time left",
        Draw::Text("E00:00", 6),
    ),
    ("flymode", "Flight mode", Draw::Text("ACRO", 4)),
    ("anti_gravity", "Anti gravity", Draw::Text("AG", 2)),
    ("g_force", "G force", Draw::Text("1.0G", 4)),
    ("throttle", "Throttle", Draw::Text("T 45", 4)),
    ("vtx_channel", "VTX channel", Draw::Text("R:8:200", 7)),
    ("crosshairs", "Crosshairs", Draw::Text("-+-", 3)),
    ("ah_sbar", "Horizon sidebars", Draw::Sidebars),
    ("ah", "Artificial horizon", Draw::Horizon),
    ("current", "Current draw", Draw::Text("  2.50A", 7)),
    ("mah_drawn", "Capacity used", Draw::Text("  45M", 5)),
    ("wh_drawn", "Energy used", Draw::Text("0.00WH", 6)),
    ("motor_diag", "Motor diagnostics", Draw::Text("||||", 4)),
    (
        "craft_name",
        "Craft name",
        Draw::Setting("craft_name", "CRAFT_NAME"),
    ),
    (
        "pilot_name",
        "Pilot name",
        Draw::Setting("pilot_name", "PILOT_NAME"),
    ),
    ("gps_speed", "GPS speed", Draw::Text("  0K", 4)),
    ("gps_lon", "GPS longitude", Draw::Text("O  0.0000000", 12)),
    ("gps_lat", "GPS latitude", Draw::Text("A  0.0000000", 12)),
    ("gps_sats", "GPS satellites", Draw::Text("S 0", 4)),
    ("home_dir", "Home direction", Draw::Text("H", 1)),
    ("home_dist", "Home distance", Draw::Text("H   0", 5)),
    ("flight_dist", "Flight distance", Draw::Text("D   0", 5)),
    ("compass_bar", "Compass bar", Draw::Text("-+-N-+-E-", 9)),
    ("altitude", "Altitude", Draw::Text("A 0.0", 6)),
    ("pid_roll", "Roll PIDs", Draw::Text("ROL  45  80  20", 15)),
    ("pid_pitch", "Pitch PIDs", Draw::Text("PIT  47  84  22", 15)),
    ("pid_yaw", "Yaw PIDs", Draw::Text("YAW  45  80   0", 15)),
    (
        "debug",
        "Debug values",
        Draw::Text("DBG     0     0     0     0", 27),
    ),
    (
        "debug2",
        "Debug values 2",
        Draw::Text("DBG     0     0     0     0", 27),
    ),
    ("power", "Power", Draw::Text("  0W", 4)),
    (
        "pidrate_profile",
        "PID and rate profile",
        Draw::Text("1-1", 3),
    ),
    ("warnings", "Warnings", Draw::Text("LOW BATTERY", 12)),
    ("pit_ang", "Pitch angle", Draw::Text("P 00.0", 6)),
    ("rol_ang", "Roll angle", Draw::Text("R 00.0", 6)),
    (
        "battery_usage",
        "Battery used bar",
        Draw::Text("[========]", 10),
    ),
    ("disarmed", "Disarmed", Draw::Text("DISARMED", 8)),
    ("nheading", "Heading", Draw::Text("H000", 4)),
    ("up_down_reference", "Up/down reference", Draw::Text("U", 1)),
    ("ready_mode", "Ready mode", Draw::Text("READY", 5)),
    ("nvario", "Vertical speed", Draw::Text("V 0.0", 5)),
    ("esc_tmp", "ESC temperature", Draw::Text(" 25C", 4)),
    ("esc_rpm", "ESC RPM", Draw::Text("    0", 5)),
    ("esc_rpm_freq", "ESC RPM frequency", Draw::Text("   0HZ", 6)),
    (
        "rtc_date_time",
        "Date and time",
        Draw::Text("2026-01-01 00:00:00", 19),
    ),
    (
        "adjustment_range",
        "Adjustment range",
        Draw::Text("PITCH RATE 0", 12),
    ),
    ("flip_arrow", "Flip arrow", Draw::Text("^", 1)),
    ("core_temp", "Core temperature", Draw::Text(" 45C", 4)),
    ("log_status", "Blackbox log status", Draw::Text("BB 0", 4)),
    ("stick_overlay_left", "Left stick overlay", Draw::Stick),
    ("stick_overlay_right", "Right stick overlay", Draw::Stick),
    (
        "rate_profile_name",
        "Rate profile name",
        Draw::Text("RATES", 5),
    ),
    (
        "pid_profile_name",
        "PID profile name",
        Draw::Text("PIDS", 8),
    ),
    (
        "battery_profile_name",
        "Battery profile name",
        Draw::Text("BATTERY", 8),
    ),
    ("profile_name", "OSD profile name", Draw::Text("OSD", 8)),
    ("rcchannels", "RC channels", Draw::Text(" 1500", 5)),
    ("camera_frame", "Camera frame", Draw::Frame),
    ("efficiency", "Efficiency", Draw::Text("  0M/K", 6)),
    ("total_flights", "Total flights", Draw::Text("#   0", 5)),
    ("aux", "AUX channel value", Draw::Text("A  0", 4)),
    (
        "custom_serial_text",
        "Custom serial text",
        Draw::Text("TEXT", 10),
    ),
    (
        "sys_goggle_voltage",
        "Goggle voltage",
        Draw::Text("G 0.0V", 6),
    ),
    ("sys_vtx_voltage", "VTX voltage", Draw::Text("V 0.0V", 6)),
    ("sys_bitrate", "Video bitrate", Draw::Text("  0MB", 5)),
    ("sys_delay", "Video delay", Draw::Text("  0MS", 5)),
    ("sys_distance", "Video link distance", Draw::Text("  0M", 4)),
    ("sys_lq", "Video link quality", Draw::Text("L  0", 4)),
    ("sys_goggle_dvr", "Goggle DVR", Draw::Text("DVR", 3)),
    ("sys_vtx_dvr", "VTX DVR", Draw::Text("DVR", 3)),
    (
        "sys_warnings",
        "Video system warnings",
        Draw::Text("WARNING", 12),
    ),
    ("sys_vtx_temp", "VTX temperature", Draw::Text(" 45C", 4)),
    ("sys_fan_speed", "Fan speed", Draw::Text("F0", 2)),
];

fn lookup(name: &str) -> Option<(&'static str, Draw)> {
    ELEMENTS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, label, d)| (*label, *d))
}

/// What the files say about the OSD, after reading them in order (a later line wins).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OsdConfig {
    /// `osd_<name>_pos` values by name.
    pub positions: BTreeMap<String, u16>,
    /// `osd_profile_N_name` by profile.
    pub profile_names: BTreeMap<u8, String>,
    /// `osd_profile`, the profile in use.
    pub active: Option<u8>,
    /// `vcd_video_system`, in capitals.
    pub video_system: Option<String>,
    /// `osd_displayport_device`, in capitals.
    pub displayport: Option<String>,
    /// `osd_camera_frame_width` and `_height`.
    pub camera_frame: (Option<i32>, Option<i32>),
    /// `craft_name` and `pilot_name`.
    pub names: BTreeMap<String, String>,
    /// The `# Betaflight / ...` version line.
    pub firmware: Option<String>,
    /// True when a file was a `dump`: every element is listed.
    pub complete: bool,
}

/// The kind of text a file holds, from its first command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// `dump` or `dump all`: every setting.
    Dump,
    /// `diff` or `diff all`: only settings changed from the firmware's defaults.
    Diff,
    /// Other CLI lines (an apply file).
    Lines,
}

/// Which kind of text this is.
pub fn source_kind(text: &str) -> SourceKind {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let cmd = first.trim_start_matches('#').trim();
    if cmd == "dump" || cmd.starts_with("dump ") {
        SourceKind::Dump
    } else if cmd == "diff" || cmd.starts_with("diff ") {
        SourceKind::Diff
    } else {
        SourceKind::Lines
    }
}

impl OsdConfig {
    /// Reads one more file's text on top of what is read so far.
    pub fn read(&mut self, text: &str) -> SourceKind {
        let kind = source_kind(text);
        if kind == SourceKind::Dump {
            self.complete = true;
        }
        for raw in text.lines() {
            let line = raw.trim();
            if let Some(v) = line.strip_prefix("# Betaflight") {
                self.firmware = Some(format!("Betaflight{}", v.trim_end()));
                continue;
            }
            let Some(rest) = line.strip_prefix("set ") else {
                continue;
            };
            let Some((name, value)) = rest.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if let Some(el) = name
                .strip_prefix("osd_")
                .and_then(|n| n.strip_suffix("_pos"))
            {
                if let Ok(v) = value.parse::<u16>() {
                    self.positions.insert(el.to_string(), v);
                }
            } else if let Some(p) = name
                .strip_prefix("osd_profile_")
                .and_then(|n| n.strip_suffix("_name"))
                .and_then(|n| n.parse::<u8>().ok())
            {
                self.profile_names.insert(p, value.to_string());
            } else {
                match name {
                    "osd_profile" => self.active = value.parse().ok(),
                    "vcd_video_system" => self.video_system = Some(value.to_uppercase()),
                    "osd_displayport_device" => self.displayport = Some(value.to_uppercase()),
                    "osd_camera_frame_width" => self.camera_frame.0 = value.parse().ok(),
                    "osd_camera_frame_height" => self.camera_frame.1 = value.parse().ok(),
                    "craft_name" | "pilot_name" => {
                        self.names.insert(name.to_string(), value.to_string());
                    }
                    _ => {}
                }
            }
        }
        kind
    }

    /// Reads several files in order.
    pub fn parse<'a>(texts: impl IntoIterator<Item = &'a str>) -> OsdConfig {
        let mut c = OsdConfig::default();
        for t in texts {
            c.read(t);
        }
        c
    }
}

/// The screen the OSD draws on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Grid {
    /// `NTSC`, `PAL`, `HD`, or `WxH` for a given size.
    pub name: String,
    pub width: u8,
    pub height: u8,
}

impl Grid {
    pub fn ntsc() -> Grid {
        Grid::named("NTSC", 30, 13)
    }
    pub fn pal() -> Grid {
        Grid::named("PAL", 30, 16)
    }
    pub fn hd() -> Grid {
        Grid::named("HD", 53, 20)
    }
    fn named(name: &str, width: u8, height: u8) -> Grid {
        Grid {
            name: name.into(),
            width,
            height,
        }
    }

    /// `NTSC`, `PAL`, `HD` (any case), or `WxH` with W 1-64 and H 1-32.
    pub fn parse(s: &str) -> Result<Grid> {
        let up = s.trim().to_uppercase();
        match up.as_str() {
            "NTSC" => return Ok(Grid::ntsc()),
            "PAL" => return Ok(Grid::pal()),
            "HD" => return Ok(Grid::hd()),
            _ => {}
        }
        if let Some((w, h)) = up.split_once('X') {
            if let (Ok(w), Ok(h)) = (w.trim().parse::<u8>(), h.trim().parse::<u8>()) {
                if (1..=64).contains(&w) && (1..=32).contains(&h) {
                    return Ok(Grid::named(&format!("{w}x{h}"), w, h));
                }
            }
        }
        bail!("{s:?} is not a grid: use NTSC, PAL, HD or WxH (W up to 64, H up to 32)")
    }
}

/// One element in the files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct OsdElement {
    /// The setting name without `osd_` and `_pos` (`vbat`, `link_quality`).
    pub name: String,
    /// What it shows ("Battery voltage"); the name when the table does not know it.
    pub label: String,
    /// The raw `_pos` value.
    pub value: u16,
    pub x: u8,
    pub y: u8,
    /// The OSD profiles (1-3) it shows in; empty when it is off.
    pub profiles: Vec<u8>,
    pub variant: u8,
    /// Cells it takes: columns and rows from (x, y). The horizon's full sweep is
    /// `HORIZON_HALF` either side and `HORIZON_ROWS` down.
    pub width: u8,
    pub height: u8,
    /// The sample text it draws as.
    pub sample: String,
    /// False when the element table does not know it: drawn 5 wide.
    pub known: bool,
}

/// A rectangle an element takes on one profile's grid, after clipping to the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OsdBox {
    pub element: String,
    pub label: String,
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
}

/// What the check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// Two elements share cells.
    Overlap,
    /// Some of an element's cells fall outside the grid.
    OffScreen,
}

/// One check failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OsdProblem {
    pub kind: ProblemKind,
    pub element: String,
    /// The other element, for an overlap.
    pub other: Option<String>,
    /// How many cells.
    pub cells: u32,
    pub message: String,
}

/// One OSD profile drawn on the grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct OsdProfile {
    /// 1-3.
    pub index: u8,
    /// `osd_profile_N_name`; empty when unset.
    pub name: String,
    /// True for the profile `osd_profile` selects.
    pub active: bool,
    /// The elements on in this profile, by name.
    pub elements: Vec<String>,
    /// The drawn screen, one string per row, each `grid.width` characters.
    pub rows: Vec<String>,
    /// Where each drawn element sits, for hover names.
    pub boxes: Vec<OsdBox>,
    pub problems: Vec<OsdProblem>,
    /// What the drawing leaves out or cannot check.
    pub notes: Vec<String>,
}

/// The OSD of one configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct OsdView {
    /// Where it was read from (file names; never the person's folders).
    pub source: Vec<String>,
    /// The Betaflight version line, when the files have one.
    pub firmware: Option<String>,
    /// `vcd_video_system` (`NTSC`, `PAL`, `HD`, `AUTO`), when set.
    pub video_system: Option<String>,
    pub grid: Grid,
    /// False when no file was a `dump`: elements a `diff` leaves out keep their firmware
    /// default, which this view does not know.
    pub complete: bool,
    /// Every element the files list, by name.
    pub elements: Vec<OsdElement>,
    pub profiles: Vec<OsdProfile>,
    /// Elements on in a profile that the table does not know (drawn 5 wide).
    pub unknown: Vec<String>,
    /// Notes for the whole view (grid choice, a diff).
    pub notes: Vec<String>,
    /// True when no profile has a problem.
    pub ok: bool,
}

/// The grid for this configuration: the one asked for, else the video system's. NTSC
/// when the video system is `AUTO` or missing, with a note.
pub fn grid_for(config: &OsdConfig, asked: Option<&str>) -> Result<(Grid, Option<String>)> {
    if let Some(g) = asked.filter(|g| !g.trim().is_empty()) {
        return Ok((Grid::parse(g)?, None));
    }
    Ok(match config.video_system.as_deref() {
        Some("NTSC") => (Grid::ntsc(), None),
        Some("PAL") => (Grid::pal(), None),
        Some("HD") => (Grid::hd(), None),
        Some(other) => (
            Grid::ntsc(),
            Some(format!(
                "The video system is {other}: drawn on NTSC (30x13). Pass the grid PAL or HD to draw on those."
            )),
        ),
        None => (
            Grid::ntsc(),
            Some(
                "The files do not set vcd_video_system: drawn on NTSC (30x13). Pass the grid PAL or HD to draw on those."
                    .into(),
            ),
        ),
    })
}

fn element(config: &OsdConfig, name: &str, value: u16) -> (OsdElement, Draw) {
    let pos = Pos::decode(value);
    let (label, draw, known) = match lookup(name) {
        Some((l, d)) => (l.to_string(), d, true),
        None => (
            name.replace('_', " "),
            Draw::Text("?????", UNKNOWN_WIDTH),
            false,
        ),
    };
    let (sample, width, height) = match draw {
        Draw::Text(s, w) => (s.to_string(), w, 1),
        Draw::Setting(key, empty) => {
            let v = config
                .names
                .get(key)
                .map(|v| v.trim().to_uppercase())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| empty.to_string());
            let w = v.chars().count() as i32;
            (v, w, 1)
        }
        Draw::Horizon => ("=".repeat(9), 2 * HORIZON_HALF + 1, HORIZON_ROWS),
        Draw::Stick => (String::new(), STICK.0, STICK.1),
        Draw::Frame => (
            String::new(),
            config.camera_frame.0.unwrap_or(0).max(0),
            config.camera_frame.1.unwrap_or(0).max(0),
        ),
        Draw::Sidebars => (String::new(), 0, 0),
    };
    (
        OsdElement {
            name: name.to_string(),
            label,
            value,
            x: pos.x,
            y: pos.y,
            profiles: pos.profile_list(),
            variant: pos.variant,
            width: width.clamp(0, 255) as u8,
            height: height.clamp(0, 255) as u8,
            sample,
            known,
        },
        draw,
    )
}

/// The cells an element draws, as (column, row, character). The horizon draws its level
/// line only; its full sweep is checked separately.
fn cells(e: &OsdElement, draw: Draw) -> Vec<(i32, i32, char)> {
    let (x, y) = (e.x as i32, e.y as i32);
    match draw {
        Draw::Horizon => (-HORIZON_HALF..=HORIZON_HALF)
            .map(|dx| (x + dx, y + HORIZON_ROWS / 2 - 1, '='))
            .collect(),
        Draw::Stick => {
            let (w, h) = STICK;
            let mut v = Vec::new();
            for dy in 0..h {
                for dx in 0..w {
                    let c = if dx == w / 2 || dy == h / 2 { '+' } else { ' ' };
                    v.push((x + dx, y + dy, c));
                }
            }
            v
        }
        Draw::Frame => {
            let (w, h) = (e.width as i32, e.height as i32);
            let mut v = Vec::new();
            for dy in 0..h {
                for dx in 0..w {
                    let edge_x = dx == 0 || dx == w - 1;
                    let edge_y = dy == 0 || dy == h - 1;
                    let c = match (edge_x, edge_y) {
                        (true, true) => '+',
                        (false, true) => '-',
                        (true, false) => '|',
                        _ => continue,
                    };
                    v.push((x + dx, y + dy, c));
                }
            }
            v
        }
        Draw::Sidebars => Vec::new(),
        Draw::Text(..) | Draw::Setting(..) => {
            let mut chars: Vec<char> = e.sample.chars().collect();
            chars.resize(e.width as usize, ' ');
            chars
                .into_iter()
                .enumerate()
                .map(|(i, c)| (x + i as i32, y, c))
                .collect()
        }
    }
}

/// Element pairs that may share cells: the crosshairs sit on the horizon's level line.
fn may_overlap(a: &str, b: &str) -> bool {
    matches!((a, b), ("ah", "crosshairs") | ("crosshairs", "ah"))
        || a == "camera_frame"
        || b == "camera_frame"
}

fn render_profile(
    config: &OsdConfig,
    all: &[(OsdElement, Draw)],
    grid: &Grid,
    p: u8,
) -> OsdProfile {
    let (w, h) = (grid.width as i32, grid.height as i32);
    let mut canvas = vec![vec![' '; w as usize]; h as usize];
    let mut owner: Vec<Vec<Option<usize>>> = vec![vec![None; w as usize]; h as usize];
    let mut shown: Vec<usize> = (0..all.len())
        .filter(|i| all[*i].0.profiles.contains(&p))
        .collect();
    // The camera frame first (behind everything), the crosshairs last (over the horizon),
    // the rest by name.
    shown.sort_by_key(|i| {
        let n = all[*i].0.name.as_str();
        (
            match n {
                "camera_frame" => 0,
                "crosshairs" => 2,
                _ => 1,
            },
            n.to_string(),
        )
    });
    let mut overlaps: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut off: BTreeMap<String, u32> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let mut boxes = Vec::new();
    for &i in &shown {
        let (e, draw) = &all[i];
        match draw {
            Draw::Sidebars => {
                notes.push(format!(
                    "{} is on: drawn around the screen centre, not shown here.",
                    e.name
                ));
                continue;
            }
            Draw::Frame if e.width == 0 || e.height == 0 => {
                notes.push(format!(
                    "{} is on, with no osd_camera_frame_width or _height in the files: not drawn.",
                    e.name
                ));
                continue;
            }
            Draw::Horizon => {
                let (x, y) = (e.x as i32, e.y as i32);
                let (left, right, bottom) =
                    (x - HORIZON_HALF, x + HORIZON_HALF, y + HORIZON_ROWS - 1);
                if left < 0 || right >= w || bottom >= h {
                    problems.push(OsdProblem {
                        kind: ProblemKind::OffScreen,
                        element: e.name.clone(),
                        other: None,
                        cells: 0,
                        message: format!(
                            "{}: the moving horizon reaches off screen (columns {left}-{right}, rows {y}-{bottom}).",
                            e.name
                        ),
                    });
                }
                notes.push(format!(
                    "{}: the level line is drawn; at full pitch it sweeps rows {y}-{bottom}.",
                    e.name
                ));
            }
            _ => {}
        }
        if !e.known {
            notes.push(format!(
                "{}: not in QuadCam's element table, drawn {UNKNOWN_WIDTH} wide.",
                e.name
            ));
        }
        let mut drawn = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (cx, cy, ch) in cells(e, *draw) {
            if !(0..w).contains(&cx) || !(0..h).contains(&cy) {
                if *draw != Draw::Horizon {
                    *off.entry(e.name.clone()).or_default() += 1;
                }
                continue;
            }
            let (ux, uy) = (cx as usize, cy as usize);
            if let Some(prev) = owner[uy][ux] {
                let pn = &all[prev].0.name;
                if pn != &e.name && !may_overlap(pn, &e.name) {
                    *overlaps.entry((e.name.clone(), pn.clone())).or_default() += 1;
                }
            }
            owner[uy][ux] = Some(i);
            canvas[uy][ux] = ch;
            drawn = (
                drawn.0.min(cx),
                drawn.1.min(cy),
                drawn.2.max(cx),
                drawn.3.max(cy),
            );
        }
        if drawn.0 <= drawn.2 {
            boxes.push(OsdBox {
                element: e.name.clone(),
                label: e.label.clone(),
                x: drawn.0 as u8,
                y: drawn.1 as u8,
                width: (drawn.2 - drawn.0 + 1) as u8,
                height: (drawn.3 - drawn.1 + 1) as u8,
            });
        }
    }
    for (name, n) in off {
        problems.push(OsdProblem {
            kind: ProblemKind::OffScreen,
            message: format!("{name}: {n} cells off screen."),
            element: name,
            other: None,
            cells: n,
        });
    }
    for ((a, b), n) in overlaps {
        problems.push(OsdProblem {
            kind: ProblemKind::Overlap,
            message: format!("{a} overlaps {b} ({n} cells)."),
            element: a,
            other: Some(b),
            cells: n,
        });
    }
    OsdProfile {
        index: p,
        name: config.profile_names.get(&p).cloned().unwrap_or_default(),
        active: config.active == Some(p),
        elements: shown.iter().map(|i| all[*i].0.name.clone()).collect(),
        rows: canvas
            .into_iter()
            .map(|r| r.into_iter().collect())
            .collect(),
        boxes,
        problems,
        notes,
    }
}

/// Draws and checks every profile. `source` names the files for the answer.
pub fn view(config: &OsdConfig, grid: Grid, source: Vec<String>) -> OsdView {
    let all: Vec<(OsdElement, Draw)> = config
        .positions
        .iter()
        .map(|(n, v)| element(config, n, *v))
        .collect();
    let profiles: Vec<OsdProfile> = PROFILES
        .into_iter()
        .map(|p| render_profile(config, &all, &grid, p))
        .collect();
    let unknown: Vec<String> = all
        .iter()
        .filter(|(e, _)| !e.known && !e.profiles.is_empty())
        .map(|(e, _)| e.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut notes = Vec::new();
    if !config.complete {
        notes.push(
            "No dump among the files: an element a diff leaves out keeps its firmware default, which is not shown.".into(),
        );
    }
    if config.positions.is_empty() {
        notes.push("The files hold no osd_*_pos settings.".into());
    }
    OsdView {
        source,
        firmware: config.firmware.clone(),
        video_system: config.video_system.clone(),
        ok: profiles.iter().all(|p| p.problems.is_empty()),
        grid,
        complete: config.complete,
        elements: all.into_iter().map(|(e, _)| e).collect(),
        profiles,
        unknown,
        notes,
    }
}

/// The view as text: each profile's grid with a ruler, the elements on in it, its notes and
/// its check. The CLI's `--text` and the MCP answer.
pub fn render_text(v: &OsdView) -> String {
    let w = v.grid.width as usize;
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    if let Some(f) = &v.firmware {
        line(f.clone());
    }
    line(format!(
        "Grid {} ({}x{}){}",
        v.grid.name,
        v.grid.width,
        v.grid.height,
        v.video_system
            .as_deref()
            .map(|s| format!(", video system {s}"))
            .unwrap_or_default()
    ));
    for n in &v.notes {
        line(format!("Note: {n}"));
    }
    for p in &v.profiles {
        line(String::new());
        let name = if p.name.is_empty() {
            String::new()
        } else {
            format!(" {}", p.name)
        };
        let active = if p.active { " (in use)" } else { "" };
        line(format!("OSD profile {}{name}{active}", p.index));
        line(format!(
            "    {}",
            (0..w)
                .map(|c| char::from(b'0' + (c / 10 % 10) as u8))
                .collect::<String>()
        ));
        line(format!(
            "    {}",
            (0..w)
                .map(|c| char::from(b'0' + (c % 10) as u8))
                .collect::<String>()
        ));
        line(format!("   +{}+", "-".repeat(w)));
        for (r, row) in p.rows.iter().enumerate() {
            line(format!("{r:2} |{row}|"));
        }
        line(format!("   +{}+", "-".repeat(w)));
        for name in &p.elements {
            if let Some(e) = v.elements.iter().find(|e| &e.name == name) {
                let variant = if e.variant != 0 {
                    format!(", variant {}", e.variant)
                } else {
                    String::new()
                };
                line(format!(
                    "  {:<24} x {:2}, y {:2}{variant}  {}",
                    e.name, e.x, e.y, e.label
                ));
            }
        }
        for n in &p.notes {
            line(format!("  note: {n}"));
        }
        for pr in &p.problems {
            line(format!("  FAIL: {}", pr.message));
        }
        line(format!(
            "  check: {}",
            if p.problems.is_empty() {
                "ok (no overlaps, nothing off screen)"
            } else {
                "FAIL"
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip_every_value() {
        for x in 0..64u8 {
            for y in 0..32u8 {
                for profiles in 0..8u8 {
                    for variant in 0..4u8 {
                        let p = Pos {
                            x,
                            y,
                            profiles,
                            variant,
                        };
                        assert_eq!(Pos::decode(p.encode()), p);
                    }
                }
            }
        }
        for v in 0..=u16::MAX {
            assert_eq!(Pos::decode(v).encode(), v, "{v}");
        }
    }

    #[test]
    fn known_values() {
        // x 2, y 16, profiles 1 and 2 (osdmap.py's reference: `pos 2 16 1,2`).
        let p = Pos::decode(6658);
        assert_eq!((p.x, p.y, p.profile_list()), (2, 16, vec![1, 2]));
        // x 43 needs bit 10.
        let p = Pos {
            x: 43,
            y: 14,
            profiles: 0b011,
            variant: 0,
        };
        assert_eq!(p.encode(), 7627);
        assert_eq!(Pos::decode(341).profile_list(), Vec::<u8>::new());
        assert!(Pos::decode(1 << 11).in_profile(1));
        assert!(!Pos::decode(1 << 11).in_profile(0));
        assert_eq!(Pos::decode(3 << 14).variant, 3);
    }

    #[test]
    fn grids() {
        assert_eq!(Grid::parse("ntsc").unwrap(), Grid::ntsc());
        assert_eq!((Grid::pal().width, Grid::pal().height), (30, 16));
        assert_eq!((Grid::hd().width, Grid::hd().height), (53, 20));
        let g = Grid::parse("40x18").unwrap();
        assert_eq!((g.name.as_str(), g.width, g.height), ("40x18", 40, 18));
        assert!(Grid::parse("0x10").is_err());
        assert!(Grid::parse("100x10").is_err());
        assert!(Grid::parse("SECAM").is_err());
    }

    #[test]
    fn source_kinds() {
        assert_eq!(source_kind("dump all\n# version\n"), SourceKind::Dump);
        assert_eq!(source_kind("\ndiff all\n"), SourceKind::Diff);
        assert_eq!(source_kind("diff\n"), SourceKind::Diff);
        assert_eq!(source_kind("set osd_vbat_pos = 1\n"), SourceKind::Lines);
    }

    fn cfg(lines: &str) -> OsdConfig {
        OsdConfig::parse([lines])
    }

    #[test]
    fn later_files_win_and_grid_follows_video_system() {
        let c = OsdConfig::parse([
            "dump all\nset vcd_video_system = PAL\nset osd_vbat_pos = 2049\n",
            "set osd_vbat_pos = 2050\n# comment\n",
        ]);
        assert!(c.complete);
        assert_eq!(c.positions["vbat"], 2050);
        let (g, note) = grid_for(&c, None).unwrap();
        assert_eq!((g, note), (Grid::pal(), None));
        let (g, _) = grid_for(&c, Some("HD")).unwrap();
        assert_eq!(g, Grid::hd());
        let (g, note) = grid_for(&cfg("set vcd_video_system = AUTO"), None).unwrap();
        assert_eq!(g, Grid::ntsc());
        assert!(note.unwrap().contains("AUTO"));
    }

    fn pos(x: u8, y: u8, profiles: u8) -> u16 {
        Pos {
            x,
            y,
            profiles,
            variant: 0,
        }
        .encode()
    }

    #[test]
    fn overlap_in_one_profile_only() {
        // vbat (6 wide) at 1,1 and rssi at 4,1: they overlap in profile 1, where both are
        // on. In profile 2 only vbat is on.
        let c = cfg(&format!(
            "set osd_vbat_pos = {}\nset osd_rssi_pos = {}\n",
            pos(1, 1, 0b011),
            pos(4, 1, 0b001)
        ));
        let v = view(&c, Grid::ntsc(), vec![]);
        assert!(!v.ok);
        let p1 = &v.profiles[0];
        assert_eq!(p1.problems.len(), 1);
        assert_eq!(p1.problems[0].kind, ProblemKind::Overlap);
        assert_eq!(p1.problems[0].cells, 3);
        assert!(v.profiles[1].problems.is_empty());
        assert!(v.profiles[2].elements.is_empty());
    }

    #[test]
    fn off_screen_right_and_bottom() {
        // warnings (12 wide) at x 25 on a 30-wide grid: 7 cells off. vbat at row 14 is on
        // a PAL screen but below an NTSC one.
        let c = cfg(&format!(
            "set osd_warnings_pos = {}\nset osd_vbat_pos = {}\n",
            pos(25, 0, 1),
            pos(1, 14, 1)
        ));
        let ntsc = view(&c, Grid::ntsc(), vec![]);
        let msgs: Vec<_> = ntsc.profiles[0]
            .problems
            .iter()
            .map(|p| (p.element.as_str(), p.cells))
            .collect();
        assert_eq!(msgs, vec![("vbat", 6), ("warnings", 7)]);
        let pal = view(&c, Grid::pal(), vec![]);
        assert_eq!(pal.profiles[0].problems.len(), 1, "vbat fits PAL");
        let hd = view(&c, Grid::hd(), vec![]);
        assert!(hd.ok);
    }

    #[test]
    fn horizon_sweep_and_crosshairs() {
        // The horizon at y 5 on NTSC sweeps rows 5-14: off the 13-row screen. The
        // crosshairs on its level line are fine.
        let c = cfg(&format!(
            "set osd_ah_pos = {}\nset osd_crosshairs_pos = {}\n",
            pos(14, 5, 1),
            pos(13, 9, 1)
        ));
        let v = view(&c, Grid::ntsc(), vec![]);
        let p = &v.profiles[0];
        assert_eq!(p.problems.len(), 1);
        assert_eq!(p.problems[0].kind, ProblemKind::OffScreen);
        assert!(p.problems[0].message.contains("rows 5-14"));
        assert_eq!(&p.rows[9][10..19], "===-+-===");
        assert!(view(&c, Grid::pal(), vec![]).ok);
    }

    #[test]
    fn unknown_elements_draw_five_wide() {
        let c = cfg(&format!("set osd_new_thing_pos = {}\n", pos(0, 0, 1)));
        let v = view(&c, Grid::ntsc(), vec![]);
        assert_eq!(v.unknown, vec!["new_thing".to_string()]);
        assert_eq!(&v.profiles[0].rows[0][..6], "????? ");
        assert!(!v.complete);
        assert!(v.notes.iter().any(|n| n.contains("diff")));
    }

    #[test]
    fn camera_frame_sidebars_and_craft_name() {
        let c = cfg(&format!(
            "set osd_camera_frame_pos = {}\nset osd_camera_frame_width = 6\nset osd_camera_frame_height = 3\n\
             set osd_craft_name_pos = {}\nset craft_name = Whoop\nset osd_ah_sbar_pos = {}\n",
            pos(0, 0, 1),
            pos(2, 1, 1),
            pos(3, 3, 1)
        ));
        let v = view(&c, Grid::ntsc(), vec![]);
        let p = &v.profiles[0];
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(&p.rows[0][..6], "+----+");
        assert_eq!(&p.rows[1][..7], "| WHOOP");
        assert!(p.notes.iter().any(|n| n.starts_with("ah_sbar")));
        assert!(p
            .boxes
            .iter()
            .any(|b| b.element == "craft_name" && b.width == 5));
    }
}
