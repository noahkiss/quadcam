//! Rate curves for the Rates segment (design 6.6, sim-design 5.1): what an FC's rate profiles
//! do to the sticks, drawn as degrees per second against stick deflection, with the throttle
//! curve, and the conversion of one rate model to another.
//!
//! The curve math is `quadcam_sim::rates` (the one implementation); this module re-exports it,
//! reads every rate profile of a `dump all` or `diff all`, samples the curves for the chart,
//! and fits one model to another by least squares. It reads text only.
//!
//! **Fit.** Sims take Betaflight-style rates. `fit` finds whole-number settings of the target
//! model (the CLI stores whole numbers) whose curve is closest to the source curve over the
//! stick range, by sum of squared differences, and reports the largest difference in degrees
//! per second. The stated bound is `FIT_TOLERANCE`: the fit of an Actual profile with a
//! centre of 40-200 deg/s, a maximum of 300-1000 deg/s and expo 0-60 onto the Betaflight
//! model stays within that share of the profile's maximum rate (tested over a grid in
//! `fit_stays_within_tolerance`). The Betaflight model cannot bend the centre as freely as
//! Actual can, so the largest difference sits at mid stick. Outside that grid (a centre
//! under 40 deg/s, or expo above 60) the error grows, up to about 12 %; every view and sim
//! row states the actual error, so nothing hides behind the bound.

use crate::gear::bf::dump::Config;
use crate::gear::model::Section;
use serde::{Deserialize, Serialize};
use specta::Type;

pub use quadcam_sim::rates::{
    rate, RateAxis, Rates, RatesType, ThrottleCurve, ThrottleLimit, MAX_RATE_DEG_S,
};

/// Samples per curve: stick 0 to 1 in `STEPS` equal steps, `STEPS + 1` points.
pub const STEPS: usize = 50;
/// The largest difference a fit onto the Betaflight model leaves, as a share of the
/// profile's maximum rate, for typical profiles (see the module note for the grid).
pub const FIT_TOLERANCE: f64 = 0.06;
/// Two curves closer than this (deg/s, at every sample) read as the same rates.
pub const SAME_DEG_S: f64 = 1.0;

/// Betaflight 4.3 and later, when a diff leaves a value out: Actual rates, 70 deg/s centre,
/// 670 deg/s max, expo 0.54.
const DEFAULT_AXIS: RateAxis = RateAxis {
    rc_rate: 7.0,
    srate: 67.0,
    expo: 54.0,
};
const AXES: [&str; 3] = ["roll", "pitch", "yaw"];

/// The CLI name of a rate model, lower case.
pub fn type_name(t: RatesType) -> &'static str {
    match t {
        RatesType::Betaflight => "betaflight",
        RatesType::Raceflight => "raceflight",
        RatesType::Kiss => "kiss",
        RatesType::Actual => "actual",
        RatesType::Quick => "quick",
    }
}

/// The reverse of `type_name`.
pub fn type_of(s: &str) -> Option<RatesType> {
    RatesType::from_cli(s)
}

/// One axis of a rate profile, with its sampled curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct AxisView {
    /// `roll`, `pitch` or `yaw`.
    pub axis: String,
    /// The three numbers as the CLI stores them (`*_rc_rate`, `*_srate`, `*_expo`).
    pub rc_rate: f64,
    pub srate: f64,
    pub expo: f64,
    /// `*_rate_limit` (deg/s).
    pub rate_limit: f64,
    /// Deg/s at full stick (after the limit).
    pub max_deg_s: f64,
    /// Deg/s per full stick at the centre: the curve's slope at 0.
    pub center_deg_s: f64,
    /// Deg/s at stick `i / STEPS`, `i` from 0 to `STEPS`. The curve is odd: the negative
    /// half mirrors it.
    pub curve: Vec<f64>,
}

/// The throttle curve of a rate profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ThrottleView {
    pub mid: f64,
    pub expo: f64,
    /// `thr_hover`, when the firmware has it.
    pub hover: Option<f64>,
    /// `off`, `scale` or `clip`.
    pub limit: String,
    pub limit_percent: f64,
    /// Output (0-1) at stick `i / STEPS`.
    pub curve: Vec<f64>,
}

