//! The telemetry overlay (`docs/telemetry-overlay.md`): a clip's flight numbers, from the
//! radio log rows it matched or from a goggles `.srt` sidecar, as timed samples; the samples
//! as a SubRip file; and the ffmpeg arguments that burn that file into a copy of the clip.

use crate::logs::LogRow;
use crate::media::{Encoder, Tools};
use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// A gap between log rows longer than this ends a sample at its row interval instead.
const MAX_HOLD_S: f64 = 2.0;
/// A sample's length when nothing follows it.
const LAST_HOLD_S: f64 = 0.5;
/// A LiPo or LiHV cell is never above this, so the highest voltage sets the cell count.
const MAX_CELL_V: f64 = 4.4;

/// One number the overlay can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryField {
    /// The aircraft's battery voltage: the receiver's `RxBt`, or the sidecar's air-unit battery.
    Battery,
    BatteryPct,
    /// mAh used.
    Used,
    Current,
    Lq,
    Rssi,
    Snr,
    /// Transmit power, mW.
    Power,
    Throttle,
    Mode,
    /// Armed time so far (radio log), or the sidecar's flight time.
    Timer,
    GogglesBattery,
    /// The sidecar's signal bars.
    Signal,
    Delay,
    Bitrate,
    Distance,
}

impl TelemetryField {
    pub const ALL: [TelemetryField; 16] = [
        TelemetryField::Battery,
        TelemetryField::BatteryPct,
        TelemetryField::Used,
        TelemetryField::Current,
        TelemetryField::GogglesBattery,
        TelemetryField::Lq,
        TelemetryField::Rssi,
        TelemetryField::Snr,
        TelemetryField::Power,
        TelemetryField::Signal,
        TelemetryField::Delay,
        TelemetryField::Bitrate,
        TelemetryField::Distance,
        TelemetryField::Throttle,
        TelemetryField::Mode,
        TelemetryField::Timer,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TelemetryField::Battery => "battery",
            TelemetryField::BatteryPct => "battery_pct",
            TelemetryField::Used => "used",
            TelemetryField::Current => "current",
            TelemetryField::Lq => "lq",
            TelemetryField::Rssi => "rssi",
            TelemetryField::Snr => "snr",
            TelemetryField::Power => "power",
            TelemetryField::Throttle => "throttle",
            TelemetryField::Mode => "mode",
            TelemetryField::Timer => "timer",
            TelemetryField::GogglesBattery => "goggles_battery",
            TelemetryField::Signal => "signal",
            TelemetryField::Delay => "delay",
            TelemetryField::Bitrate => "bitrate",
            TelemetryField::Distance => "distance",
        }
    }

    pub fn parse(s: &str) -> Option<TelemetryField> {
        TelemetryField::ALL
            .into_iter()
            .find(|f| f.name() == s.trim())
    }

    /// The overlay line it sits on: battery, link, then flying.
    fn line(self) -> usize {
        match self {
            TelemetryField::Battery
            | TelemetryField::BatteryPct
            | TelemetryField::Used
            | TelemetryField::Current
            | TelemetryField::GogglesBattery => 0,
            TelemetryField::Lq
            | TelemetryField::Rssi
            | TelemetryField::Snr
            | TelemetryField::Power
            | TelemetryField::Signal
            | TelemetryField::Delay
            | TelemetryField::Bitrate
            | TelemetryField::Distance => 1,
            TelemetryField::Throttle | TelemetryField::Mode | TelemetryField::Timer => 2,
        }
    }
}

/// The numbers at one moment; a number the source lacks is None.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Values {
    pub battery_v: Option<f64>,
    pub battery_pct: Option<f64>,
    pub used_mah: Option<f64>,
    pub current_a: Option<f64>,
    pub lq: Option<f64>,
    pub rssi_db: Option<f64>,
    pub snr_db: Option<f64>,
    pub power_mw: Option<f64>,
    /// 0..1.
    pub throttle: Option<f64>,
    pub mode: Option<String>,
    pub timer_s: Option<f64>,
    pub goggles_battery_v: Option<f64>,
    pub signal: Option<f64>,
    pub delay_ms: Option<f64>,
    pub bitrate_mbps: Option<f64>,
    pub distance_m: Option<f64>,
}

