//! Profile fitting (sim-design 6.3): a quad's own logs give the numbers its sim profile
//! flies on. The fit starts from a base profile (a preset), keeps every value it cannot
//! measure, and marks each value it measures `fitted` with its residual.
//!
//! In order, each step holding the earlier values:
//!
//! | Step | From | Gives |
//! |---|---|---|
//! | Hover | Level, still, 1 g samples | hover command, speed, current; kT from the mass |
//! | Motor load | Steady samples: pack current against Σ duty and Σ duty·ω² | no-load and electronics current |
//! | Motor at hover | Hover current, speed, duty and voltage, with the spec Kv | kQ (torque balance), winding resistance (duty·V = ω/Kv + R·I) |
//! | Rotor inertia | The motor model replayed on logged commands in steps and punches | rotor inertia (the motor time constant) |
//! | Pack | Vbat against current in 1 s stretches (blackbox), per flight (radio logs) | pack plus wiring resistance |
//! | Rate response | Closed-loop replays of the logged setpoint | a gain scale per axis, so the lag matches |
//!
//! Mass comes from the user (a weighed quad) or the base profile; the logs give only thrust
//! over mass. Drag stays an estimate: blackbox logs carry no ground speed, so there are no
//! coast-downs to fit it to. Rates, throttle curve and modes come from the quad's `diff all`.

use serde::Serialize;
use specta::Type;

use crate::diff::FcConfig;
use crate::log::LogData;
use crate::motor::Motors;
use crate::profile::{fm_from_kq, MotorModel, SimProfile, Source, Sv, G, RPM_TO_RADS};
use crate::validate::{
    find_punches, group_score, lag, median, moving_avg, overshoot, rate_windows, rise_90, tilt_deg,
    Group,
};

/// One radio-log flight's pack telemetry (EdgeTX `RxBt` and `Curr`, about 10 Hz).
#[derive(Debug, Clone, Default)]
pub struct RadioFlight {
    pub name: String,
    /// Seconds from the flight's start.
    pub t: Vec<f64>,
    pub volts: Vec<f64>,
    pub amps: Vec<f64>,
}

/// What the fit is given besides the blackbox logs.
#[derive(Debug, Clone, Default)]
pub struct FitInputs {
    /// The new profile's id and label.
    pub id: String,
    pub label: String,
    /// All-up mass with a pack (kg), weighed; None keeps the base profile's.
    pub mass: Option<f64>,
    pub pack_mass: Option<f64>,
    /// The quad's `diff all`: rates, throttle curve, modes, idle, pole count.
    pub diff: Option<String>,
    pub radio: Vec<RadioFlight>,
}

/// One value the fit set, or measured for reference.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct FitValue {
    /// The profile path (`prop.kt`, `motor.r_winding`, `fc.gains.roll`), or a measure.
    pub name: String,
    pub unit: String,
    /// The base profile's value.
    pub before: Option<f64>,
    pub value: f64,
    pub source: Source,
    /// The fit's residual in `residual_unit`.
    pub residual: Option<f64>,
    pub residual_unit: String,
    /// Samples or windows the value came from.
    pub samples: usize,
    pub note: Option<String>,
}

/// Thrust at one motor command, steady state, at a pack voltage.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct ThrustPoint {
    /// Mixer output 0..1 (the ESC duty runs from idle to full over it).
    pub command: f64,
    pub rpm: f64,
    /// Total static thrust (g-force grams) and thrust over weight.
    pub thrust_g: f64,
    pub tw: f64,
}

/// The fit's result: the profile, every value with its source, the measures, the thrust
/// curve and what is still an estimate.
#[derive(Debug, Clone, Serialize, Type)]
pub struct FitReport {
    pub profile_id: String,
    pub base: String,
    pub logs: Vec<String>,
    /// Values the fit set in the profile.
    pub values: Vec<FitValue>,
    /// Measured from the logs, for reference (not profile values).
    pub measured: Vec<FitValue>,
    /// Static thrust against command at 3.9 V per cell (the sim's pack with sag off).
    pub thrust_curve: Vec<ThrustPoint>,
    /// Profile values that stay estimates or the base profile's, and why.
    pub estimates: Vec<String>,
    pub notes: Vec<String>,
    #[serde(skip)]
    #[specta(skip)]
    pub profile: Option<SimProfile>,
}

impl FitReport {
    /// A plain-text summary.
    pub fn table(&self) -> String {
        let mut s = format!(
            "Fit of {} from {} on {} log(s)\n",
            self.profile_id,
            self.base,
            self.logs.len()
        );
        let row = |s: &mut String, v: &FitValue| {
            let before = v.before.map(fmt).unwrap_or_else(|| "-".into());
            let res = v
                .residual
                .map(|r| format!("±{} {}", fmt(r), v.residual_unit))
                .unwrap_or_default();
            s.push_str(&format!(
                "  {:<28} {:>12} -> {:>12} {:<8} {:>6}  {}\n",
                v.name,
                before,
                fmt(v.value),
                v.unit,
                v.samples,
                res
            ));
        };
        s.push_str("Profile values (before -> fitted, unit, samples, residual):\n");
        for v in &self.values {
            row(&mut s, v);
        }
        s.push_str("Measured:\n");
        for v in &self.measured {
            row(&mut s, v);
        }
        s.push_str("Thrust at 3.9 V per cell (command, rpm, grams, T/W):\n");
        for p in &self.thrust_curve {
            s.push_str(&format!(
                "  {:>4.2} {:>8.0} {:>7.1} {:>5.2}\n",
                p.command, p.rpm, p.thrust_g, p.tw
            ));
        }
        for e in &self.estimates {
            s.push_str(&format!("Estimate: {e}\n"));
        }
        for n in &self.notes {
            s.push_str(&format!("Note: {n}\n"));
        }
        s
    }
}