/// One rate profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RateProfileView {
    pub index: u8,
    /// `rateprofile_name`, when set (FREE, RACE, CINE).
    pub name: Option<String>,
    /// The profile the FC uses (the last `rateprofile` line).
    pub active: bool,
    /// `betaflight`, `actual`, `quick`, `raceflight` or `kiss`.
    pub rates_type: String,
    /// False when the source left values out (a diff): they read as Betaflight's defaults.
    pub complete: bool,
    /// Roll, pitch, yaw.
    pub axes: Vec<AxisView>,
    pub throttle: ThrottleView,
}

/// What a dump or diff says about rates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RatesView {
    /// Where it was read from (file names and backup ids).
    pub source: Vec<String>,
    /// The Betaflight version line, when the source has one.
    pub firmware: Option<String>,
    /// The active rate profile's index.
    pub active: Option<u8>,
    pub profiles: Vec<RateProfileView>,
    pub notes: Vec<String>,
}

/// Rates and throttle of one profile, as the math takes them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub rates: Rates,
    /// `*_rate_limit` per axis.
    pub limits: [f64; 3],
    pub throttle: ThrottleCurve,
}

/// Deg/s at stick `x` (-1 to 1), with the axis limit.
pub fn rate_at(model: RatesType, p: &RateAxis, limit: f64, x: f64) -> f64 {
    let r = rate(model, p, x);
    r.clamp(-limit.abs(), limit.abs())
}

/// The slope of an axis at the centre, in deg/s per full stick.
pub fn center_slope(model: RatesType, p: &RateAxis) -> f64 {
    let h = 1e-4;
    rate(model, p, h) / h
}

/// Deg/s at stick `i / STEPS`, for `i` in `0..=STEPS`.
pub fn sample(model: RatesType, p: &RateAxis, limit: f64) -> Vec<f64> {
    (0..=STEPS)
        .map(|i| rate_at(model, p, limit, i as f64 / STEPS as f64))
        .collect()
}

fn axis_view(i: usize, model: RatesType, p: &RateAxis, limit: f64) -> AxisView {
    AxisView {
        axis: AXES[i].into(),
        rc_rate: p.rc_rate,
        srate: p.srate,
        expo: p.expo,
        rate_limit: limit,
        max_deg_s: rate_at(model, p, limit, 1.0),
        center_deg_s: center_slope(model, p),
        curve: sample(model, p, limit),
    }
}

/// The throttle output (0-1) at stick `i / STEPS`.
pub fn throttle_samples(t: &ThrottleCurve) -> Vec<f64> {
    (0..=STEPS)
        .map(|i| t.apply(i as f64 / STEPS as f64))
        .collect()
}

fn limit_name(l: ThrottleLimit) -> &'static str {
    match l {
        ThrottleLimit::Off => "off",
        ThrottleLimit::Scale => "scale",
        ThrottleLimit::Clip => "clip",
    }
}

impl Profile {
    /// The chart view of this profile.
    pub fn view(
        &self,
        index: u8,
        name: Option<String>,
        active: bool,
        complete: bool,
    ) -> RateProfileView {
        let t = &self.throttle;
        RateProfileView {
            index,
            name,
            active,
            rates_type: type_name(self.rates.rates_type).into(),
            complete,
            axes: (0..3)
                .map(|i| {
                    axis_view(
                        i,
                        self.rates.rates_type,
                        &self.rates.axes[i],
                        self.limits[i],
                    )
                })
                .collect(),
            throttle: ThrottleView {
                mid: t.mid,
                expo: t.expo,
                hover: t.hover,
                limit: limit_name(t.limit).into(),
                limit_percent: t.limit_percent,
                curve: throttle_samples(t),
            },
        }
    }

    /// Largest difference in deg/s between this profile's curve and `other`'s, per axis.
    pub fn max_diff(&self, other: &Profile) -> [f64; 3] {
        let mut out = [0.0; 3];
        for (i, o) in out.iter_mut().enumerate() {
            let a = sample(self.rates.rates_type, &self.rates.axes[i], self.limits[i]);
            let b = sample(
                other.rates.rates_type,
                &other.rates.axes[i],
                other.limits[i],
            );
            *o = a
                .iter()
                .zip(&b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max);
        }
        out
    }
}