impl Values {
    fn has(&self, f: TelemetryField) -> bool {
        match f {
            TelemetryField::Battery => self.battery_v.is_some(),
            TelemetryField::BatteryPct => self.battery_pct.is_some(),
            TelemetryField::Used => self.used_mah.is_some(),
            TelemetryField::Current => self.current_a.is_some(),
            TelemetryField::Lq => self.lq.is_some(),
            TelemetryField::Rssi => self.rssi_db.is_some(),
            TelemetryField::Snr => self.snr_db.is_some(),
            TelemetryField::Power => self.power_mw.is_some(),
            TelemetryField::Throttle => self.throttle.is_some(),
            TelemetryField::Mode => self.mode.as_deref().is_some_and(|m| !m.is_empty()),
            TelemetryField::Timer => self.timer_s.is_some(),
            TelemetryField::GogglesBattery => self.goggles_battery_v.is_some(),
            TelemetryField::Signal => self.signal.is_some(),
            TelemetryField::Delay => self.delay_ms.is_some(),
            TelemetryField::Bitrate => self.bitrate_mbps.is_some(),
            TelemetryField::Distance => self.distance_m.is_some(),
        }
    }

    /// One field as overlay text, with its label; None when the value is missing.
    fn text(&self, f: TelemetryField, cells: u32) -> Option<String> {
        Some(match f {
            TelemetryField::Battery => {
                let v = self.battery_v?;
                if cells > 1 {
                    format!("BAT {v:.1}V {:.2}/c", v / cells as f64)
                } else {
                    format!("BAT {v:.2}V")
                }
            }
            TelemetryField::BatteryPct => format!("{:.0}%", self.battery_pct?),
            TelemetryField::Used => format!("{:.0}mAh", self.used_mah?),
            TelemetryField::Current => format!("{:.1}A", self.current_a?),
            TelemetryField::Lq => format!("LQ {:.0}%", self.lq?),
            TelemetryField::Rssi => format!("RSSI {:.0}dB", self.rssi_db?),
            TelemetryField::Snr => format!("SNR {:.0}dB", self.snr_db?),
            TelemetryField::Power => format!("{:.0}mW", self.power_mw?),
            TelemetryField::Throttle => format!("THR {:.0}%", self.throttle? * 100.0),
            TelemetryField::Mode => self.mode.clone().filter(|m| !m.is_empty())?,
            TelemetryField::Timer => {
                let s = self.timer_s?.max(0.0).floor() as u64;
                format!("{}:{:02}", s / 60, s % 60)
            }
            TelemetryField::GogglesBattery => format!("GOG {:.1}V", self.goggles_battery_v?),
            TelemetryField::Signal => format!("SIG {:.0}", self.signal?),
            TelemetryField::Delay => format!("{:.0}ms", self.delay_ms?),
            TelemetryField::Bitrate => format!("{:.1}Mbps", self.bitrate_mbps?),
            TelemetryField::Distance => format!("{:.0}m", self.distance_m?),
        })
    }
}

/// The numbers from `start` to `end`, in clip seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Sample {
    pub start: f64,
    pub end: f64,
    pub values: Values,
}

impl Sample {
    pub fn shifted(&self, by: f64) -> Sample {
        Sample {
            start: self.start + by,
            end: self.end + by,
            values: self.values.clone(),
        }
    }
}