fn fmt(x: f64) -> String {
    if x != 0.0 && (x.abs() < 1e-3 || x.abs() >= 1e5) {
        format!("{x:.4e}")
    } else {
        format!("{x:.4}")
    }
}

/// The values the power group of the guard covers.
const POWER_VALUES: &[&str] = &[
    "battery.electronics_current",
    "battery.resistance",
    "motor.r_winding",
    "motor.no_load_current",
    "prop.kq",
    "prop.fm",
    "motor.rotor_inertia",
];

fn fitted(value: f64, logs: &[String], residual: Option<f64>, note: &str) -> Sv {
    Sv {
        value,
        source: Source::Fitted,
        logs: logs.to_vec(),
        residual,
        note: Some(note.to_string()),
    }
}

/// The motor values the replay fits.
#[derive(Debug, Clone, Copy)]
struct MotorFit {
    r: f64,
    i0: f64,
    kq: f64,
    j: f64,
    ke: f64,
}

impl MotorFit {
    fn with(&self, k: usize, x: f64) -> MotorFit {
        let mut m = *self;
        match k {
            0 => m.r = x,
            1 => m.i0 = x,
            2 => m.kq = x,
            3 => m.j = x,
            _ => m.ke = x,
        }
        m
    }
}

/// Least squares: the x minimising |A·x − b|, by the normal equations (small systems).
fn lstsq(rows: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = rows.first()?.len();
    let mut m = vec![vec![0.0; n + 1]; n];
    for (r, y) in rows.iter().zip(b) {
        for i in 0..n {
            for j in 0..n {
                m[i][j] += r[i] * r[j];
            }
            m[i][n] += r[i] * y;
        }
    }
    // Gaussian elimination with partial pivoting.
    for c in 0..n {
        let p = (c..n).max_by(|a, b| m[*a][c].abs().total_cmp(&m[*b][c].abs()))?;
        m.swap(c, p);
        if m[c][c].abs() < 1e-300 {
            return None;
        }
        for r in 0..n {
            if r != c {
                let f = m[r][c] / m[c][c];
                for k in c..=n {
                    m[r][k] -= f * m[c][k];
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}

fn rms(v: impl Iterator<Item = f64>) -> f64 {
    let (mut s, mut n) = (0.0, 0.0);
    for x in v {
        s += x * x;
        n += 1.0;
    }
    if n == 0.0 {
        0.0
    } else {
        (s / n).sqrt()
    }
}

/// Minimise `f` over [a, b] by golden-section search.
fn golden(mut a: f64, mut b: f64, iters: usize, mut f: impl FnMut(f64) -> f64) -> f64 {
    let r = (5f64.sqrt() - 1.0) / 2.0;
    let mut c = b - r * (b - a);
    let mut d = a + r * (b - a);
    let (mut fc, mut fd) = (f(c), f(d));
    for _ in 0..iters {
        if fc < fd {
            b = d;
            d = c;
            fd = fc;
            c = b - r * (b - a);
            fc = f(c);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + r * (b - a);
            fd = f(d);
        }
    }
    (a + b) / 2.0
}

/// d(rpm)/dt per motor (rpm/s), a central difference over ±`k` samples.
fn rpm_rate(log: &LogData, i: usize, k: usize) -> [f64; 4] {
    let a = i.saturating_sub(k);
    let b = (i + k).min(log.len() - 1);
    let dt = (log.t[b] - log.t[a]).max(1e-6);
    let mut out = [0.0; 4];
    for m in 0..4 {
        out[m] = (log.rpm[b][m] - log.rpm[a][m]) / dt;
    }
    out
}

/// Steady hover samples within `band` of `rpm` (the airborne check: sitting on the ground
/// with the motors spinning also reads 1 g, level and still).
fn hover_samples(log: &LogData, rpm: f64, band: f64) -> Vec<usize> {
    (0..log.len())
        .filter(|&i| {
            (log.acc[i][2] - 1.0).abs() < 0.04
                && log.acc[i][0].abs() < 0.2
                && log.acc[i][1].abs() < 0.2
                && tilt_deg(log.quat[i]) < 10.0
                && log.gyro[i].iter().all(|g| g.abs() < 40.0)
                && log.mean_duty(i) > 0.1
                && (log.mean_rpm(i) / rpm - 1.0).abs() < band
        })
        .collect()
}

/// The steady state of one electrical motor: speed (rad/s) and winding current (A) at
/// duty `d` and voltage `v`.
pub fn steady_motor(kv_rads: f64, r: f64, i0: f64, kq: f64, d: f64, v: f64) -> (f64, f64) {
    let ke = 1.0 / kv_rads;
    // ke·((d·V − ke·ω)/R − i0) = kQ·ω²
    let a = kq;
    let b = ke * ke / r;
    let c = -ke * (d * v / r - i0);
    if c >= 0.0 {
        return (0.0, 0.0);
    }
    let w = (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a);
    (w, (d * v - ke * w) / r)
}

/// Fit a profile from `base` and decoded blackbox `logs`.
pub fn fit(base: &SimProfile, inputs: &FitInputs, logs: &[LogData]) -> Result<FitReport, String> {
    if logs.is_empty() {
        return Err("no blackbox logs to fit from".into());
    }
    let mut p = base.clone();
    p.id = inputs.id.clone();
    p.label = inputs.label.clone();
    let names: Vec<String> = logs.iter().map(|l| l.name.clone()).collect();
    let mut values = Vec::new();
    let mut measured = Vec::new();
    let mut estimates = Vec::new();
    let mut notes = Vec::new();
    let val = |name: &str, unit: &str, before: Option<f64>, sv: &Sv, ru: &str, n: usize| FitValue {
        name: name.into(),
        unit: unit.into(),
        before,
        value: sv.value,
        source: sv.source,
        residual: sv.residual,
        residual_unit: ru.into(),
        samples: n,
        note: sv.note.clone(),
    };
    let meas = |name: &str, unit: &str, value: f64, n: usize, note: &str| FitValue {
        name: name.into(),
        unit: unit.into(),
        before: None,
        value,
        source: Source::Fitted,
        residual: None,
        residual_unit: String::new(),
        samples: n,
        note: (!note.is_empty()).then(|| note.to_string()),
    };

    // ----- the diff and the mass -----
    if let Some(d) = &inputs.diff {
        p.diff = FcConfig::from_diff(d);
        p.motor.poles = Sv::new(p.diff.motor_poles as f64, Source::Diff);
    } else {
        estimates.push(
            "rates, throttle curve and modes: no diff all given, Betaflight defaults kept".into(),
        );
    }
    if let Some(m) = inputs.mass {
        let before = p.frame.mass.value;
        p.frame.mass = Sv {
            note: Some("weighed, with a pack".into()),
            ..Sv::new(m, Source::Spec)
        };
        values.push(val("frame.mass", "kg", Some(before), &p.frame.mass, "", 0));
    } else {
        estimates.push(format!(
            "frame.mass: {} kg from {} (weigh the quad with a pack and pass it)",
            p.frame.mass.value, base.id
        ));
    }
    if let Some(m) = inputs.pack_mass {
        p.frame.pack_mass = Sv::new(m, Source::Spec);
    }
    let mass = p.frame.mass.value;
    let pr0 = p.params();
    let kv_rads0 = pr0.kv_rads;

    // ----- hover -----
    let prior = (mass * G / (4.0 * pr0.kt)).sqrt() / RPM_TO_RADS;
    let mut first = Vec::new();
    for l in logs {
        first.extend(hover_samples(l, prior, 0.35).iter().map(|&i| l.mean_rpm(i)));
    }
    if first.is_empty() {
        return Err("the logs hold no steady hover".into());
    }
    let centre = median(first);
    let mut sum_w2 = Vec::new();
    let (mut h_duty, mut h_rpm, mut h_v, mut h_a) = (vec![], vec![], vec![], vec![]);
    let mut hover_secs = 0.0;
    for l in logs {
        let s = hover_samples(l, centre, 0.2);
        hover_secs += s.len() as f64 * l.dt();
        for i in s {
            sum_w2.push(
                l.rpm[i]
                    .iter()
                    .map(|r| (r * RPM_TO_RADS).powi(2))
                    .sum::<f64>(),
            );
            h_duty.push(l.mean_duty(i));
            h_rpm.push(l.mean_rpm(i));
            h_v.push(l.vbat[i]);
            h_a.push(l.amps[i]);
        }
    }
    if hover_secs < 0.5 {
        return Err(format!(
            "the logs hold {hover_secs:.1} s of steady hover; the fit needs 0.5 s"
        ));
    }
    let n_hover = h_duty.len();
    let w2 = median(sum_w2.clone());
    let kt = mass * G / w2;
    let kt_spread = rms(sum_w2.iter().map(|s| mass * G / s - kt));
    let kt_before = p.prop.kt.value;
    p.prop.kt = fitted(
        kt,
        &names,
        Some(kt_spread),
        "weight over Σω² in steady hover",
    );
    values.push(val(
        "prop.kt",
        "N/(rad/s)²",
        Some(kt_before),
        &p.prop.kt,
        "N/(rad/s)²",
        n_hover,
    ));
    let hover_cmd = median(h_duty.clone());
    let hover_rpm = median(h_rpm.clone());
    let hover_v = median(h_v.clone());
    let hover_a = median(h_a.clone());
    measured.push(meas("hover_command", "duty", hover_cmd, n_hover, ""));
    measured.push(meas("hover_speed", "rpm", hover_rpm, n_hover, ""));
    measured.push(meas("hover_voltage", "V", hover_v, n_hover, ""));
    measured.push(meas("hover_current", "A", hover_a, n_hover, ""));
    measured.push(meas("hover_seconds", "s", hover_secs, n_hover, ""));

    // ----- motor load and electrical model, on steady samples -----
    let mut load_rows = Vec::new();
    let mut load_b = Vec::new();
    let mut steady_ix: Vec<(usize, usize)> = Vec::new();
    for (li, l) in logs.iter().enumerate() {
        let step = (0.01 / l.dt()).max(1.0) as usize;
        let mut i = step;
        while i + step < l.len() {
            let rr = rpm_rate(l, i, step);
            let steady = rr.iter().all(|x| x.abs() < 15_000.0)
                && l.duty[i].iter().all(|d| *d > 0.08)
                && l.amps[i] > 0.0
                && l.vbat[i] > 2.5;
            if steady {
                let w = l.rpm[i].map(|r| r * RPM_TO_RADS);
                let sd: f64 = l.duty[i].iter().sum();
                let sdw2: f64 = (0..4).map(|k| l.duty[i][k] * w[k] * w[k]).sum();
                load_rows.push(vec![sd, sdw2, 1.0]);
                load_b.push(l.amps[i]);
                steady_ix.push((li, i));
            }
            i += step;
        }
    }
    let n_steady = steady_ix.len();
    if n_steady < 50 {
        return Err(format!(
            "the logs hold {n_steady} steady samples; the fit needs 50"
        ));
    }
    // pack A = Σd·i0 + (kQ/ke)·Σd·ω² + Ie
    let x = lstsq(&load_rows, &load_b).ok_or("the motor-load fit is singular")?;
    let (mut i0, mut kq_over_ke, mut ie) = (x[0], x[1], x[2]);
    if kq_over_ke.is_nan() || kq_over_ke <= 0.0 {
        return Err("the motor-load fit gave no prop torque (current sensor off?)".into());
    }
    if i0 < 0.0 || ie < 0.0 {
        // Refit with the electronics current held at the base profile's.
        ie = p.battery.electronics_current.value;
        let rows: Vec<Vec<f64>> = load_rows.iter().map(|r| vec![r[0], r[1]]).collect();
        let b: Vec<f64> = load_b.iter().map(|a| a - ie).collect();
        let y = lstsq(&rows, &b).ok_or("the motor-load fit is singular")?;
        i0 = y[0].max(0.0);
        kq_over_ke = y[1];
        notes.push("no-load or electronics current fitted negative; electronics current held at the base profile's".into());
    }
    let load_res = rms(load_rows
        .iter()
        .zip(&load_b)
        .map(|(r, a)| r[0] * i0 + r[1] * kq_over_ke + ie - a));
    // duty·V = ke·ω + R·I, I = i0 + (kQ/ke)·ω², per motor.
    let mut el_rows = Vec::new();
    let mut el_b = Vec::new();
    for &(li, i) in &steady_ix {
        let l = &logs[li];
        for k in 0..4 {
            let w = l.rpm[i][k] * RPM_TO_RADS;
            el_rows.push(vec![w, i0 + kq_over_ke * w * w]);
            el_b.push(l.duty[i][k] * l.vbat[i]);
        }
    }
    let y = lstsq(&el_rows, &el_b).ok_or("the electrical fit is singular")?;
    measured.push(meas(
        "kv_under_load",
        "rpm/V",
        1.0 / y[0] / RPM_TO_RADS,
        n_steady,
        "steady speed against duty × voltage, Kv and R both free (reference; the profile keeps the spec Kv)",
    ));
    let ke = 1.0 / kv_rads0;
    let (r_before, i0_before, j_before) = match &p.motor.model {
        MotorModel::Electrical {
            r_winding,
            no_load_current,
            rotor_inertia,
        } => (
            Some(r_winding.value),
            Some(no_load_current.value),
            rotor_inertia.value,
        ),
        MotorModel::FirstOrder { .. } => (None, None, 1e-7),
    };
    // The motor holds hover where the log has it, the steadiest and best-sampled operating
    // point: for a winding resistance R, the voltage balance (duty · V = ω/Kv + R · I)
    // gives the winding current at hover, and torque balance gives kQ.
    if !(0.0..0.5).contains(&i0) || i0 == 0.0 {
        i0 = i0_before.unwrap_or(0.05);
        notes.push("the no-load current did not fit; the base profile's is kept".into());
    }
    let wh = hover_rpm * RPM_TO_RADS;
    let i_h = (hover_a - ie) / (4.0 * hover_cmd);
    if i_h.is_nan() || i_h <= i0 {
        return Err(format!(
            "hover current {hover_a:.2} A leaves no motor current over {ie:.2} A of electronics"
        ));
    }
    // The voltage the winding drops at hover.
    let headroom = hover_cmd * hover_v - ke * wh;
    let kq_at = |r: f64| ke * (headroom / r - i0) / (wh * wh);
    // R where the hover current matches the log's: the search starts there.
    let r_anchor = headroom / i_h;
    if r_anchor <= 0.0 {
        return Err(format!(
            "hover speed {hover_rpm:.0} rpm at duty {hover_cmd:.3} and {hover_v:.2} V is above Kv × duty × V: check the Kv and the pole count"
        ));
    }
    measured.push(meas(
        "hover_motor_current",
        "A",
        i_h,
        n_hover,
        "winding current per motor in hover",
    ));
    let ie_before = p.battery.electronics_current.value;
    p.battery.electronics_current = fitted(
        ie,
        &names,
        Some(load_res),
        "pack current at zero motor load",
    );
    values.push(val(
        "battery.electronics_current",
        "A",
        Some(ie_before),
        &p.battery.electronics_current,
        "A",
        n_steady,
    ));

    // ----- rotor inertia: replay the motor model on steps -----
    let mut steps: Vec<(usize, usize)> = Vec::new();
    for (li, l) in logs.iter().enumerate() {
        let dt = l.dt();
        let k = (0.03 / dt).max(1.0) as usize;
        let mut last = f64::MIN;
        for i in 0..l.len().saturating_sub(k + 1) {
            let rise = l.mean_duty(i + k) - l.mean_duty(i);
            if rise.abs() > 0.12 && l.t[i] - last > 0.5 && l.duty[i].iter().all(|d| *d > 0.05) {
                steps.push((li, i));
                last = l.t[i];
            }
        }
    }
    // Punches too: they reach full command, where the model must hold up best.
    for (li, l) in logs.iter().enumerate() {
        for s in find_punches(l) {
            steps.push((li, s.saturating_sub(5)));
        }
    }
    // Spread the windows over all the logs, at most 60.
    if steps.len() > 60 {
        let every = steps.len() as f64 / 60.0;
        steps = (0..60)
            .map(|k| steps[(k as f64 * every) as usize])
            .collect();
    }
    let mut punch_rise = Vec::new();
    for l in logs {
        for s in find_punches(l) {
            let i1 = s + (0.4 / l.dt()) as usize;
            if i1 <= l.len() {
                let rpm: Vec<f64> = (s..i1).map(|i| l.mean_rpm(i)).collect();
                punch_rise.push(rise_90(&l.t[s..i1], &rpm) * 1000.0);
            }
        }
    }
    // Replay the motor model on the logged commands and voltage in each window and score
    // speed and pack current, both relative to hover.
    let replay_cost = |m: &MotorFit| -> (f64, f64) {
        let mut prm = p.params();
        prm.motor = crate::profile::MotorKind::Electrical {
            r: m.r,
            i0: m.i0,
            j: m.j,
        };
        prm.kv_rads = 1.0 / m.ke;
        prm.kq = m.kq;
        let (mut er, mut ea, mut n, mut na) = (0.0, 0.0, 0.0, 0.0);
        for &(li, s) in &steps {
            let l = &logs[li];
            let e = (s + (0.3 / l.dt()) as usize).min(l.len());
            let mut mo = Motors::new(&prm);
            mo.omega = l.rpm[s].map(|x| x * RPM_TO_RADS);
            let sub = 8;
            let mut amps = Vec::with_capacity(e - s);
            for i in s + 1..e {
                let h = (l.t[i] - l.t[i - 1]) / sub as f64;
                for _ in 0..sub {
                    mo.step(h, &l.duty[i - 1], l.vbat[i - 1], true);
                }
                for k in 0..4 {
                    er += (mo.omega[k] / RPM_TO_RADS - l.rpm[i][k]).powi(2);
                    n += 1.0;
                }
                amps.push(mo.pack_current.iter().sum::<f64>() + ie);
            }
            // The logged current is filtered: compare 40 ms averages, as the punch check does.
            let k = (0.04 / l.dt()).round() as usize;
            let sim_a = moving_avg(&amps, k);
            let log_a = moving_avg(&l.amps[s + 1..e], k);
            for (a, b) in sim_a.iter().zip(&log_a) {
                ea += (a - b).powi(2);
                na += 1.0;
            }
        }
        if n == 0.0 || na == 0.0 {
            (0.0, 0.0)
        } else {
            ((er / n).sqrt(), (ea / na).sqrt())
        }
    };
    // Speed and pack current both count, each against its value at hover.
    let score = |m: &MotorFit| {
        let (er, ea) = replay_cost(m);
        er / hover_rpm + ea / hover_a.max(0.5)
    };
    let mut mf = MotorFit {
        r: r_anchor,
        i0,
        kq: kq_at(r_anchor),
        j: j_before,
        ke,
    };
    if steps.is_empty() {
        estimates.push(
            "motor.rotor_inertia: the logs hold no motor steps; R and kq hold hover's current"
                .into(),
        );
    } else {
        // R (with kq following it, so hover stays put), then J; twice, as they trade off.
        let r_max = 0.95 * headroom / i0;
        let (lo, hi) = ((r_anchor / 4.0).ln(), (r_anchor * 4.0).min(r_max).ln());
        for _ in 0..2 {
            let lr = golden(lo, hi, 20, |x| {
                let r = x.exp();
                score(&MotorFit {
                    r,
                    kq: kq_at(r),
                    ..mf
                })
            });
            mf.r = lr.exp();
            mf.kq = kq_at(mf.r);
            let lj = golden((j_before / 20.0).ln(), (j_before * 20.0).ln(), 20, |x| {
                score(&mf.with(3, x.exp()))
            });
            mf = mf.with(3, lj.exp());
        }
    }
    let (rpm_res, amp_res) = replay_cost(&mf);
    let base_motor = r_before.map(|r| MotorFit {
        r,
        i0: i0_before.unwrap_or(i0),
        kq: pr0.kq,
        j: j_before,
        ke: 1.0 / kv_rads0,
    });
    if let Some(b) = &base_motor {
        let (rpm_base, amp_base) = replay_cost(b);
        measured.push(meas(
            "motor_replay_speed_rms_base",
            "rpm",
            rpm_base,
            steps.len(),
            "the base profile's motor in the same replay",
        ));
        measured.push(meas(
            "motor_replay_current_rms_base",
            "A",
            amp_base,
            steps.len(),
            "the base profile's motor in the same replay",
        ));
    }
    measured.push(meas(
        "motor_replay_speed_rms",
        "rpm",
        rpm_res,
        steps.len(),
        "the motor model replayed on logged steps and punches",
    ));
    measured.push(meas(
        "motor_replay_current_rms",
        "A",
        amp_res,
        steps.len(),
        "pack current in the same replay",
    ));
    let r_sv = fitted(
        mf.r,
        &names,
        Some(amp_res),
        "the motor model replayed on logged steps and punches, hover held",
    );
    let i0_sv = fitted(
        mf.i0,
        &names,
        Some(load_res),
        "pack current against duty and speed in steady flight",
    );
    let j_sv = fitted(
        mf.j,
        &names,
        Some(rpm_res),
        "the motor model replayed on logged steps and punches",
    );
    values.push(val(
        "motor.r_winding",
        "Ω",
        r_before,
        &r_sv,
        "A",
        steps.len(),
    ));
    values.push(val(
        "motor.no_load_current",
        "A",
        i0_before,
        &i0_sv,
        "A",
        n_steady,
    ));
    let kq_before = p.prop.kq.as_ref().map(|k| k.value);
    p.prop.kq = Some(fitted(
        mf.kq,
        &names,
        Some(amp_res),
        "torque balance at hover, with the fitted R",
    ));
    values.push(val(
        "prop.kq",
        "N·m/(rad/s)²",
        kq_before,
        p.prop.kq.as_ref().unwrap(),
        "A",
        steps.len(),
    ));
    let fm = fm_from_kq(kt, mf.kq, p.prop.diameter.value / 2.0);
    let fm_before = p.prop.fm.value;
    p.prop.fm = Sv {
        note: Some("follows from kt and kq".into()),
        ..Sv::new(fm, Source::Fitted)
    };
    values.push(val(
        "prop.fm",
        "",
        Some(fm_before),
        &p.prop.fm,
        "",
        n_steady,
    ));
    values.push(val(
        "motor.rotor_inertia",
        "kg·m²",
        Some(j_before),
        &j_sv,
        "rpm",
        steps.len(),
    ));
    p.motor.model = MotorModel::Electrical {
        r_winding: r_sv,
        no_load_current: i0_sv,
        rotor_inertia: j_sv,
    };
    let (r, i0, kq, j, ke) = (mf.r, mf.i0, mf.kq, mf.j, mf.ke);
    // The small-signal time constant at hover: J over (ke²/R + 2·kQ·ω).
    let wh = hover_rpm * RPM_TO_RADS;
    let tau = j / (ke * ke / r + 2.0 * kq * wh);
    measured.push(meas(
        "motor_time_constant",
        "ms",
        tau * 1000.0,
        steps.len(),
        "small steps at hover",
    ));
    if !punch_rise.is_empty() {
        let n = punch_rise.len();
        measured.push(meas(
            "punch_rise_90",
            "ms",
            median(punch_rise),
            n,
            "speed reaching 90 % in a punch",
        ));
    }

    // ----- pack resistance -----
    let mut seg_rows = Vec::new();
    let (mut cov, mut var) = (0.0, 0.0);
    for l in logs {
        let n = (1.0 / l.dt()) as usize;
        let mut i = 0;
        while i + n <= l.len() {
            let a = &l.amps[i..i + n];
            let v = &l.vbat[i..i + n];
            let ma = a.iter().sum::<f64>() / n as f64;
            let mv = v.iter().sum::<f64>() / n as f64;
            let va = a.iter().map(|x| (x - ma).powi(2)).sum::<f64>() / n as f64;
            if va.sqrt() > 0.15 * hover_a.max(0.5) && ma > 0.0 {
                let c = a
                    .iter()
                    .zip(v)
                    .map(|(x, y)| (x - ma) * (y - mv))
                    .sum::<f64>()
                    / n as f64;
                cov += c;
                var += va;
                seg_rows.push((ma, mv, va, c));
            }
            i += n;
        }
    }
    let r_bb = (var > 0.0).then(|| -cov / var);
    let r_radio = radio_resistance(&inputs.radio);
    if let Some((rr, n)) = r_radio {
        measured.push(meas(
            "pack_resistance_radio",
            "Ω",
            rr,
            n,
            "radio-log RxBt against Curr, per flight with drift",
        ));
    }
    let pack_before = p.battery.resistance.value;
    match (r_bb, r_radio) {
        (Some(rb), _) if rb > 0.0 => {
            p.battery.resistance = fitted(
                rb,
                &names,
                None,
                "blackbox vbat against current in 1 s stretches",
            );
            values.push(val(
                "battery.resistance",
                "Ω",
                Some(pack_before),
                &p.battery.resistance,
                "",
                seg_rows.len(),
            ));
        }
        (_, Some((rr, n))) if rr > 0.0 => {
            p.battery.resistance = Sv {
                note: Some("radio-log RxBt against Curr".into()),
                ..Sv::new(rr, Source::Fitted)
            };
            values.push(val(
                "battery.resistance",
                "Ω",
                Some(pack_before),
                &p.battery.resistance,
                "",
                n,
            ));
        }
        _ => estimates.push("battery.resistance: no current swings in the logs".into()),
    }

    // ----- thrust curve and T/W -----
    let idle = p.diff.dshot_idle_value / 10_000.0;
    let cells = p.battery.cells.value;
    let weight = mass * G;
    let thrust_at = |u: f64, v_cell: f64| {
        let d = idle + (1.0 - idle) * u;
        let (w, _) = steady_motor(1.0 / ke, r, i0, kq, d, v_cell * cells);
        (w, 4.0 * kt * w * w)
    };
    let thrust_curve: Vec<ThrustPoint> = (0..=10)
        .map(|k| {
            let u = k as f64 / 10.0;
            let (w, t) = thrust_at(u, 3.9);
            ThrustPoint {
                command: u,
                rpm: w / RPM_TO_RADS,
                thrust_g: t / G * 1000.0,
                tw: t / weight,
            }
        })
        .collect();
    measured.push(meas(
        "tw_static_4v2",
        "",
        thrust_at(1.0, 4.2).1 / weight,
        0,
        "full command, 4.2 V per cell at the ESC",
    ));
    measured.push(meas(
        "tw_static_3v6",
        "",
        thrust_at(1.0, 3.6).1 / weight,
        0,
        "full command, 3.6 V per cell at the ESC",
    ));
    let pk: Vec<f64> = logs
        .iter()
        .flat_map(|l| {
            find_punches(l).into_iter().filter_map(move |s| {
                let i1 = s + (0.4 / l.dt()) as usize;
                (i1 <= l.len()).then(|| (s..i1).map(|i| l.acc[i][2]).fold(f64::MIN, f64::max))
            })
        })
        .collect();
    if !pk.is_empty() {
        let n = pk.len();
        measured.push(meas(
            "punch_peak_accel",
            "g",
            median(pk),
            n,
            "logged body-z specific force in punches",
        ));
    }

    // ----- rate response: a gain scale per axis -----
    let hp = crate::validate::harness_profile(&p);
    for (axis, name) in [(0, "roll"), (1, "pitch"), (2, "yaw")] {
        let mut wins = Vec::new();
        for (li, l) in logs.iter().enumerate() {
            for w in rate_windows(l, axis, 1.0) {
                wins.push((li, w));
            }
        }
        wins.truncate(4);
        if wins.is_empty() {
            estimates.push(format!("fc.gains.{name}: the logs hold no {name} moves"));
            continue;
        }
        let logged: Vec<(f64, f64)> = wins
            .iter()
            .map(|&(li, (i0w, i1w))| {
                let l = &logs[li];
                let dt = l.dt();
                let skip = (0.15 / dt) as usize;
                let sp: Vec<f64> = (i0w + skip..i1w).map(|i| l.setpoint[i][axis]).collect();
                let g: Vec<f64> = (i0w + skip..i1w).map(|i| l.gyro[i][axis]).collect();
                (lag(&sp, &g, dt) * 1000.0, overshoot(&sp, &g, dt))
            })
            .collect();
        let lag_log = median(logged.iter().map(|x| x.0).collect());
        measured.push(meas(
            &format!("{name}_lag"),
            "ms",
            lag_log,
            wins.len(),
            "setpoint to gyro",
        ));
        let cost = |scale: f64| -> (f64, f64) {
            let mut q = hp.clone();
            let g = &mut q.fc.gains[axis];
            g.p.value *= scale;
            g.d.value *= scale;
            g.ff.value *= scale;
            let mut lags = Vec::new();
            let mut ov = 0.0;
            for (k, &(li, (i0w, i1w))) in wins.iter().enumerate() {
                let l = &logs[li];
                let dt = l.dt();
                let skip = (0.15 / dt) as usize;
                let gy = crate::validate::closed_loop(&q, l, i0w, i1w, [0.0; 3], false).gyro;
                let sp: Vec<f64> = (i0w + skip..i1w).map(|i| l.setpoint[i][axis]).collect();
                let sg: Vec<f64> = gy[skip..].iter().map(|x| x[axis]).collect();
                lags.push(lag(&sp, &sg, dt) * 1000.0);
                if axis < 2 {
                    ov += (overshoot(&sp, &sg, dt) - logged[k].1).abs();
                }
            }
            (median(lags), ov / wins.len() as f64)
        };
        let scale = 10f64.powf(golden((0.25f64).log10(), 4f64.log10(), 14, |x| {
            let (l, o) = cost(10f64.powf(x));
            (l - lag_log).abs() + 0.05 * o
        }));
        let (lag_sim, _) = cost(scale);
        let g = &mut p.fc.gains[axis];
        let before = g.p.value;
        for sv in [&mut g.p, &mut g.d, &mut g.ff] {
            sv.value *= scale;
            sv.source = Source::Fitted;
            sv.logs = names.clone();
            sv.residual = Some(lag_sim - lag_log);
            sv.note = Some(format!(
                "{name} gains × {scale:.2} so the replayed lag matches the log"
            ));
        }
        let shown = Sv {
            residual: Some(lag_sim - lag_log),
            ..g.p.clone()
        };
        values.push(val(
            &format!("fc.gains.{name}.p"),
            "1/s",
            Some(before),
            &shown,
            "ms",
            wins.len(),
        ));
    }

    // ----- the guard: a fitted group the harness scores worse than the base's is not used -----
    let no_worse = |a: (usize, f64), b: (usize, f64)| a.0 < b.0 || (a.0 == b.0 && a.1 <= b.1);
    let mut motor_rise_ms = Some(tau * 1000.0);
    {
        let mut q = p.clone();
        q.motor.model = base.motor.model.clone();
        q.prop.kq = base.prop.kq.clone();
        q.prop.fm = base.prop.fm.clone();
        q.battery.electronics_current = base.battery.electronics_current.clone();
        q.battery.resistance = base.battery.resistance.clone();
        let (fit_s, base_s) = (
            group_score(&p, logs, Group::Power),
            group_score(&q, logs, Group::Power),
        );
        if !no_worse(fit_s, base_s) {
            p = q;
            motor_rise_ms = base.reference.motor_rise_ms;
            values.retain(|v| !POWER_VALUES.contains(&v.name.as_str()));
            notes.push(format!(
                "the fitted motor and pack validate worse than {}'s (hover, punch and sag: {} failed, against {}), so {}'s are kept",
                base.id, fit_s.0, base_s.0, base.id
            ));
        }
    }
    for (axis, name) in [(0, "roll"), (1, "pitch"), (2, "yaw")] {
        if p.fc.gains[axis].p.source != Source::Fitted {
            continue;
        }
        let mut q = p.clone();
        q.fc.gains[axis] = base.fc.gains[axis].clone();
        let (fit_s, base_s) = (
            group_score(&p, logs, Group::Axis(axis)),
            group_score(&q, logs, Group::Axis(axis)),
        );
        if !no_worse(fit_s, base_s) {
            p = q;
            let n = format!("fc.gains.{name}.p");
            values.retain(|v| v.name != n);
            notes.push(format!(
                "the fitted {name} gains validate worse than {}'s, so {}'s are kept",
                base.id, base.id
            ));
        }
    }

    // ----- what stays an estimate -----
    estimates.push(
        "frame.cda, duct.c_duct: blackbox logs carry no ground speed, so there are no coast-downs to fit drag to"
            .into(),
    );
    estimates.push("frame.inertia: the rate fit scales the controller gains, which act in angular acceleration, so inertia stays the geometry estimate".into());

    p.reference = crate::profile::Reference {
        hover_cmd: Some(hover_cmd),
        hover_rpm: Some(hover_rpm),
        tw_static_4v2: Some(thrust_at(1.0, 4.2).1 / weight),
        tw_static_3v6: Some(thrust_at(1.0, 3.6).1 / weight),
        motor_rise_ms,
        pack_resistance: Some(p.battery.resistance.value),
        hover_current: Some(hover_a),
        roll_lag_ms: measured
            .iter()
            .find(|m| m.name == "roll_lag")
            .map(|m| m.value),
    };
    Ok(FitReport {
        profile_id: p.id.clone(),
        base: base.id.clone(),
        logs: names,
        values,
        measured,
        thrust_curve,
        estimates,
        notes,
        profile: Some(p),
    })
}

/// Pack resistance from radio logs: per flight, `V = a + b·t − R·I` by least squares; the
/// flights' R weighted by their rows. None without current swings.
pub fn radio_resistance(flights: &[RadioFlight]) -> Option<(f64, usize)> {
    let (mut sum, mut w) = (0.0, 0usize);
    for f in flights {
        let ix: Vec<usize> = (0..f.t.len())
            .filter(|&i| f.volts[i] > 0.5 && f.amps[i] >= 0.0)
            .collect();
        if ix.len() < 30 {
            continue;
        }
        let rows: Vec<Vec<f64>> = ix.iter().map(|&i| vec![1.0, f.t[i], -f.amps[i]]).collect();
        let b: Vec<f64> = ix.iter().map(|&i| f.volts[i]).collect();
        let ma = ix.iter().map(|&i| f.amps[i]).sum::<f64>() / ix.len() as f64;
        let sd =
            (ix.iter().map(|&i| (f.amps[i] - ma).powi(2)).sum::<f64>() / ix.len() as f64).sqrt();
        if sd < 0.3 {
            continue;
        }
        if let Some(x) = lstsq(&rows, &b) {
            if x[2] > 0.0 && x[2] < 1.0 {
                sum += x[2] * ix.len() as f64;
                w += ix.len();
            }
        }
    }
    (w > 0).then(|| (sum / w as f64, w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn least_squares_recovers_a_line() {
        let rows: Vec<Vec<f64>> = (0..10).map(|i| vec![1.0, i as f64]).collect();
        let b: Vec<f64> = (0..10).map(|i| 3.0 - 0.5 * i as f64).collect();
        let x = lstsq(&rows, &b).unwrap();
        assert!((x[0] - 3.0).abs() < 1e-9 && (x[1] + 0.5).abs() < 1e-9);
    }

    #[test]
    fn golden_finds_a_minimum() {
        let x = golden(-3.0, 5.0, 60, |x| (x - 1.25).powi(2));
        assert!((x - 1.25).abs() < 1e-6);
    }

    #[test]
    fn radio_resistance_recovers_a_known_pack() {
        let t: Vec<f64> = (0..600).map(|i| i as f64 / 10.0).collect();
        let amps: Vec<f64> = t.iter().map(|x| 4.0 + 3.0 * (x * 1.7).sin()).collect();
        let volts: Vec<f64> = t
            .iter()
            .zip(&amps)
            .map(|(x, a)| 4.1 - 0.004 * x - 0.03 * a)
            .collect();
        let (r, n) = radio_resistance(&[RadioFlight {
            name: "f".into(),
            t,
            volts,
            amps,
        }])
        .unwrap();
        assert!((r - 0.03).abs() < 1e-6, "{r}");
        assert_eq!(n, 600);
    }

    #[test]
    fn steady_motor_balances_torque() {
        let kv = 21000.0 * RPM_TO_RADS;
        let (w, i) = steady_motor(kv, 0.14, 0.07, 2.0e-10, 0.6, 3.9);
        let ke = 1.0 / kv;
        assert!((ke * (i - 0.07) - 2.0e-10 * w * w).abs() < 1e-9);
        assert!(w > 0.0);
    }
}
