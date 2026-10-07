//! The validation harness (sim-design 6.4): replay a quad's logged sticks and motor
//! commands through the sim and compare the result with the log, check by check, each
//! against its band.
//!
//! | Check | Mode | Band |
//! |---|---|---|
//! | Hover | Closed loop, a height-holding pilot at the log's hover voltage | command ±0.02, speed ±5 % |
//! | Punch | Logged motor commands and voltage, body held | peak vertical accel ±15 %, rise to 90 % ±15 ms, peak current ±10 % |
//! | Sag | Logged motor commands, sag on | minimum voltage ±0.1 V |
//! | Roll, pitch | Closed loop, logged setpoint and throttle | lag ±3 ms, overshoot ±10 points |
//! | Yaw | Closed loop | lag ±5 ms |
//! | Coast-down | Closed loop | deceleration ±15 % (needs speed in the log) |
//! | Fall recovery | Closed loop | height lost ±20 % |

use rapier3d_f64::glamx::{DQuat, DVec3};
use serde::Serialize;

use crate::aero::{body_drag, inflow_factor};
use crate::battery::Battery;
use crate::diff::{mode, AuxRange};
use crate::fc::Override;
use crate::filters::Pt1;
use crate::log::LogData;
use crate::motor::Motors;
use crate::profile::{SimProfile, G, RPM_TO_RADS};
use crate::rates::ThrottleCurve;
use crate::ring::{RcFrame, Sticks};
use crate::world::WorldSpec;
use crate::{Sim, SimSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    /// The log has no window for this check.
    NotInLog,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    /// `hover`, `punch`, `sag`, `roll`, `pitch`, `yaw`, `coast_down`, `fall_recovery`.
    pub check: String,
    /// What is compared: `command`, `speed`, `peak_accel`, `rise_90`, ...
    pub measure: String,
    pub log: String,
    /// Window in the log (s).
    pub window: Option<(f64, f64)>,
    pub logged: f64,
    pub sim: f64,
    /// Allowed difference; `relative` bands are fractions of the logged value.
    pub band: f64,
    pub relative: bool,
    pub unit: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationReport {
    pub profile: String,
    pub logs: Vec<String>,
    pub checks: Vec<CheckResult>,
}

impl ValidationReport {
    /// Every check that ran passed, and at least one ran.
    pub fn passed(&self) -> bool {
        self.checks.iter().any(|c| c.outcome == Outcome::Pass)
            && self.checks.iter().all(|c| c.outcome != Outcome::Fail)
    }

    pub fn failures(&self) -> Vec<&CheckResult> {
        self.checks
            .iter()
            .filter(|c| c.outcome == Outcome::Fail)
            .collect()
    }

    /// A plain-text table.
    pub fn table(&self) -> String {
        let mut s = format!(
            "Validation of {} on {} log(s)\n",
            self.profile,
            self.logs.len()
        );
        s.push_str("check          measure      log                        window          logged      sim         band     result\n");
        for c in &self.checks {
            let w = c
                .window
                .map(|(a, b)| format!("{a:.2}-{b:.2} s"))
                .unwrap_or_else(|| "-".into());
            let band = if c.relative {
                format!("±{:.0} %", c.band * 100.0)
            } else {
                format!("±{} {}", c.band, c.unit)
            };
            let res = match c.outcome {
                Outcome::Pass => "pass",
                Outcome::Fail => "FAIL",
                Outcome::NotInLog => "not in log",
            };
            s.push_str(&format!(
                "{:<14} {:<12} {:<26} {:<15} {:>10.3} {:>10.3}  {:<8} {}\n",
                c.check, c.measure, c.log, w, c.logged, c.sim, band, res
            ));
        }
        s.push_str(if self.passed() { "PASS\n" } else { "FAIL\n" });
        s
    }
}

#[allow(clippy::too_many_arguments)]
fn result(
    check: &str,
    measure: &str,
    log: &str,
    window: Option<(f64, f64)>,
    logged: f64,
    sim: f64,
    band: f64,
    relative: bool,
    unit: &str,
) -> CheckResult {
    let diff = (sim - logged).abs();
    let ok = if relative {
        diff <= band * logged.abs()
    } else {
        diff <= band + 1e-9
    };
    CheckResult {
        check: check.into(),
        measure: measure.into(),
        log: log.into(),
        window,
        logged,
        sim,
        band,
        relative,
        unit: unit.into(),
        outcome: if ok { Outcome::Pass } else { Outcome::Fail },
    }
}

fn not_in_log(check: &str, log: &str) -> CheckResult {
    CheckResult {
        check: check.into(),
        measure: "-".into(),
        log: log.into(),
        window: None,
        logged: 0.0,
        sim: 0.0,
        band: 0.0,
        relative: false,
        unit: String::new(),
        outcome: Outcome::NotInLog,
    }
}

/// Validate `profile` against decoded logs.
pub fn validate(profile: &SimProfile, logs: &[LogData]) -> ValidationReport {
    let hp = harness_profile(profile);
    let mut checks = Vec::new();
    for log in logs {
        checks.extend(hover(&hp, log));
        checks.extend(punches(&hp, log));
        for (axis, name) in [(0, "roll"), (1, "pitch"), (2, "yaw")] {
            checks.extend(rate_steps(&hp, log, axis, name));
        }
        checks.extend(coast_downs(&hp, log));
        checks.extend(fall_recoveries(&hp, log));
    }
    // A check with no window in one log but a result in another is not reported missing.
    let ran: std::collections::BTreeSet<String> = checks
        .iter()
        .filter(|c| c.outcome != Outcome::NotInLog)
        .map(|c| c.check.clone())
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    checks.retain(|c| {
        c.outcome != Outcome::NotInLog || (!ran.contains(&c.check) && seen.insert(c.check.clone()))
    });
    for c in checks.iter_mut() {
        if c.outcome == Outcome::NotInLog {
            c.log = "-".into();
        }
    }
    ValidationReport {
        profile: profile.id.clone(),
        logs: logs.iter().map(|l| l.name.clone()).collect(),
        checks,
    }
}

// ----- the harness's own controls -----

/// Betaflight's accelerometer low-pass (`acc_lpf_hz`, 10 Hz), as two PT1 stages.
const ACC_LPF_STAGE_HZ: f64 = 10.0 / 0.643_594_252_905_582_6;

const ARM_CH: usize = 4;
const ANGLE_CH: usize = 5;

/// The profile with the harness's switch layout (AUX1 arm, AUX2 angle) and a linear
/// throttle, so a logged throttle is the collective. Physics and tuning are untouched.
fn harness_profile(p: &SimProfile) -> SimProfile {
    let mut p = p.clone();
    p.diff.aux = vec![
        AuxRange {
            mode_id: mode::ARM,
            ch: ARM_CH,
            start: 1700,
            end: 2100,
        },
        AuxRange {
            mode_id: mode::ANGLE,
            ch: ANGLE_CH,
            start: 1700,
            end: 2100,
        },
    ];
    p.diff.throttle = ThrottleCurve::default();
    p.diff.small_angle = 180.0;
    p
}

fn frame(throttle: f64, armed: bool, angle: bool) -> RcFrame {
    let b = |on: bool| if on { 2000 } else { 1000 };
    RcFrame::from_sticks(
        Sticks {
            throttle,
            ..Sticks::default()
        },
        &[b(armed), b(angle)],
    )
}

fn sim_at(p: &SimProfile, vbat: f64) -> Sim {
    let settings = SimSettings {
        ideal_cell_v: vbat / p.battery.cells.value,
        prop_wash: 0.0,
        ..SimSettings::default()
    };
    Sim::new(p, WorldSpec::empty(), settings, 1)
}

fn arm(sim: &mut Sim, angle: bool) {
    for _ in 0..10 {
        sim.step(&frame(0.0, false, angle));
    }
    for _ in 0..10 {
        sim.step(&frame(0.0, true, angle));
    }
}

/// An altitude-hold pilot on the throttle stick: PI on climb rate toward a height.
pub struct HoverPilot {
    pub target_z: f64,
    i: f64,
}

impl HoverPilot {
    pub fn new(target_z: f64, throttle: f64) -> HoverPilot {
        HoverPilot {
            target_z,
            i: throttle,
        }
    }

    pub fn throttle(&mut self, sim: &Sim) -> f64 {
        let z = sim.position()[2];
        let vz = sim.velocity()[2];
        let want = (1.5 * (self.target_z - z)).clamp(-1.0, 1.0);
        let e = want - vz;
        self.i = (self.i + 0.4 * e * sim.dt).clamp(0.0, 1.0);
        (self.i + 0.15 * e).clamp(0.0, 1.0)
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn moving_avg(x: &[f64], n: usize) -> Vec<f64> {
    let n = n.max(1);
    let mut out = Vec::with_capacity(x.len());
    let mut acc = 0.0;
    for i in 0..x.len() {
        acc += x[i];
        if i >= n {
            acc -= x[i - n];
        }
        out.push(acc / (i + 1).min(n) as f64);
    }
    out
}

// ----- hover -----

/// Steady hover samples: thrust equals weight (body-z specific force 1 g), level, no
/// sideways force (so no forward flight lifting the rotors), not rotating.
/// Only samples within 25 % of the profile's hover speed count, which rules out sitting
/// on the ground with the motors spinning (also 1 g, level and still).
fn hover_samples(p: &SimProfile, log: &LogData) -> Vec<usize> {
    let pr = p.params();
    let hover_rpm = (pr.mass * G / (4.0 * pr.kt)).sqrt() / RPM_TO_RADS;
    (0..log.len())
        .filter(|&i| {
            let q = log.quat[i];
            let tilt = (1.0 - 2.0 * (q[1] * q[1] + q[2] * q[2]))
                .clamp(-1.0, 1.0)
                .acos();
            (log.acc[i][2] - 1.0).abs() < 0.04
                && log.acc[i][0].abs() < 0.2
                && log.acc[i][1].abs() < 0.2
                && tilt < 10f64.to_radians()
                && log.gyro[i].iter().all(|g| g.abs() < 40.0)
                && log.mean_duty(i) > 0.1
                && (log.mean_rpm(i) / hover_rpm - 1.0).abs() < 0.25
        })
        .collect()
}

/// The log's own hover speed (rpm), if it has half a second of steady hover.
fn log_hover_rpm(p: &SimProfile, log: &LogData) -> Option<f64> {
    let calm = hover_samples(p, log);
    ((calm.len() as f64) * log.dt() >= 0.5)
        .then(|| median(calm.iter().map(|&i| log.mean_rpm(i)).collect()))
}

fn hover(p: &SimProfile, log: &LogData) -> Vec<CheckResult> {
    let calm = hover_samples(p, log);
    // At least half a second of steady hover.
    if (calm.len() as f64) * log.dt() < 0.5 {
        return vec![not_in_log("hover", &log.name)];
    }
    let duty = median(calm.iter().map(|&i| log.mean_duty(i)).collect());
    let rpm = median(calm.iter().map(|&i| log.mean_rpm(i)).collect());
    let vbat = median(calm.iter().map(|&i| log.vbat[i]).collect());
    let mut sim = sim_at(p, vbat);
    sim.set_body([0.0, 0.0, 50.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    arm(&mut sim, true);
    let mut pilot = HoverPilot::new(50.0, duty);
    let n = (4.0 / sim.dt) as usize;
    let last = (1.0 / sim.dt) as usize;
    let (mut sd, mut sr, mut k) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let t = pilot.throttle(&sim);
        sim.step(&frame(t, true, true));
        if i + last >= n {
            let s = sim.snapshot();
            sd += s
                .motor_u
                .iter()
                .map(|u| sim.motors.duty_for(*u))
                .sum::<f64>()
                / 4.0;
            sr += s.motor_rpm.iter().sum::<f64>() / 4.0;
            k += 1.0;
        }
    }
    let w = Some((log.t[calm[0]], log.t[*calm.last().unwrap()]));
    vec![
        result(
            "hover",
            "command",
            &log.name,
            w,
            duty,
            sd / k,
            0.02,
            false,
            "",
        ),
        result(
            "hover",
            "speed",
            &log.name,
            w,
            rpm,
            sr / k,
            0.05,
            true,
            "rpm",
        ),
    ]
}

// ----- punch and sag (open loop) -----

fn tilt_deg(q: [f64; 4]) -> f64 {
    (1.0 - 2.0 * (q[1] * q[1] + q[2] * q[2]))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// Punches: the mean command rises by 0.3 within 100 ms and reaches 0.65, with the quad
/// near level and not rotating hard for the 0.4 s that follow (a straight punch-out, not a
/// flip or a turn).
pub fn find_punches(log: &LogData) -> Vec<usize> {
    let dt = log.dt();
    let k = (0.1 / dt).round() as usize;
    let mut out = Vec::new();
    let mut last = f64::MIN;
    let n = log.len();
    for i in 0..n.saturating_sub(3 * k) {
        let rise = log.mean_duty(i + k) - log.mean_duty(i);
        let peak = (i + k..i + 3 * k)
            .map(|j| log.mean_duty(j))
            .fold(0.0, f64::max);
        let w = (0.4 / dt) as usize;
        let straight = i + w < n
            && (i..i + w).all(|j| {
                tilt_deg(log.quat[j]) < 30.0 && log.gyro[j].iter().all(|g| g.abs() < 300.0)
            });
        if rise >= 0.3 && peak >= 0.65 && straight && log.t[i] - last > 1.0 && log.t[i] > 0.06 {
            // Start at the first sample of the rise.
            let mut s = i;
            while s + 1 < i + k && log.mean_duty(s + 1) - log.mean_duty(i) < 0.02 {
                s += 1;
            }
            out.push(s);
            last = log.t[i];
        }
    }
    out
}

pub struct OpenLoop {
    pub accel_g: Vec<f64>,
    pub rpm: Vec<f64>,
    pub amps: Vec<f64>,
    pub volts: Vec<f64>,
}

/// Replay logged commands through the motor model at log sample times, attitude held
/// level and free to climb, as the logged quad does in a punch (so inflow and drag act).
/// `sag`: the pack model supplies the voltage; otherwise the logged voltage does.
/// `accel_g` is what the FC's accelerometer would log: body-z specific force through
/// Betaflight's default accelerometer low-pass (10 Hz, two poles).
pub fn open_loop(p: &SimProfile, log: &LogData, i0: usize, i1: usize, sag: bool) -> OpenLoop {
    open_loop_from(p, log, i0, i1, sag, sink_at(log, i0))
}

/// Vertical speed (m/s) at `i`: the quad's sink since the motors were last above chop,
/// from earth-frame specific force (taken as still before the chop; zero with no chop).
fn sink_at(log: &LogData, i: usize) -> f64 {
    let mut j = i;
    let lim = i.saturating_sub((1.0 / log.dt()) as usize);
    while j > lim && log.duty[j - 1].iter().all(|d| *d < 0.15) {
        j -= 1;
    }
    let mut v = 0.0;
    for k in j + 1..=i {
        let q = log.quat[k];
        let a = DQuat::from_xyzw(q[1], q[2], q[3], q[0]) * DVec3::from(log.acc[k]);
        v += (a.z - 1.0) * G * (log.t[k] - log.t[k - 1]);
    }
    v
}

fn open_loop_from(
    p: &SimProfile,
    log: &LogData,
    i0: usize,
    i1: usize,
    sag: bool,
    vz0: f64,
) -> OpenLoop {
    let pr = p.params();
    let mut acc_lpf = [
        Pt1::new(ACC_LPF_STAGE_HZ, log.dt()),
        Pt1::new(ACC_LPF_STAGE_HZ, log.dt()),
    ];
    for f in &mut acc_lpf {
        f.reset(log.acc[i0][2]);
    }
    let mut vz = vz0;
    let mut force = 0.0;
    let mut m = Motors::new(&pr);
    m.omega = log.rpm[i0].map(|r| r * RPM_TO_RADS);
    let mut b = Battery::new(&pr, true);
    b.set_resting_cell_voltage((log.vbat[i0] + b.resistance * log.amps[i0]) / pr.cells);
    let dt = 1.0 / 4000.0;
    let mut out = OpenLoop {
        accel_g: Vec::new(),
        rpm: Vec::new(),
        amps: Vec::new(),
        volts: Vec::new(),
    };
    let mut t = log.t[i0];
    for i in i0..i1 {
        while t < log.t[i] {
            let duty = log.duty[i.saturating_sub(1).max(i0)];
            let v = if sag {
                let (s, o) = m.pack_current_affine(&duty, true, b.voltage);
                b.loaded_voltage(s, o)
            } else {
                log.vbat[i.saturating_sub(1).max(i0)]
            };
            m.step(dt, &duty, v, true);
            b.update(dt, v, m.pack_current.iter().sum());
            let thrust: f64 = m
                .omega
                .iter()
                .map(|w| pr.kt * w * w * inflow_factor(vz, *w, 2.0 * pr.prop_radius, pr.j0))
                .sum();
            let drag = body_drag([0.0, 0.0, vz], pr.cda)[2];
            force = thrust + drag;
            vz += (force / pr.mass - G) * dt;
            t += dt;
        }
        let a = force / (pr.mass * G);
        let a0 = acc_lpf[0].apply(a);
        let a = acc_lpf[1].apply(a0);
        out.accel_g.push(a);
        out.rpm.push(m.rpm().iter().sum::<f64>() / 4.0);
        out.amps
            .push(m.pack_current.iter().sum::<f64>() + pr.electronics_current);
        out.volts.push(b.voltage);
    }
    out
}

/// Time (s) from the window start to the mean rpm covering 90 % of its rise.
fn rise_90(t: &[f64], rpm: &[f64]) -> f64 {
    let r0 = rpm[0];
    let peak = rpm.iter().cloned().fold(f64::MIN, f64::max);
    let target = r0 + 0.9 * (peak - r0);
    let k = rpm
        .iter()
        .position(|r| *r >= target)
        .unwrap_or(rpm.len() - 1);
    t[k] - t[0]
}

fn punches(p: &SimProfile, log: &LogData) -> Vec<CheckResult> {
    let events = find_punches(log);
    if events.is_empty() {
        return vec![not_in_log("punch", &log.name), not_in_log("sag", &log.name)];
    }
    let dt = log.dt();
    let smooth = (0.04 / dt).round() as usize;
    // Thrust over weight in the log's own units: a flight on a lighter pack hovers at a
    // lower speed, and the accelerometer measures thrust over that flight's mass.
    let weight_scale = match (log_hover_rpm(p, log), p.reference.hover_rpm) {
        (Some(log_rpm), Some(_)) => {
            let w = p.params();
            let sim_rpm = (w.mass * G / (4.0 * w.kt)).sqrt() / RPM_TO_RADS;
            (sim_rpm / log_rpm).powi(2)
        }
        _ => 1.0,
    };
    let mut out = Vec::new();
    for s in events {
        let i1 = s + (0.4 / dt) as usize;
        if i1 > log.len() {
            continue;
        }
        let w = Some((log.t[s], log.t[i1 - 1]));
        let held = open_loop(p, log, s, i1, false);
        let log_acc = moving_avg(
            &log.acc[s..i1].iter().map(|a| a[2]).collect::<Vec<_>>(),
            smooth,
        );
        let sim_acc: Vec<f64> = moving_avg(&held.accel_g, smooth)
            .iter()
            .map(|a| a * weight_scale)
            .collect();
        let peak = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
        out.push(result(
            "punch",
            "peak_accel",
            &log.name,
            w,
            peak(&log_acc),
            peak(&sim_acc),
            0.15,
            true,
            "g",
        ));
        let t = &log.t[s..i1];
        let log_rpm: Vec<f64> = (s..i1).map(|i| log.mean_rpm(i)).collect();
        out.push(result(
            "punch",
            "rise_90",
            &log.name,
            w,
            rise_90(t, &log_rpm) * 1000.0,
            rise_90(t, &held.rpm) * 1000.0,
            15.0,
            false,
            "ms",
        ));
        out.push(result(
            "punch",
            "peak_current",
            &log.name,
            w,
            peak(&moving_avg(&log.amps[s..i1], smooth)),
            peak(&moving_avg(&held.amps, smooth)),
            0.10,
            true,
            "A",
        ));
        let sag = open_loop(p, log, s, i1, true);
        let low = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
        out.push(result(
            "sag",
            "min_voltage",
            &log.name,
            w,
            low(&log.vbat[s..i1]),
            low(&sag.volts),
            0.1,
            false,
            "V",
        ));
    }
    out
}

// ----- closed-loop rate response -----

/// Windows of `len` seconds where the axis's setpoint moves (σ > 30 deg/s, peak > 150).
fn rate_windows(log: &LogData, axis: usize, len: f64) -> Vec<(usize, usize)> {
    let dt = log.dt();
    let n = (len / dt) as usize;
    let mut cands = Vec::new();
    let mut i = 0;
    while i + n <= log.len() {
        let s: Vec<f64> = (i..i + n).map(|k| log.setpoint[k][axis]).collect();
        let mean = s.iter().sum::<f64>() / n as f64;
        let sd = (s.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
        let pk = s.iter().fold(0.0f64, |a, x| a.max(x.abs()));
        let flying = (i..i + n).filter(|&k| log.mean_duty(k) > 0.05).count() * 20 >= n * 19;
        let gpk = (i..i + n).fold(0.0f64, |a, k| a.max(log.gyro[k][axis].abs()));
        // The quad followed its setpoint (no crash, no tumble).
        let tracking = gpk < 1.5 * pk + 100.0;
        if sd > 30.0 && pk > 200.0 && flying && tracking {
            cands.push((sd, i));
        }
        i += n / 2;
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (_, i) in cands {
        if out.iter().all(|(a, b)| i + n <= *a || i >= *b) {
            out.push((i, i + n));
        }
        if out.len() == 2 {
            break;
        }
    }
    out.sort();
    out
}

/// Lag (s) of `y` behind `x`: the shift, 0..80 ms, with the best Pearson correlation
/// between `x` and the shifted `y` over their overlap.
fn lag(x: &[f64], y: &[f64], dt: f64) -> f64 {
    let max = ((0.08 / dt) as usize).min(x.len() / 2);
    let pearson = |a: &[f64], b: &[f64]| {
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
        for (u, v) in a.iter().zip(b) {
            sab += (u - ma) * (v - mb);
            saa += (u - ma).powi(2);
            sbb += (v - mb).powi(2);
        }
        sab / (saa * sbb).sqrt().max(1e-12)
    };
    let mut best = (f64::MIN, 0usize);
    for l in 0..=max {
        let c = pearson(&x[..x.len() - l], &y[l..]);
        if c > best.0 {
            best = (c, l);
        }
    }
    best.1 as f64 * dt
}

/// Overshoot in points at the window's largest setpoint peak: the gyro's peak in the same
/// direction within 80 ms after it, against the setpoint's.
fn overshoot(sp: &[f64], g: &[f64], dt: f64) -> f64 {
    let (k, pk) =
        sp.iter().enumerate().fold(
            (0, 0.0f64),
            |a, (i, x)| if x.abs() > a.1.abs() { (i, *x) } else { a },
        );
    let end = (k + (0.08 / dt) as usize).min(g.len());
    let gpk = g[k..end]
        .iter()
        .map(|x| x * pk.signum())
        .fold(f64::MIN, f64::max);
    (gpk / pk.abs() - 1.0) * 100.0
}

/// BF axes (roll, pitch, yaw) to body rates (rad/s) and back.
fn bf_to_body(r: [f64; 3]) -> [f64; 3] {
    [r[0].to_radians(), r[1].to_radians(), -r[2].to_radians()]
}

fn body_to_bf(r: [f64; 3]) -> [f64; 3] {
    [r[0].to_degrees(), r[1].to_degrees(), -r[2].to_degrees()]
}

struct Replay {
    /// Gyro, BF axes (deg/s).
    gyro: Vec<[f64; 3]>,
    z: Vec<f64>,
    vz: Vec<f64>,
    /// Horizontal speed (m/s).
    speed: Vec<f64>,
}

/// Fly the logged setpoint and throttle closed loop over `i0..i1`, starting level with
/// velocity `v0` (world); the sim's state at each log sample.
/// `attitude`: start from the log's attitude (same body frame as the sim's) rather than level.
fn closed_loop(
    p: &SimProfile,
    log: &LogData,
    i0: usize,
    i1: usize,
    v0: [f64; 3],
    attitude: bool,
) -> Replay {
    let vbat = median(log.vbat[i0..i1].to_vec());
    let mut sim = sim_at(p, vbat);
    sim.set_body([0.0, 0.0, 500.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    arm(&mut sim, false);
    // Airmode engages once the throttle has passed its start point.
    for _ in 0..20 {
        sim.step(&frame(0.3, true, false));
    }
    let q = if attitude {
        let q = log.quat[i0];
        DQuat::from_xyzw(q[1], q[2], q[3], q[0]).normalize()
    } else {
        DQuat::IDENTITY
    };
    sim.set_body([0.0, 0.0, 500.0], q, v0, bf_to_body(log.gyro[i0]));
    for m in sim.motors.omega.iter_mut().zip(log.rpm[i0]) {
        *m.0 = m.1 * RPM_TO_RADS;
    }
    let mut r = Replay {
        gyro: Vec::new(),
        z: Vec::new(),
        vz: Vec::new(),
        speed: Vec::new(),
    };
    let mut t = log.t[i0];
    for i in i0..i1 {
        let k = i.saturating_sub(1).max(i0);
        let ov = Override {
            setpoint: Some(log.setpoint[k]),
        };
        let f = frame(log.throttle[k], true, false);
        while t < log.t[i] {
            sim.step_with(&f, &ov);
            t += sim.dt;
        }
        let v = sim.velocity();
        r.gyro.push(body_to_bf(sim.body_rates()));
        r.z.push(sim.position()[2]);
        r.vz.push(v[2]);
        r.speed.push((v[0] * v[0] + v[1] * v[1]).sqrt());
    }
    r
}

fn rate_steps(p: &SimProfile, log: &LogData, axis: usize, name: &str) -> Vec<CheckResult> {
    let wins = rate_windows(log, axis, 1.0);
    if wins.is_empty() {
        return vec![not_in_log(name, &log.name)];
    }
    let dt = log.dt();
    let skip = (0.15 / dt) as usize;
    let mut out = Vec::new();
    for (i0, i1) in wins {
        let g = closed_loop(p, log, i0, i1, [0.0; 3], false).gyro;
        let sp: Vec<f64> = (i0 + skip..i1).map(|i| log.setpoint[i][axis]).collect();
        let lg: Vec<f64> = (i0 + skip..i1).map(|i| log.gyro[i][axis]).collect();
        let sg: Vec<f64> = g[skip..].iter().map(|x| x[axis]).collect();
        let w = Some((log.t[i0], log.t[i1 - 1]));
        let band = if axis == 2 { 5.0 } else { 3.0 };
        out.push(result(
            name,
            "lag",
            &log.name,
            w,
            lag(&sp, &lg, dt) * 1000.0,
            lag(&sp, &sg, dt) * 1000.0,
            band,
            false,
            "ms",
        ));
        if axis < 2 {
            out.push(result(
                name,
                "overshoot",
                &log.name,
                w,
                overshoot(&sp, &lg, dt),
                overshoot(&sp, &sg, dt),
                10.0,
                false,
                "points",
            ));
        }
    }
    out
}

// ----- fall recovery -----

/// Chop (all motors under 15 % for 0.2 s) then punch (mean over 60 % within 0.2 s).
fn find_falls(log: &LogData) -> Vec<(usize, usize)> {
    let dt = log.dt();
    let mut out = Vec::new();
    let mut i = 0;
    let n = log.len();
    while i < n {
        if log.duty[i].iter().all(|d| *d < 0.15) {
            let mut j = i;
            while j < n && log.duty[j].iter().all(|d| *d < 0.15) {
                j += 1;
            }
            // Airborne: in free fall the accelerometer reads near zero, not 1 g.
            let earth_z = (i..j)
                .map(|k| {
                    let q = log.quat[k];
                    (DQuat::from_xyzw(q[1], q[2], q[3], q[0]) * DVec3::from(log.acc[k])).z
                })
                .sum::<f64>()
                / (j - i).max(1) as f64;
            if (j - i) as f64 * dt >= 0.2 && earth_z < 0.4 {
                let lim = (j + (0.2 / dt) as usize).min(n);
                // A real recovery: a straight climb-out after the punch, not a crash or turtle.
                let after = (0.3 / dt) as usize;
                if let Some(k) = (j..lim).find(|&k| log.mean_duty(k) > 0.6) {
                    let straight = k + after < n
                        && (k..k + after).all(|m| {
                            tilt_deg(log.quat[m]) < 30.0
                                && log.gyro[m].iter().all(|g| g.abs() < 300.0)
                        })
                        && (k..k + after).map(|m| log.mean_duty(m)).sum::<f64>() / after as f64
                            > 0.4;
                    if straight {
                        out.push((i, k));
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Height lost from `i0` (vertical speed taken as zero there) until climbing again,
/// from earth-frame specific force; `None` when the log never climbs again before `end`
/// (no recovery: a crash, or a second chop).
fn logged_height_lost(log: &LogData, i0: usize, end: usize) -> Option<f64> {
    let mut v = 0.0;
    let mut z: f64 = 0.0;
    let mut low: f64 = 0.0;
    for i in i0 + 1..end {
        let q = log.quat[i];
        let a = DQuat::from_xyzw(q[1], q[2], q[3], q[0]) * DVec3::from(log.acc[i]);
        let dt = log.t[i] - log.t[i - 1];
        v += (a.z - 1.0) * G * dt;
        z += v * dt;
        low = low.min(z);
        if i > i0 + 5 && v >= 0.0 && z < 0.0 && low < z {
            return Some(-low);
        }
    }
    None
}

fn fall_recoveries(p: &SimProfile, log: &LogData) -> Vec<CheckResult> {
    let falls = find_falls(log);
    if falls.is_empty() {
        return vec![not_in_log("fall_recovery", &log.name)];
    }
    let dt = log.dt();
    let mut out = Vec::new();
    for (i0, k) in falls {
        let end = (k + (1.0 / dt) as usize).min(log.len());
        let Some(logged) = logged_height_lost(log, i0, end) else {
            continue;
        };
        let Replay { z, vz, .. } = closed_loop(p, log, i0, end, [0.0; 3], false);
        let mut low = z[0];
        for i in 1..z.len() {
            low = low.min(z[i]);
            if vz[i] >= 0.0 && z[i] > low && i > 5 {
                break;
            }
        }
        let w = Some((log.t[i0], log.t[end - 1]));
        out.push(result(
            "fall_recovery",
            "height_lost",
            &log.name,
            w,
            logged,
            z[0] - low,
            0.2,
            true,
            "m",
        ));
    }
    if out.is_empty() {
        out.push(not_in_log("fall_recovery", &log.name));
    }
    out
}

// ----- coast-down -----

/// Coasts: moving faster than 3 m/s, then 0.5 s level (tilt under 10°), not rotating and
/// not punching. Needs a ground-speed column.
fn find_coasts(log: &LogData) -> Vec<(usize, usize)> {
    let Some(speed) = &log.speed else {
        return Vec::new();
    };
    let n = (0.5 / log.dt()) as usize;
    let mut out = Vec::new();
    let mut i = 0;
    while i + n < log.len() {
        let calm = (i..i + n).all(|k| {
            tilt_deg(log.quat[k]) < 10.0
                && log.gyro[k].iter().all(|g| g.abs() < 100.0)
                && log.mean_duty(k) < 0.6
        });
        if speed[i] > 3.0 && calm && speed[i + n] < speed[i] {
            out.push((i, i + n));
            i += n;
        } else {
            i += n / 10;
        }
    }
    out
}

fn coast_downs(p: &SimProfile, log: &LogData) -> Vec<CheckResult> {
    let coasts = find_coasts(log);
    let Some(speed) = &log.speed else {
        return vec![not_in_log("coast_down", &log.name)];
    };
    if coasts.is_empty() {
        return vec![not_in_log("coast_down", &log.name)];
    }
    let mut out = Vec::new();
    for (i0, i1) in coasts {
        let span = log.t[i1] - log.t[i0];
        let logged = (speed[i0] - speed[i1]) / span;
        let r = closed_loop(p, log, i0, i1 + 1, [speed[i0], 0.0, 0.0], true);
        let sim = (r.speed[0] - r.speed[r.speed.len() - 1]) / span;
        let w = Some((log.t[i0], log.t[i1]));
        out.push(result(
            "coast_down",
            "deceleration",
            &log.name,
            w,
            logged,
            sim,
            0.15,
            true,
            "m/s²",
        ));
    }
    out
}

// ----- synthetic logs -----

/// A log written by the sim itself, in the shape a decoded blackbox log has (800 Hz,
/// accelerometer through Betaflight's low-pass, ground speed included). `script` gives
/// the sticks and whether Angle mode is on at each sample; it sees the sim, so it can fly
/// a height-holding throttle. The harness run on such a log against the same profile is
/// its own consistency test.
pub fn synth_log(
    p: &SimProfile,
    seconds: f64,
    mut script: impl FnMut(&Sim, f64) -> (Sticks, bool),
) -> LogData {
    let settings = SimSettings {
        rate_hz: 4000.0,
        prop_wash: 0.0,
        battery_sag: true,
        ..SimSettings::default()
    };
    let hp = harness_profile(p);
    let mut sim = Sim::new(&hp, WorldSpec::empty(), settings, 1);
    sim.battery.set_resting_cell_voltage(4.1);
    sim.set_body([0.0, 0.0, 100.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    arm(&mut sim, true);
    sim.set_body([0.0, 0.0, 100.0], DQuat::IDENTITY, [0.0; 3], [0.0; 3]);
    let per_sample = 5;
    let dt = sim.dt * per_sample as f64;
    let mut lpf = [
        Pt1::new(ACC_LPF_STAGE_HZ, dt),
        Pt1::new(ACC_LPF_STAGE_HZ, dt),
    ];
    for f in &mut lpf {
        f.reset(1.0);
    }
    let mut log = LogData {
        name: "synthetic".into(),
        speed: Some(Vec::new()),
        ..LogData::default()
    };
    let n = (seconds / dt) as usize;
    let mut v_prev = DVec3::from(sim.velocity());
    let b = |on: bool| if on { 2000 } else { 1000 };
    for k in 0..n {
        let t = k as f64 * dt;
        let (s, angle) = script(&sim, t);
        let f = RcFrame::from_sticks(s, &[b(true), b(angle)]);
        for _ in 0..per_sample {
            sim.step(&f);
        }
        let q = sim.attitude();
        let v = DVec3::from(sim.velocity());
        let a_world = (v - v_prev) / dt + DVec3::new(0.0, 0.0, G);
        v_prev = v;
        let a_body = q.inverse() * a_world / G;
        let az0 = lpf[0].apply(a_body.z);
        let az = lpf[1].apply(az0);
        let snap = sim.snapshot();
        let sp = snap.setpoint;
        log.t.push(t);
        log.setpoint.push([sp[0], sp[1], -sp[2]]);
        log.throttle.push(snap.throttle);
        log.gyro.push(body_to_bf(sim.body_rates()));
        log.acc.push([a_body.x, a_body.y, az]);
        log.duty.push(snap.motor_u.map(|u| sim.motors.duty_for(u)));
        log.rpm.push(snap.motor_rpm);
        log.vbat.push(snap.vbat);
        log.amps.push(snap.current);
        log.quat.push([q.w, q.x, q.y, q.z]);
        if let Some(sp) = log.speed.as_mut() {
            sp.push((v.x * v.x + v.y * v.y).sqrt());
        }
    }
    log
}