/// Samples from the claimed log rows, timed in seconds from `zero` (the first armed row).
/// Rows outside every armed window (`windows`) are left out; the timer counts the armed time
/// of the windows up to each row. 0 V and 0 % mean no telemetry yet, not a reading.
pub fn from_log(
    rows: &[LogRow],
    windows: &[(NaiveDateTime, NaiveDateTime)],
    zero: NaiveDateTime,
) -> Vec<Sample> {
    let secs = |t: NaiveDateTime| (t - zero).num_milliseconds() as f64 / 1000.0;
    let inside: Vec<&LogRow> = rows
        .iter()
        .filter(|r| windows.iter().any(|(a, b)| r.time >= *a && r.time <= *b))
        .collect();
    let armed_until = |t: NaiveDateTime| -> f64 {
        windows
            .iter()
            .map(|(a, b)| {
                let end = (*b).min(t);
                if end > *a {
                    (end - *a).num_milliseconds() as f64 / 1000.0
                } else {
                    0.0
                }
            })
            .sum()
    };
    let mut out: Vec<Sample> = Vec::with_capacity(inside.len());
    for (i, r) in inside.iter().enumerate() {
        let start = secs(r.time);
        let next = inside.get(i + 1).map(|n| secs(n.time));
        let end = match next {
            Some(n) if n - start <= MAX_HOLD_S && n > start => n,
            _ => start + LAST_HOLD_S,
        };
        let values = Values {
            battery_v: r.rx_bat.filter(|v| *v > 0.0),
            battery_pct: r.bat_pct.filter(|v| *v > 0.0),
            used_mah: r.capacity_mah,
            current_a: r.current_a,
            lq: r.lq.filter(|v| *v > 0.0),
            rssi_db: r.rssi.filter(|v| *v < 0.0),
            snr_db: r.snr_db,
            power_mw: r.tx_power_mw.filter(|v| *v > 0.0),
            throttle: r
                .sticks
                .map(|s| ((s.thr + 1024.0) / 2048.0).clamp(0.0, 1.0)),
            mode: r.flight_mode.as_deref().map(str::to_string),
            timer_s: Some(armed_until(r.time)),
            ..Values::default()
        };
        out.push(Sample { start, end, values });
    }
    out
}

/// One SubRip cue: its times in seconds and its text with the markup kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// `HH:MM:SS,mmm` (a `.` works too) as seconds.
fn srt_time(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let mut parts = s.split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let sec: f64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// The cues of a SubRip file. Blocks without a valid time line are skipped; an empty file
/// has no cues.
pub fn parse_srt(text: &str) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|l| !l.is_empty());
        let mut first = lines.next();
        // The cue number is optional in practice.
        if first.is_some_and(|l| !l.contains("-->")) {
            first = lines.next();
        }
        let Some((a, b)) = first.and_then(|l| l.split_once("-->")) else {
            continue;
        };
        let (Some(start), Some(end)) = (srt_time(a), srt_time(b)) else {
            continue;
        };
        let body: Vec<&str> = lines.collect();
        out.push(Cue {
            start,
            end,
            text: body.join("\n"),
        });
    }
    out
}

/// `key:value` pairs in a cue's text. Markup (`<font ...>`) and brackets go, and a space
/// around `:` or `=` closes up, so `[iso : 100]`, `Signal:4` and `Sp=13` all read. Keys are
/// lowercased.
pub fn pairs(text: &str) -> Vec<(String, String)> {
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '[' | ']' | ',' if !in_tag => plain.push(' '),
            _ if !in_tag => plain.push(ch),
            _ => {}
        }
    }
    let mut closed = plain;
    for sep in [':', '='] {
        loop {
            let next = closed
                .replace(&format!(" {sep}"), &sep.to_string())
                .replace(&format!("{sep} "), &sep.to_string());
            if next == closed {
                break;
            }
            closed = next;
        }
    }
    closed
        .split_whitespace()
        .filter_map(|tok| {
            let i = tok.find([':', '='])?;
            let (k, v) = (&tok[..i], &tok[i + 1..]);
            (!k.is_empty() && !v.is_empty() && k.chars().next()?.is_alphabetic())
                .then(|| (k.to_lowercase(), v.to_string()))
        })
        .collect()
}

/// The number a value starts with: `9.9V` is 9.9, `-52dBm` is -52.
fn number(v: &str) -> Option<f64> {
    let end = v
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))))
        .map(|(i, _)| i)
        .unwrap_or(v.len());
    v[..end].parse().ok()
}

/// What a goggles sidecar held: its samples and the keys QuadCam does not show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sidecar {
    pub samples: Vec<Sample>,
    pub unknown_keys: Vec<String>,
}