// ----- reading a dump -----

fn has(config: &Config, section: Section, name: &str) -> bool {
    config.sets().any(|(s, n, _)| s == section && n == name)
}

fn num(config: &Config, section: Section, name: &str) -> Option<f64> {
    config.get(section, name)?.trim().parse().ok()
}

/// Every rate profile in the text, in index order.
pub fn read(config: &Config, source: Vec<String>) -> RatesView {
    let mut indexes: Vec<u8> = config
        .lines
        .iter()
        .filter_map(|l| match l.section {
            Section::RateProfile(n) => Some(n),
            _ => None,
        })
        .collect();
    indexes.sort_unstable();
    indexes.dedup();
    let mut notes = Vec::new();
    let master_only = indexes.is_empty();
    if master_only {
        indexes.push(0);
    }
    // The last `rateprofile N` line is the one in use.
    let active = config.lines.iter().rev().find_map(|l| match l.cmd {
        Some(crate::gear::bf::dump::Cmd::Select(Section::RateProfile(n))) => Some(n),
        _ => None,
    });
    let mut profiles = Vec::new();
    for &n in &indexes {
        let section = if master_only {
            Section::Master
        } else {
            Section::RateProfile(n)
        };
        let (profile, complete) = read_profile(config, section);
        let name = config
            .get(section, "rateprofile_name")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "-");
        profiles.push(profile.view(n, name, active == Some(n), complete));
    }
    if profiles.iter().any(|p| !p.complete) {
        notes.push(
            "A diff lists only values that differ from the defaults. Values it leaves out read as Betaflight's defaults (Actual rates 7 / 67 / 54, throttle mid 50). Use a dump for exact curves."
                .into(),
        );
    }
    if active.is_none() && !master_only {
        notes.push("The source does not say which rate profile is in use.".into());
    }
    RatesView {
        source,
        firmware: config
            .version
            .as_ref()
            .map(|v| format!("{} {}", v.firmware, v.version)),
        active,
        profiles,
        notes,
    }
}

/// One profile's numbers and whether the source named every one of them.
fn read_profile(config: &Config, section: Section) -> (Profile, bool) {
    let mut complete = has(config, section, "rates_type");
    let rates_type = config
        .get(section, "rates_type")
        .and_then(type_of)
        .unwrap_or(RatesType::Actual);
    let mut axes = [DEFAULT_AXIS; 3];
    let mut limits = [MAX_RATE_DEG_S; 3];
    for (i, ax) in AXES.iter().enumerate() {
        for (field, slot) in [
            ("rc_rate", &mut axes[i].rc_rate),
            ("srate", &mut axes[i].srate),
            ("expo", &mut axes[i].expo),
        ] {
            let key = format!("{ax}_{field}");
            complete &= has(config, section, &key);
            if let Some(v) = num(config, section, &key) {
                *slot = v;
            }
        }
        if let Some(v) = num(config, section, &format!("{ax}_rate_limit")) {
            limits[i] = v;
        }
    }
    let mut t = ThrottleCurve::default();
    if let Some(v) = num(config, section, "thr_mid") {
        t.mid = v;
    }
    if let Some(v) = num(config, section, "thr_expo") {
        t.expo = v;
    }
    t.hover = num(config, section, "thr_hover");
    t.limit = match config
        .get(section, "throttle_limit_type")
        .map(|s| s.trim().to_ascii_uppercase())
        .as_deref()
    {
        Some("SCALE") => ThrottleLimit::Scale,
        Some("CLIP") => ThrottleLimit::Clip,
        _ => ThrottleLimit::Off,
    };
    if let Some(v) = num(config, section, "throttle_limit_percent") {
        t.limit_percent = v;
    }
    (
        Profile {
            rates: Rates { rates_type, axes },
            limits,
            throttle: t,
        },
        complete,
    )
}

