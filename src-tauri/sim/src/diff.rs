//! The flight-controller settings the sim takes from a quad's Betaflight `diff all`: rates,
//! throttle curve, airmode, angle and horizon, arming guards, crashflip, idle and the `aux`
//! mode ranges (sim-design 5.1, 5.4, 5.5, 6.1).
//!
//! The sim crate cannot depend on the app crate, so this is a small parser of its own. It
//! reads only `set`, `feature`, `aux`, `profile` and `rateprofile` lines. Setting names and
//! defaults are Betaflight's published CLI names and defaults (fact).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rates::{RateAxis, Rates, RatesType, ThrottleCurve, ThrottleLimit};

/// Betaflight mode ids the sim acts on (the `aux` line's second number).
pub mod mode {
    pub const ARM: u32 = 0;
    pub const ANGLE: u32 = 1;
    pub const HORIZON: u32 = 2;
    pub const AIRMODE: u32 = 28;
    pub const FLIP_OVER_AFTER_CRASH: u32 = 35;
}

/// One `aux` range: mode `mode_id` is on while channel `ch` (0-based; 4 is AUX1) is in
/// `start..end` µs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxRange {
    pub mode_id: u32,
    pub ch: usize,
    pub start: u16,
    pub end: u16,
}

impl AuxRange {
    pub fn active(&self, ch: &[u16]) -> bool {
        let v = ch.get(self.ch).copied().unwrap_or(1500);
        // Betaflight's top step is 2100; a range ending there takes everything above.
        v >= self.start && (v < self.end || self.end >= 2100)
    }
}

/// The quad's own PIDs, kept for reference. They do not drive the sim (sim-design 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct BfPid {
    pub p: Option<u16>,
    pub i: Option<u16>,
    pub d: Option<u16>,
    pub f: Option<u16>,
}

/// Everything the sim reads from a `diff all`. `Default` is Betaflight's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FcConfig {
    pub rates: Rates,
    pub throttle: ThrottleCurve,
    /// `feature AIRMODE` (on by default); an AIR MODE aux range overrides it.
    pub airmode: bool,
    /// `airmode_start_throttle_percent`.
    pub airmode_start_throttle: f64,
    /// `angle_limit` (deg).
    pub angle_limit: f64,
    /// `angle_p_gain`: the level gain.
    pub angle_p_gain: f64,
    /// `horizon_level_strength`, `horizon_limit_sticks` (%), `horizon_limit_degrees`.
    pub horizon_level_strength: f64,
    pub horizon_limit_sticks: f64,
    pub horizon_limit_degrees: f64,
    /// `small_angle` (deg): no arming tilted past this.
    pub small_angle: f64,
    /// `min_check` (µs): arming needs the throttle below it.
    pub min_check: u16,
    /// `dshot_idle_value` (0.01 %): the ESC duty at zero throttle.
    pub dshot_idle_value: f64,
    pub motor_poles: u32,
    /// `crashflip_motor_percent`, `crashflip_expo`.
    pub crashflip_motor_percent: f64,
    pub crashflip_expo: f64,
    pub yaw_motors_reversed: bool,
    pub aux: Vec<AuxRange>,
    /// Roll, pitch, yaw.
    pub pids: [BfPid; 3],
    /// Every setting key the diff set, so a profile can mark those values `diff`.
    pub keys: Vec<String>,
}