/// Samples from a DJI or Walksnail goggles `.srt`: each cue's known keys.
pub fn from_sidecar(text: &str) -> Sidecar {
    let mut unknown: Vec<String> = Vec::new();
    let mut samples = Vec::new();
    for cue in parse_srt(text) {
        let mut v = Values::default();
        let mut any = false;
        for (k, raw) in pairs(&cue.text) {
            let n = number(&raw);
            let slot = match k.as_str() {
                "sbat" | "uavbat" => &mut v.battery_v,
                "gbat" | "glsbat" => &mut v.goggles_battery_v,
                "signal" => &mut v.signal,
                "delay" => &mut v.delay_ms,
                "bitrate" => &mut v.bitrate_mbps,
                "distance" => &mut v.distance_m,
                "flighttime" => &mut v.timer_s,
                _ => {
                    if !unknown.contains(&k) {
                        unknown.push(k);
                    }
                    continue;
                }
            };
            if let Some(n) = n {
                *slot = Some(n);
                any = true;
            }
        }
        if any {
            samples.push(Sample {
                start: cue.start,
                end: cue.end,
                values: v,
            });
        }
    }
    unknown.sort();
    Sidecar {
        samples,
        unknown_keys: unknown,
    }
}

/// The fields any sample has, in display order.
pub fn available(samples: &[Sample]) -> Vec<TelemetryField> {
    TelemetryField::ALL
        .into_iter()
        .filter(|f| samples.iter().any(|s| s.values.has(*f)))
        .collect()
}

/// The battery's cell count from its highest voltage; 1 without a voltage.
pub fn cells(samples: &[Sample]) -> u32 {
    let max = samples
        .iter()
        .filter_map(|s| s.values.battery_v)
        .fold(0.0_f64, f64::max);
    if max <= 0.0 {
        1
    } else {
        (max / MAX_CELL_V).ceil().max(1.0) as u32
    }
}

/// A sample's overlay text: the fields in `fields` order, one line per group.
pub fn text(v: &Values, fields: &[TelemetryField], cells: u32) -> String {
    let mut lines: [Vec<String>; 3] = Default::default();
    for f in fields {
        if let Some(t) = v.text(*f, cells) {
            lines[f.line()].push(t);
        }
    }
    lines
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| l.join("  "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn srt_stamp(s: f64) -> String {
    let ms = (s.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// The samples as a SubRip file, cut to `0..duration`. Neighbours with the same text join
/// into one cue. Returns the file's text and its cue count.
pub fn to_srt(
    samples: &[Sample],
    fields: &[TelemetryField],
    cells: u32,
    duration: f64,
) -> (String, usize) {
    let mut cues: Vec<(f64, f64, String)> = Vec::new();
    for s in samples {
        let (a, b) = (s.start.max(0.0), s.end.min(duration));
        if b <= a {
            continue;
        }
        let t = text(&s.values, fields, cells);
        if t.is_empty() {
            continue;
        }
        match cues.last_mut() {
            Some(last) if last.2 == t && (a - last.1).abs() < 0.001 => last.1 = b,
            _ => cues.push((a, b, t)),
        }
    }
    let mut out = String::new();
    for (i, (a, b, t)) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_stamp(*a),
            srt_stamp(*b),
            t
        ));
    }
    (out, cues.len())
}

/// The libass style of the burned-in text: a fixed-width font, white with a dark outline, at
/// the bottom left. Sizes are in the 384x288 frame libass gives a SubRip file.
pub const BURN_STYLE: &str = "FontName=Menlo,FontSize=11,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,BorderStyle=1,Outline=1,Shadow=0,Alignment=1,MarginL=10,MarginV=8";

/// True when this ffmpeg has the `subtitles` filter (built with libass).
pub fn can_burn(tools: &Tools) -> Result<bool> {
    let out = Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-filters"])
        .output()
        .context("running ffmpeg")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some("subtitles")))
}

/// A path as a quoted filter-graph value: `'` closes the quote, an escaped `'` and opens it
/// again.
fn filter_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