/// The profile numbers behind `RateProfileView` (a view holds the same numbers; sims compare
/// curves, so they need them back).
pub fn profile_of(v: &RateProfileView) -> Profile {
    let mut axes = [DEFAULT_AXIS; 3];
    let mut limits = [MAX_RATE_DEG_S; 3];
    for (i, a) in v.axes.iter().take(3).enumerate() {
        axes[i] = RateAxis {
            rc_rate: a.rc_rate,
            srate: a.srate,
            expo: a.expo,
        };
        limits[i] = a.rate_limit;
    }
    Profile {
        rates: Rates {
            rates_type: type_of(&v.rates_type).unwrap_or(RatesType::Actual),
            axes,
        },
        limits,
        throttle: ThrottleCurve {
            mid: v.throttle.mid,
            expo: v.throttle.expo,
            hover: v.throttle.hover,
            limit: match v.throttle.limit.as_str() {
                "scale" => ThrottleLimit::Scale,
                "clip" => ThrottleLimit::Clip,
                _ => ThrottleLimit::Off,
            },
            limit_percent: v.throttle.limit_percent,
        },
    }
}

// ----- fitting one model to another -----

/// The result of a fit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
pub struct Fit {
    pub rc_rate: f64,
    pub srate: f64,
    pub expo: f64,
    /// Largest difference from the source curve (deg/s) over the stick range.
    pub max_diff: f64,
    /// `max_diff` as a share of the source's maximum rate (0-1).
    pub max_diff_share: f64,
}

/// The CLI range of each setting under a model: (min, max) for rc_rate, srate, expo.
fn bounds(model: RatesType) -> [(f64, f64); 3] {
    match model {
        RatesType::Betaflight => [(1.0, 255.0), (0.0, 99.0), (0.0, 100.0)],
        RatesType::Actual => [(1.0, 200.0), (1.0, 200.0), (0.0, 100.0)],
        RatesType::Quick => [(1.0, 255.0), (1.0, 200.0), (0.0, 100.0)],
        RatesType::Raceflight => [(1.0, 255.0), (0.0, 255.0), (0.0, 100.0)],
        RatesType::Kiss => [(1.0, 255.0), (0.0, 99.0), (0.0, 100.0)],
    }
}

fn axis_of(v: &[f64; 3]) -> RateAxis {
    RateAxis {
        rc_rate: v[0],
        srate: v[1],
        expo: v[2],
    }
}

/// Stick points the fit compares: 0 to 1 in steps of 0.01.
fn fit_points() -> Vec<f64> {
    (0..=100).map(|i| i as f64 / 100.0).collect()
}

fn cost(model: RatesType, v: &[f64; 3], target: &[f64], xs: &[f64], limit: f64) -> f64 {
    let p = axis_of(v);
    xs.iter()
        .zip(target)
        .map(|(&x, &t)| {
            let d = rate_at(model, &p, limit, x) - t;
            d * d
        })
        .sum()
}

fn clamp_to(model: RatesType, v: &mut [f64; 3]) {
    for (x, (lo, hi)) in v.iter_mut().zip(bounds(model)) {
        *x = x.clamp(lo, hi);
    }
}