impl Default for FcConfig {
    fn default() -> FcConfig {
        FcConfig {
            rates: Rates::default(),
            throttle: ThrottleCurve::default(),
            airmode: true,
            airmode_start_throttle: 25.0,
            angle_limit: 60.0,
            angle_p_gain: 50.0,
            horizon_level_strength: 75.0,
            horizon_limit_sticks: 75.0,
            horizon_limit_degrees: 135.0,
            small_angle: 25.0,
            min_check: 1050,
            dshot_idle_value: 550.0,
            motor_poles: 14,
            crashflip_motor_percent: 0.0,
            crashflip_expo: 35.0,
            yaw_motors_reversed: false,
            aux: vec![
                AuxRange {
                    mode_id: mode::ARM,
                    ch: 4,
                    start: 1700,
                    end: 2100,
                },
                AuxRange {
                    mode_id: mode::ANGLE,
                    ch: 5,
                    start: 1700,
                    end: 2100,
                },
            ],
            pids: [BfPid::default(); 3],
            keys: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Sections {
    global: BTreeMap<String, String>,
    pid: BTreeMap<u32, BTreeMap<String, String>>,
    rate: BTreeMap<u32, BTreeMap<String, String>>,
}

impl FcConfig {
    /// Read a `diff all` (or `dump all`) text. Unknown lines are ignored; a missing setting
    /// keeps Betaflight's default.
    pub fn from_diff(text: &str) -> FcConfig {
        let mut s = Sections::default();
        enum Ctx {
            Global,
            Pid(u32),
            Rate(u32),
        }
        let mut ctx = Ctx::Global;
        let (mut sel_pid, mut sel_rate) = (0u32, 0u32);
        let mut features: BTreeMap<String, bool> = BTreeMap::new();
        let mut aux: BTreeMap<u32, AuxRange> = BTreeMap::new();
        let mut aux_seen = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let w: Vec<&str> = line.split_whitespace().collect();
            match w[0] {
                "profile" => {
                    if let Some(n) = w.get(1).and_then(|x| x.parse().ok()) {
                        ctx = Ctx::Pid(n);
                        sel_pid = n;
                    }
                }
                "rateprofile" => {
                    if let Some(n) = w.get(1).and_then(|x| x.parse().ok()) {
                        ctx = Ctx::Rate(n);
                        sel_rate = n;
                    }
                }
                "feature" => {
                    if let Some(f) = w.get(1) {
                        let (on, name) = match f.strip_prefix('-') {
                            Some(n) => (false, n),
                            None => (true, *f),
                        };
                        features.insert(name.to_ascii_uppercase(), on);
                    }
                }
                "aux" => {
                    aux_seen = true;
                    let n = |i: usize| w.get(i).and_then(|x| x.parse::<u32>().ok());
                    if let (Some(slot), Some(mode_id), Some(ch), Some(start), Some(end)) =
                        (n(1), n(2), n(3), n(4), n(5))
                    {
                        aux.insert(
                            slot,
                            AuxRange {
                                mode_id,
                                ch: ch as usize + 4,
                                start: start as u16,
                                end: end as u16,
                            },
                        );
                    }
                }
                "set" => {
                    let rest = line[3..].trim();
                    if let Some((k, v)) = rest.split_once('=') {
                        let k = k.trim().to_ascii_lowercase();
                        let v = v.trim().to_string();
                        let map = match ctx {
                            Ctx::Global => &mut s.global,
                            Ctx::Pid(n) => s.pid.entry(n).or_default(),
                            Ctx::Rate(n) => s.rate.entry(n).or_default(),
                        };
                        map.insert(k, v);
                    }
                }
                _ => {}
            }
        }

        let empty = BTreeMap::new();
        let rate_map = s.rate.get(&sel_rate).unwrap_or(&empty);
        let pid_map = s.pid.get(&sel_pid).unwrap_or(&empty);
        let mut keys = Vec::new();
        let mut get = |k: &str| -> Option<String> {
            let v = rate_map
                .get(k)
                .or_else(|| pid_map.get(k))
                .or_else(|| s.global.get(k))
                .cloned();
            if v.is_some() {
                keys.push(k.to_string());
            }
            v
        };
        let mut c = FcConfig::default();
        let num = |v: Option<String>| v.and_then(|x| x.parse::<f64>().ok());

        if let Some(t) = get("rates_type").and_then(|v| RatesType::from_cli(&v)) {
            c.rates.rates_type = t;
            if t != RatesType::Actual {
                // A diff only lists values that differ from the defaults, and the defaults
                // above are Actual's: start other models from Betaflight's own defaults.
                let d = RateAxis {
                    rc_rate: 100.0,
                    srate: 70.0,
                    expo: 0.0,
                };
                c.rates.axes = [d; 3];
                if t == RatesType::Quick {
                    c.rates.axes = [RateAxis {
                        rc_rate: 100.0,
                        srate: 67.0,
                        expo: 0.0,
                    }; 3];
                }
            }
        }
        for (i, ax) in ["roll", "pitch", "yaw"].iter().enumerate() {
            if let Some(v) = num(get(&format!("{ax}_rc_rate"))) {
                c.rates.axes[i].rc_rate = v;
            }
            if let Some(v) = num(get(&format!("{ax}_srate"))) {
                c.rates.axes[i].srate = v;
            }
            if let Some(v) = num(get(&format!("{ax}_expo"))) {
                c.rates.axes[i].expo = v;
            }
            let pid = |k: &str, get: &mut dyn FnMut(&str) -> Option<String>| {
                get(&format!("{k}_{ax}")).and_then(|x| x.parse::<u16>().ok())
            };
            c.pids[i] = BfPid {
                p: pid("p", &mut get),
                i: pid("i", &mut get),
                d: pid("d", &mut get),
                f: pid("f", &mut get),
            };
        }
        if let Some(v) = num(get("thr_mid")) {
            c.throttle.mid = v;
        }
        if let Some(v) = num(get("thr_expo")) {
            c.throttle.expo = v;
        }
        c.throttle.hover = num(get("thr_hover"));
        if let Some(v) = get("throttle_limit_type") {
            c.throttle.limit = match v.to_ascii_uppercase().as_str() {
                "SCALE" => ThrottleLimit::Scale,
                "CLIP" => ThrottleLimit::Clip,
                _ => ThrottleLimit::Off,
            };
        }
        if let Some(v) = num(get("throttle_limit_percent")) {
            c.throttle.limit_percent = v;
        }
        if let Some(v) = num(get("angle_limit")).or_else(|| num(get("level_limit"))) {
            c.angle_limit = v;
        }
        if let Some(v) = num(get("angle_p_gain")) {
            c.angle_p_gain = v;
        }
        if let Some(v) = num(get("horizon_level_strength")) {
            c.horizon_level_strength = v;
        }
        if let Some(v) = num(get("horizon_limit_sticks")) {
            c.horizon_limit_sticks = v;
        }
        if let Some(v) = num(get("horizon_limit_degrees")) {
            c.horizon_limit_degrees = v;
        }
        if let Some(v) = num(get("small_angle")) {
            c.small_angle = v;
        }
        if let Some(v) = num(get("min_check")) {
            c.min_check = v as u16;
        }
        if let Some(v) = num(get("dshot_idle_value")) {
            c.dshot_idle_value = v;
        }
        if let Some(v) = num(get("motor_poles")) {
            c.motor_poles = v as u32;
        }
        if let Some(v) = num(get("crashflip_motor_percent")) {
            c.crashflip_motor_percent = v;
        }
        if let Some(v) = num(get("crashflip_expo")) {
            c.crashflip_expo = v;
        }
        if let Some(v) = num(get("airmode_start_throttle_percent")) {
            c.airmode_start_throttle = v;
        }
        if let Some(v) = get("yaw_motors_reversed") {
            c.yaw_motors_reversed = v.eq_ignore_ascii_case("ON");
        }
        if let Some(on) = features.get("AIRMODE") {
            c.airmode = *on;
            keys.push("feature AIRMODE".into());
        }
        if aux_seen {
            c.aux = aux
                .into_values()
                .filter(|a| a.start < a.end)
                .collect::<Vec<_>>();
            keys.push("aux".into());
        }
        keys.sort();
        keys.dedup();
        c.keys = keys;
        c
    }

    /// The range for a mode, if the quad has one.
    pub fn ranges(&self, mode_id: u32) -> impl Iterator<Item = &AuxRange> {
        self.aux.iter().filter(move |a| a.mode_id == mode_id)
    }

    /// Mode `mode_id` is on for these channel values (any of its ranges).
    pub fn mode_on(&self, mode_id: u32, ch: &[u16]) -> bool {
        self.ranges(mode_id).any(|a| a.active(ch))
    }

    pub fn has_mode(&self, mode_id: u32) -> bool {
        self.ranges(mode_id).next().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
# version
# Betaflight / STM32F411 (S411) 2025.12.0
batch start
feature -AIRMODE
feature TELEMETRY
aux 0 0 0 1700 2100 0 0
aux 1 1 4 1700 2100 0 0
aux 2 35 1 1700 2100 0 0
aux 3 13 1 1300 1700 0 0
aux 4 0 2 900 900 0 0
set motor_poles = 12
set small_angle = 180
set dshot_idle_value = 600
profile 0
set p_roll = 52
set angle_limit = 55
profile 1
set angle_limit = 30
profile 0
rateprofile 0
set thr_mid = 30
set thr_hover = 34
set rates_type = BETAFLIGHT
set roll_rc_rate = 127
set roll_srate = 72
set roll_expo = 40
rateprofile 1
set rates_type = ACTUAL
set roll_rc_rate = 20
rateprofile 0
";

    #[test]
    fn reads_the_selected_profiles_and_globals() {
        let c = FcConfig::from_diff(DIFF);
        assert_eq!(c.rates.rates_type, RatesType::Betaflight);
        assert_eq!(c.rates.axes[0].rc_rate, 127.0);
        assert_eq!(c.rates.axes[0].srate, 72.0);
        assert_eq!(c.rates.axes[0].expo, 40.0);
        // Pitch was not in the diff: Betaflight-model defaults.
        assert_eq!(c.rates.axes[1].rc_rate, 100.0);
        assert_eq!(c.throttle.mid, 30.0);
        assert_eq!(c.throttle.hover, Some(34.0));
        assert_eq!(c.angle_limit, 55.0);
        assert_eq!(c.small_angle, 180.0);
        assert_eq!(c.dshot_idle_value, 600.0);
        assert_eq!(c.motor_poles, 12);
        assert!(!c.airmode);
        assert_eq!(c.pids[0].p, Some(52));
        assert!(c.keys.contains(&"roll_rc_rate".to_string()));
        // The empty aux range (900..900) is not a range.
        assert_eq!(c.aux.len(), 4);
        let arm = c.ranges(mode::ARM).next().unwrap();
        assert_eq!((arm.ch, arm.start, arm.end), (4, 1700, 2100));
        let flip = c.ranges(mode::FLIP_OVER_AFTER_CRASH).next().unwrap();
        assert_eq!(flip.ch, 5);
        let mut ch = [1500u16; 16];
        ch[4] = 2000;
        assert!(c.mode_on(mode::ARM, &ch));
        ch[4] = 1000;
        assert!(!c.mode_on(mode::ARM, &ch));
    }

    #[test]
    fn empty_diff_is_betaflight_defaults() {
        let c = FcConfig::from_diff("");
        assert_eq!(c, FcConfig::default());
        assert!(c.airmode);
        assert_eq!(c.rates.rates_type, RatesType::Actual);
    }
}