/// The ffmpeg arguments (after the global flags) that burn `srt` into `src`, written to
/// `dst` as MP4: the first video stream re-encoded, the audio as AAC, the source's metadata
/// kept.
pub fn burn_args(src: &Path, srt: &Path, dst: &Path, encoder: Encoder) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().to_string();
    let mut a: Vec<String> = vec![
        "-i".into(),
        s(src),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-vf".into(),
        format!(
            "subtitles=filename={}:force_style='{BURN_STYLE}'",
            filter_quote(srt)
        ),
    ];
    match encoder {
        Encoder::Videotoolbox => {
            a.extend(["-c:v", "h264_videotoolbox", "-q:v", "65"].map(String::from))
        }
        Encoder::X264 => {
            a.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"].map(String::from))
        }
    }
    a.extend(
        [
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-map_metadata",
            "0",
            "-f",
            "mp4",
        ]
        .map(String::from),
    );
    a.push(s(dst));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::Sticks;
    use chrono::NaiveDate;

    fn t(s: f64) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 5)
            .unwrap()
            .and_hms_opt(18, 0, 0)
            .unwrap()
            + chrono::Duration::milliseconds((s * 1000.0) as i64)
    }

    fn row(s: f64, v: f64, thr: f64) -> LogRow {
        let mut r = LogRow::at(t(s));
        r.rx_bat = Some(v);
        r.lq = Some(100.0);
        r.rssi = Some(-50.0);
        r.flight_mode = Some("ACRO".into());
        r.sticks = Some(Sticks {
            thr,
            ..Sticks::default()
        });
        r
    }

    #[test]
    fn log_rows_become_samples_inside_the_flights() {
        let rows: Vec<LogRow> = (0..40)
            .map(|i| row(i as f64 * 0.5, 16.4 - i as f64 * 0.01, 0.0))
            .collect();
        // Two flights: 0-5 s and 10-15 s.
        let windows = [(t(0.0), t(5.0)), (t(10.0), t(15.0))];
        let s = from_log(&rows, &windows, t(0.0));
        assert_eq!(s.len(), 22);
        assert_eq!(s[0].start, 0.0);
        assert_eq!(s[0].end, 0.5);
        // The last row of the first flight holds only its interval, not the 5 s gap.
        let last_first = s.iter().find(|x| x.start == 5.0).unwrap();
        assert_eq!(last_first.end, 5.5);
        // The timer counts armed time only: 5 s of the first flight, 2 s of the second.
        let at12 = s.iter().find(|x| x.start == 12.0).unwrap();
        assert_eq!(at12.values.timer_s, Some(7.0));
        assert_eq!(at12.values.throttle, Some(0.5));
        assert_eq!(cells(&s), 4);
    }

    #[test]
    fn missing_telemetry_reads_as_none() {
        let mut r = row(0.0, 0.0, -1024.0);
        r.lq = Some(0.0);
        r.rssi = Some(0.0);
        let s = from_log(&[r], &[(t(0.0), t(1.0))], t(0.0));
        assert_eq!(s[0].values.battery_v, None);
        assert_eq!(s[0].values.lq, None);
        assert_eq!(s[0].values.rssi_db, None);
        assert_eq!(s[0].values.throttle, Some(0.0));
        assert_eq!(cells(&s), 1);
    }

    #[test]
    fn text_puts_fields_on_three_lines() {
        let v = Values {
            battery_v: Some(15.8),
            lq: Some(98.0),
            rssi_db: Some(-61.0),
            throttle: Some(0.42),
            mode: Some("ACRO".into()),
            timer_s: Some(83.4),
            ..Values::default()
        };
        let all = TelemetryField::ALL;
        assert_eq!(
            text(&v, &all, 4),
            "BAT 15.8V 3.95/c\nLQ 98%  RSSI -61dB\nTHR 42%  ACRO  1:23"
        );
        assert_eq!(text(&v, &[TelemetryField::Battery], 1), "BAT 15.80V");
        assert_eq!(text(&v, &[TelemetryField::Delay], 1), "");
    }

    #[test]
    fn srt_joins_same_text_and_cuts_to_the_clip() {
        let v = Values {
            lq: Some(100.0),
            ..Values::default()
        };
        let s = vec![
            Sample {
                start: -1.0,
                end: 0.5,
                values: v.clone(),
            },
            Sample {
                start: 0.5,
                end: 1.0,
                values: v.clone(),
            },
            Sample {
                start: 1.0,
                end: 3.0,
                values: Values {
                    lq: Some(90.0),
                    ..Values::default()
                },
            },
        ];
        let (srt, n) = to_srt(&s, &[TelemetryField::Lq], 1, 2.0);
        assert_eq!(n, 2);
        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:01,000\nLQ 100%\n\n2\n00:00:01,000 --> 00:00:02,000\nLQ 90%\n\n"
        );
        // Its own output reads back.
        let cues = parse_srt(&srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[1].start, 1.0);
        assert_eq!(cues[1].text, "LQ 90%");
    }

    #[test]
    fn walksnail_sidecar() {
        let srt = "1\r\n00:00:00,000 --> 00:00:00,150\r\nSignal:4 CH:8 Hz:5740000 FlightTime:0 Sp=13 Gp=14 SBat:9.9V GBat:12.0V Delay:48ms Bitrate:25.0Mbps Distance:9m\r\n\r\n2\r\n00:00:00,150 --> 00:00:00,300\r\nSignal:3 CH:8 FlightTime:1 SBat:9.8V GBat:12.0V Delay:52ms Bitrate:24.5Mbps Distance:12m\r\n";
        let s = from_sidecar(srt);
        assert_eq!(s.samples.len(), 2);
        let v = &s.samples[0].values;
        assert_eq!(v.battery_v, Some(9.9));
        assert_eq!(v.goggles_battery_v, Some(12.0));
        assert_eq!(v.signal, Some(4.0));
        assert_eq!(v.delay_ms, Some(48.0));
        assert_eq!(v.bitrate_mbps, Some(25.0));
        assert_eq!(v.distance_m, Some(9.0));
        assert_eq!(v.timer_s, Some(0.0));
        assert_eq!(s.samples[1].start, 0.15);
        assert_eq!(s.unknown_keys, ["ch", "gp", "hz", "sp"]);
        assert_eq!(
            available(&s.samples),
            [
                TelemetryField::Battery,
                TelemetryField::GogglesBattery,
                TelemetryField::Signal,
                TelemetryField::Delay,
                TelemetryField::Bitrate,
                TelemetryField::Distance,
                TelemetryField::Timer
            ]
        );
    }

    #[test]
    fn dji_sidecar_with_markup_and_brackets() {
        let srt = "1\n00:00:00,000 --> 00:00:00,033\n<font size=\"28\">signal:4 ch:1 flightTime:40 uavBat:16.2V glsBat:15.4V uavBatCells:4 delay:25ms bitrate:37.9Mbps [iso : 100] [rel_alt: 1.5 abs_alt: 30.2]</font>\n";
        let s = from_sidecar(srt);
        let v = &s.samples[0].values;
        assert_eq!(v.battery_v, Some(16.2));
        assert_eq!(v.goggles_battery_v, Some(15.4));
        assert_eq!(v.timer_s, Some(40.0));
        assert_eq!(v.bitrate_mbps, Some(37.9));
        assert!(s.unknown_keys.contains(&"iso".to_string()));
        assert!(s.unknown_keys.contains(&"abs_alt".to_string()));
        assert!(!s.unknown_keys.iter().any(|k| k == "size"));
    }

    #[test]
    fn an_empty_or_camera_only_sidecar_has_no_samples() {
        assert!(from_sidecar("").samples.is_empty());
        let cam = "1\n00:00:00,000 --> 00:00:00,033\n[iso : 100] [shutter : 1/60.0]\n";
        let s = from_sidecar(cam);
        assert!(s.samples.is_empty());
        assert_eq!(s.unknown_keys, ["iso", "shutter"]);
    }

    #[test]
    fn fields_parse_by_name() {
        for f in TelemetryField::ALL {
            assert_eq!(TelemetryField::parse(f.name()), Some(f));
            assert_eq!(
                serde_json::to_value(f).unwrap(),
                serde_json::Value::String(f.name().into())
            );
        }
        assert_eq!(TelemetryField::parse("altitude"), None);
    }

    #[test]
    fn burn_args_quote_the_subtitle_path() {
        let a = burn_args(
            Path::new("/in/a.mp4"),
            Path::new("/tmp/it's.srt"),
            Path::new("/out/b.mp4"),
            Encoder::X264,
        );
        let vf = &a[a.iter().position(|x| x == "-vf").unwrap() + 1];
        assert!(
            vf.starts_with(r"subtitles=filename='/tmp/it'\''s.srt':force_style='FontName=Menlo,")
        );
        assert!(a.contains(&"libx264".to_string()));
        assert_eq!(a.last().unwrap(), "/out/b.mp4");
    }
}