/// Finds the whole-number settings of `to` whose curve is closest to `from`'s (axis
/// `p`, limit `limit`). Deterministic: a fixed set of starts, Nelder-Mead on the clamped
/// settings, then a whole-number search around the best.
pub fn fit(from: RatesType, p: &RateAxis, limit: f64, to: RatesType) -> Fit {
    let xs = fit_points();
    let target: Vec<f64> = xs.iter().map(|&x| rate_at(from, p, limit, x)).collect();
    let max = target.last().copied().unwrap_or(0.0).abs().max(1.0);
    let b = bounds(to);
    let c = |v: &[f64; 3]| {
        let mut w = *v;
        clamp_to(to, &mut w);
        cost(to, &w, &target, &xs, limit)
    };
    let mut best = ([0.0; 3], f64::MAX);
    // Starts spread over the model's range; the centre slope pins rc_rate roughly.
    let center = center_slope(from, p).abs().max(1.0);
    let mut starts: Vec<[f64; 3]> = Vec::new();
    for &e in &[0.0, 0.3, 0.6] {
        for &s in &[0.3, 0.6, 0.85] {
            let rc = match to {
                RatesType::Actual => center / 10.0,
                RatesType::Quick => center / 2.0,
                _ => center / 200.0 * 100.0,
            };
            starts.push([
                rc.clamp(b[0].0, b[0].1),
                b[1].0 + (b[1].1 - b[1].0) * s,
                b[2].1 * e,
            ]);
        }
    }
    for s in starts {
        let (v, f) = nelder_mead(&c, s);
        if f < best.1 {
            best = (v, f);
        }
    }
    let mut v = best.0;
    clamp_to(to, &mut v);
    v = v.map(f64::round);
    clamp_to(to, &mut v);
    // Whole-number search: step one setting by one while the cost drops.
    let mut cur = cost(to, &v, &target, &xs, limit);
    loop {
        let mut improved = false;
        for i in 0..3 {
            for d in [-1.0, 1.0] {
                let mut w = v;
                w[i] += d;
                clamp_to(to, &mut w);
                let f = cost(to, &w, &target, &xs, limit);
                if f + 1e-9 < cur {
                    v = w;
                    cur = f;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    let q = axis_of(&v);
    let max_diff = xs
        .iter()
        .zip(&target)
        .map(|(&x, &t)| (rate_at(to, &q, limit, x) - t).abs())
        .fold(0.0, f64::max);
    Fit {
        rc_rate: v[0],
        srate: v[1],
        expo: v[2],
        max_diff,
        max_diff_share: max_diff / max,
    }
}

/// Nelder-Mead on three variables from `start`; returns the best point and its cost.
fn nelder_mead(f: &dyn Fn(&[f64; 3]) -> f64, start: [f64; 3]) -> ([f64; 3], f64) {
    let mut pts: Vec<([f64; 3], f64)> = Vec::new();
    pts.push((start, f(&start)));
    for i in 0..3 {
        let mut p = start;
        p[i] += if p[i].abs() > 1.0 { p[i] * 0.2 } else { 5.0 };
        pts.push((p, f(&p)));
    }
    for _ in 0..400 {
        pts.sort_by(|a, b| a.1.total_cmp(&b.1));
        if (pts[3].1 - pts[0].1).abs() < 1e-9 {
            break;
        }
        let mut cen = [0.0; 3];
        for p in &pts[..3] {
            for (c, x) in cen.iter_mut().zip(p.0) {
                *c += x / 3.0;
            }
        }
        let at = |t: f64| -> [f64; 3] {
            let w = pts[3].0;
            [
                cen[0] + t * (cen[0] - w[0]),
                cen[1] + t * (cen[1] - w[1]),
                cen[2] + t * (cen[2] - w[2]),
            ]
        };
        let r = at(1.0);
        let fr = f(&r);
        if fr < pts[0].1 {
            let e = at(2.0);
            let fe = f(&e);
            pts[3] = if fe < fr { (e, fe) } else { (r, fr) };
        } else if fr < pts[2].1 {
            pts[3] = (r, fr);
        } else {
            let c = if fr < pts[3].1 { at(0.5) } else { at(-0.5) };
            let fc = f(&c);
            if fc < pts[3].1.min(fr) {
                pts[3] = (c, fc);
            } else {
                let b = pts[0].0;
                for p in pts.iter_mut().skip(1) {
                    for (x, y) in p.0.iter_mut().zip(b) {
                        *x = y + 0.5 * (*x - y);
                    }
                    p.1 = f(&p.0);
                }
            }
        }
    }
    pts.sort_by(|a, b| a.1.total_cmp(&b.1));
    pts[0]
}

/// The profile as a sim would take it: every axis fitted onto the Betaflight model. A
/// profile already on the Betaflight model comes back unchanged with zero error.
pub fn to_betaflight(p: &Profile) -> (Profile, [Fit; 3]) {
    let mut out = *p;
    out.rates.rates_type = RatesType::Betaflight;
    let mut fits = [Fit {
        rc_rate: 0.0,
        srate: 0.0,
        expo: 0.0,
        max_diff: 0.0,
        max_diff_share: 0.0,
    }; 3];
    for (i, (fit_slot, a)) in fits.iter_mut().zip(p.rates.axes).enumerate() {
        if p.rates.rates_type == RatesType::Betaflight {
            *fit_slot = Fit {
                rc_rate: a.rc_rate,
                srate: a.srate,
                expo: a.expo,
                max_diff: 0.0,
                max_diff_share: 0.0,
            };
        } else {
            *fit_slot = fit(p.rates.rates_type, &a, p.limits[i], RatesType::Betaflight);
            out.rates.axes[i] = RateAxis {
                rc_rate: fit_slot.rc_rate,
                srate: fit_slot.srate,
                expo: fit_slot.expo,
            };
        }
    }
    (out, fits)
}

/// Throttle curves differ by more than a percent of throttle at any sample.
pub fn throttle_differs(a: &ThrottleCurve, b: &ThrottleCurve) -> bool {
    throttle_samples(a)
        .iter()
        .zip(throttle_samples(b))
        .any(|(x, y)| (x - y).abs() > 0.01)
}

/// A text rendering of a view for the CLI and MCP.
pub fn render_text(v: &RatesView) -> String {
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    line(format!(
        "Rates{}{}",
        v.firmware
            .as_deref()
            .map(|f| format!(" ({f})"))
            .unwrap_or_default(),
        if v.source.is_empty() {
            String::new()
        } else {
            format!(" from {}", v.source.join(", "))
        }
    ));
    for p in &v.profiles {
        line(format!(
            "Rate profile {}{}{} - {}{}",
            p.index,
            p.name
                .as_deref()
                .map(|n| format!(" {n}"))
                .unwrap_or_default(),
            if p.active { " (in use)" } else { "" },
            p.rates_type,
            if p.complete {
                ""
            } else {
                ", values from defaults"
            }
        ));
        for a in &p.axes {
            line(format!(
                "  {:<5} rc {} super {} expo {}  centre {:.0} deg/s  max {:.0} deg/s",
                a.axis, a.rc_rate, a.srate, a.expo, a.center_deg_s, a.max_deg_s
            ));
        }
        let t = &p.throttle;
        line(format!(
            "  throttle mid {} expo {}{} limit {} {}",
            t.mid,
            t.expo,
            t.hover.map(|h| format!(" hover {h}")).unwrap_or_default(),
            t.limit,
            t.limit_percent
        ));
    }
    for n in &v.notes {
        line(n.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "\
# Betaflight / STM32G47X (G473) 2025.12.5 Jun 25 2026 / 03:24:50 (eb2bb5a33) MSP API: 1.47
rateprofile 0

# rateprofile 0
set rateprofile_name = FREE
set thr_mid = 30
set thr_expo = 50
set thr_hover = 34
set rates_type = BETAFLIGHT
set roll_rc_rate = 127
set pitch_rc_rate = 127
set yaw_rc_rate = 100
set roll_expo = 40
set pitch_expo = 40
set yaw_expo = 0
set roll_srate = 72
set pitch_srate = 72
set yaw_srate = 75
set throttle_limit_type = OFF
set throttle_limit_percent = 100
set roll_rate_limit = 1998
set pitch_rate_limit = 1998
set yaw_rate_limit = 1998

rateprofile 1

# rateprofile 1
set rateprofile_name = CINE
set rates_type = ACTUAL
set roll_rc_rate = 7
set pitch_rc_rate = 7
set yaw_rc_rate = 5
set roll_expo = 54
set pitch_expo = 54
set yaw_expo = 54
set roll_srate = 30
set pitch_srate = 30
set yaw_srate = 20
set throttle_limit_type = CLIP
set throttle_limit_percent = 80
set roll_rate_limit = 500
set pitch_rate_limit = 500
set yaw_rate_limit = 400

rateprofile 1
";

    fn near(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn reads_every_profile_with_names_and_the_active_one() {
        let v = read(&Config::parse(DUMP), vec!["dump".into()]);
        assert_eq!(v.profiles.len(), 2);
        assert_eq!(v.active, Some(1));
        let (a, b) = (&v.profiles[0], &v.profiles[1]);
        assert_eq!(a.name.as_deref(), Some("FREE"));
        assert_eq!(b.name.as_deref(), Some("CINE"));
        assert!(!a.active && b.active);
        assert_eq!(a.rates_type, "betaflight");
        assert_eq!(b.rates_type, "actual");
        assert!(a.complete && b.complete);
        assert_eq!(v.firmware.as_deref(), Some("Betaflight 2025.12.5"));
        // Configurator: RC 1.27, super 0.72, expo 0.40 -> 907 deg/s at full stick.
        near(a.axes[0].max_deg_s, 907.14, 0.01);
        // Actual: centre 70, max 300, limited to 500 (not reached).
        near(b.axes[0].center_deg_s, 70.0, 0.1);
        near(b.axes[0].max_deg_s, 300.0, 1e-9);
        // The axis limit cuts the curve: yaw max 200 under a 400 limit stays 200.
        near(b.axes[2].max_deg_s, 200.0, 1e-9);
        assert_eq!(a.throttle.curve.len(), STEPS + 1);
        near(a.throttle.curve[STEPS], 1.0, 1e-12);
        assert_eq!(b.throttle.limit, "clip");
        near(b.throttle.curve[STEPS], 0.8, 1e-12);
    }

    #[test]
    fn rate_limit_clips_the_curve() {
        let p = RateAxis {
            rc_rate: 100.0,
            srate: 70.0,
            expo: 0.0,
        };
        near(rate_at(RatesType::Betaflight, &p, 400.0, 1.0), 400.0, 1e-9);
        near(
            rate_at(RatesType::Betaflight, &p, 400.0, -1.0),
            -400.0,
            1e-9,
        );
        near(
            rate_at(RatesType::Betaflight, &p, 400.0, 0.2),
            40.0 / 0.86,
            0.01,
        );
    }

    #[test]
    fn a_diff_reads_missing_values_as_defaults_and_says_so() {
        let v = read(
            &Config::parse("rateprofile 0\nset rates_type = QUICK\nset roll_srate = 50\n"),
            Vec::new(),
        );
        let p = &v.profiles[0];
        assert!(!p.complete);
        assert_eq!(p.rates_type, "quick");
        assert_eq!(p.axes[0].srate, 50.0);
        assert_eq!(p.axes[1].srate, 67.0);
        assert_eq!(p.throttle.mid, 50.0);
        assert!(v.notes.iter().any(|n| n.contains("dump")));
    }

    #[test]
    fn cli_lines_without_a_profile_read_as_profile_zero() {
        let v = read(&Config::parse("set roll_rc_rate = 9\n"), Vec::new());
        assert_eq!(v.profiles.len(), 1);
        assert_eq!(v.profiles[0].index, 0);
        assert_eq!(v.profiles[0].axes[0].rc_rate, 9.0);
    }

    #[test]
    fn betaflight_actual_and_quick_curves_match_reference_values() {
        // Hand-worked points on the published formulas (the sim crate's tests hold more).
        let bf = RateAxis {
            rc_rate: 100.0,
            srate: 70.0,
            expo: 0.0,
        };
        near(rate(RatesType::Betaflight, &bf, 1.0), 666.67, 0.01);
        let act = RateAxis {
            rc_rate: 7.0,
            srate: 67.0,
            expo: 54.0,
        };
        near(rate(RatesType::Actual, &act, 1.0), 670.0, 1e-9);
        near(
            rate(RatesType::Actual, &act, 0.5),
            35.0 + 600.0 * 0.1234375,
            1e-9,
        );
        let q = RateAxis {
            rc_rate: 100.0,
            srate: 67.0,
            expo: 0.0,
        };
        near(rate(RatesType::Quick, &q, 1.0), 670.0, 1e-9);
        near(center_slope(RatesType::Quick, &q), 200.0, 0.5);
    }

    #[test]
    fn throttle_curve_reference_points() {
        let t = ThrottleCurve {
            mid: 30.0,
            expo: 50.0,
            hover: Some(34.0),
            ..ThrottleCurve::default()
        };
        let s = throttle_samples(&t);
        near(s[0], 0.0, 1e-12);
        near(s[STEPS], 1.0, 1e-12);
        // Stick 0.3 is the mid point: the output is the hover value.
        near(s[15], 0.34, 1e-12);
    }

    #[test]
    fn fit_onto_the_same_model_finds_the_same_numbers() {
        let p = RateAxis {
            rc_rate: 127.0,
            srate: 72.0,
            expo: 40.0,
        };
        let f = fit(
            RatesType::Betaflight,
            &p,
            MAX_RATE_DEG_S,
            RatesType::Betaflight,
        );
        near(f.max_diff, 0.0, 0.5);
        near(f.rc_rate, 127.0, 1.0);
        near(f.srate, 72.0, 1.0);
        near(f.expo, 40.0, 2.0);
    }

    #[test]
    fn actual_default_fits_the_betaflight_model() {
        let act = RateAxis {
            rc_rate: 7.0,
            srate: 67.0,
            expo: 54.0,
        };
        let f = fit(
            RatesType::Actual,
            &act,
            MAX_RATE_DEG_S,
            RatesType::Betaflight,
        );
        assert!(
            f.max_diff_share <= FIT_TOLERANCE,
            "{f:?} over {FIT_TOLERANCE}"
        );
        // The fitted settings end at the same maximum.
        let q = axis_of(&[f.rc_rate, f.srate, f.expo]);
        near(rate(RatesType::Betaflight, &q, 1.0), 670.0, 20.0);
    }

    #[test]
    fn betaflight_fits_back_onto_actual_and_quick() {
        let bf = RateAxis {
            rc_rate: 127.0,
            srate: 72.0,
            expo: 40.0,
        };
        for to in [RatesType::Actual, RatesType::Quick] {
            let f = fit(RatesType::Betaflight, &bf, MAX_RATE_DEG_S, to);
            assert!(f.max_diff_share <= FIT_TOLERANCE, "{to:?} {f:?}");
        }
    }

    #[test]
    fn fit_stays_within_tolerance() {
        // The stated grid: centre 40-200 deg/s, max 300-1000, expo 0-60.
        let mut worst: f64 = 0.0;
        for center in [40.0, 70.0, 120.0, 200.0] {
            for max in [300.0, 500.0, 700.0, 1000.0] {
                for expo in [0.0, 25.0, 54.0, 60.0] {
                    if max <= center {
                        continue;
                    }
                    let a = RateAxis {
                        rc_rate: center / 10.0,
                        srate: max / 10.0,
                        expo,
                    };
                    let f = fit(RatesType::Actual, &a, MAX_RATE_DEG_S, RatesType::Betaflight);
                    worst = worst.max(f.max_diff_share);
                    assert!(
                        f.max_diff_share <= FIT_TOLERANCE,
                        "actual {center}/{max}/{expo}: {f:?}"
                    );
                }
            }
        }
        assert!(worst > 0.0);
    }

    #[test]
    fn to_betaflight_keeps_a_betaflight_profile_and_fits_others() {
        let v = read(&Config::parse(DUMP), Vec::new());
        let bf = profile_of(&v.profiles[0]);
        let (same, fits) = to_betaflight(&bf);
        assert_eq!(same.rates, bf.rates);
        assert!(fits.iter().all(|f| f.max_diff == 0.0));
        let act = profile_of(&v.profiles[1]);
        let (conv, fits) = to_betaflight(&act);
        assert_eq!(conv.rates.rates_type, RatesType::Betaflight);
        assert!(fits.iter().all(|f| f.max_diff_share <= FIT_TOLERANCE));
        // The converted curve sits within the fit error of the original.
        let d = act.max_diff(&conv);
        assert!(d.iter().all(|&x| x <= 0.03 * 700.0));
    }

    #[test]
    fn max_diff_is_zero_for_equal_profiles() {
        let v = read(&Config::parse(DUMP), Vec::new());
        let p = profile_of(&v.profiles[0]);
        assert_eq!(p.max_diff(&p), [0.0; 3]);
        assert!(!throttle_differs(&p.throttle, &p.throttle));
        let q = profile_of(&v.profiles[1]);
        assert!(p.max_diff(&q)[0] > 100.0);
        assert!(throttle_differs(&p.throttle, &q.throttle));
    }

    #[test]
    fn text_names_profiles_and_the_one_in_use() {
        let t = render_text(&read(&Config::parse(DUMP), vec!["quad".into()]));
        assert!(t.contains("Rate profile 1 CINE (in use) - actual"), "{t}");
        assert!(t.contains("Rate profile 0 FREE - betaflight"));
    }
}
