//! Quad profiles: the parameter set of sim-design 6.1, each value with its source, and the
//! built-in presets (a 65 mm and a 75 mm 1S whoop, a 5-inch and a 7-inch).
//!
//! A profile is data. `Params` is the plain numbers the physics runs on, built from it.
//! Units are SI throughout (kg, m, s, rad, N, V, A, Ω).

use serde::{Deserialize, Serialize};

use crate::diff::FcConfig;

/// Where a value came from (sim-design 6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// From logs, by the fit (S3); carries the log ids and residual.
    Fitted,
    /// Manufacturer's number.
    Spec,
    /// From the quad's `diff all`.
    Diff,
    /// Geometry or a typical value.
    Estimate,
    /// The sim's default.
    Default,
}

/// A number and its source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sv {
    pub value: f64,
    pub source: Source,
    /// Fitted values: the log ids the fit used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<String>,
    /// Fitted values: the fit's residual, in the value's own units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residual: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Sv {
    pub fn new(value: f64, source: Source) -> Sv {
        Sv {
            value,
            source,
            logs: Vec::new(),
            residual: None,
            note: None,
        }
    }
}

impl From<&Sv> for f64 {
    fn from(s: &Sv) -> f64 {
        s.value
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// All-up mass with the pack (kg).
    pub mass: Sv,
    pub pack_mass: Sv,
    /// Motor-to-motor diagonal (m).
    pub wheelbase: Sv,
    /// Principal inertia about body x (roll), y (pitch), z (yaw) (kg·m²).
    pub inertia: [Sv; 3],
    /// Drag area Cd·A for flow along body x, y, z (m²).
    pub cda: [Sv; 3],
    /// Body angular damping, as a rate (1/s): torque = -rate · I · ω.
    pub angular_damping: Sv,
    /// Collider: the body box's half extents (m).
    pub body_half: [Sv; 3],
}

/// The motor's dynamic model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum MotorModel {
    /// DC-motor electrical model (sim-design 4.2): winding resistance, no-load current and
    /// rotor inertia, with the prop's torque as the load.
    Electrical {
        r_winding: Sv,
        no_load_current: Sv,
        rotor_inertia: Sv,
    },
    /// The fallback for a profile with no fit: a first-order lag to a linear speed map,
    /// ω = idle + (fraction · Kv · V − idle) · command.
    FirstOrder {
        tau_up: Sv,
        tau_down: Sv,
        loaded_fraction: Sv,
        idle_rpm: Sv,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Motor {
    /// rpm per volt.
    pub kv: Sv,
    pub poles: Sv,
    #[serde(flatten)]
    pub model: MotorModel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prop {
    /// m.
    pub diameter: Sv,
    pub blades: Sv,
    /// m.
    pub pitch: Sv,
    /// Static thrust T = kT · ω² (N per (rad/s)²).
    pub kt: Sv,
    /// Figure of merit; gives kQ from kT when `kq` is absent.
    pub fm: Sv,
    /// Drag torque Q = kQ · ω² (N·m per (rad/s)²).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kq: Option<Sv>,
    /// Zero-thrust advance ratio: CT(J) = CT0 · (1 − J / J0).
    pub j0: Sv,
    /// Reversed (turtle) thrust as a fraction of forward thrust.
    pub reverse_factor: Sv,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Duct {
    pub ducted: bool,
    /// Rotor momentum drag coefficient: F = −c · ṁ · v_inplane (ducted, sim-design 4.4).
    pub c_duct: Sv,
    /// Open props: H-force F = −c_h · ω · v_inplane (N per (rad/s · m/s)).
    pub c_h: Sv,
    /// Collider: duct ring outer radius and height (m).
    pub radius: Sv,
    pub height: Sv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chemistry {
    Lipo,
    Lihv,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    pub cells: Sv,
    pub chemistry: Chemistry,
    pub capacity_mah: Sv,
    /// Pack plus wiring resistance (Ω).
    pub resistance: Sv,
    /// Electronics draw: FC, receiver, video (A).
    pub electronics_current: Sv,
}

/// One axis's sim PID gains, in physical units: the rate PID outputs angular acceleration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gains {
    /// 1/s.
    pub p: Sv,
    /// 1/s².
    pub i: Sv,
    /// s.
    pub d: Sv,
    /// Fraction of the setpoint's angular acceleration fed forward.
    pub ff: Sv,
}

/// The sim's own controller tuning (sim-design 5.3): fitted per profile, never the quad's PIDs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimFc {
    /// Roll, pitch, yaw.
    pub gains: [Gains; 3],
    pub dterm_lpf_hz: Sv,
    /// I-term relax: high-pass cutoff (Hz) and the setpoint rate that stops accumulation (deg/s).
    pub iterm_relax_hz: Sv,
    pub iterm_relax_threshold: Sv,
    /// I-term clamp (rad/s²).
    pub i_limit: Sv,
    /// TPA: D falls linearly from the breakpoint (throttle 0..1) to (1 − rate) at full.
    pub tpa_breakpoint: Sv,
    pub tpa_rate: Sv,
    /// The "Ideal" option: no PID, body rates follow setpoint through this first-order lag (s).
    pub ideal_lag: Sv,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub uptilt_deg: Sv,
    pub fov_deg: Sv,
    /// "4:3" or "16:9".
    pub aspect: String,
}

/// The example fit's measured values (sim-design 6.3), for display and the preset tests.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Reference {
    pub hover_cmd: Option<f64>,
    pub hover_rpm: Option<f64>,
    pub tw_static_4v2: Option<f64>,
    pub tw_static_3v6: Option<f64>,
    pub motor_rise_ms: Option<f64>,
    pub pack_resistance: Option<f64>,
    pub hover_current: Option<f64>,
    pub roll_lag_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimProfile {
    pub id: String,
    pub label: String,
    pub frame: Frame,
    pub motor: Motor,
    pub prop: Prop,
    pub duct: Duct,
    pub battery: Battery,
    pub fc: SimFc,
    /// From the quad's `diff all`; Betaflight defaults for a preset.
    pub diff: FcConfig,
    pub camera: Camera,
    #[serde(default)]
    pub reference: Reference,
}

/// Air density at sea level (kg/m³).
pub const RHO_SEA_LEVEL: f64 = 1.225;
pub const G: f64 = 9.80665;

pub const RPM_TO_RADS: f64 = std::f64::consts::TAU / 60.0;

/// The physics' plain numbers, from a profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub mass: f64,
    pub inertia: [f64; 3],
    pub cda: [f64; 3],
    pub angular_damping: f64,
    pub body_half: [f64; 3],
    /// Motor positions in the body frame (Betaflight Quad X order: rear right, front right,
    /// rear left, front left).
    pub rotor_pos: [[f64; 3]; 4],
    /// Spin seen from above: +1 counter-clockwise, −1 clockwise.
    pub rotor_spin: [f64; 4],
    pub kv_rads: f64,
    pub motor: MotorKind,
    pub idle_duty: f64,
    pub prop_radius: f64,
    pub prop_pitch: f64,
    pub kt: f64,
    pub kq: f64,
    pub j0: f64,
    pub reverse_factor: f64,
    pub ducted: bool,
    pub c_duct: f64,
    pub c_h: f64,
    pub duct_radius: f64,
    pub duct_height: f64,
    pub cells: f64,
    pub chemistry: Chemistry,
    pub capacity_mah: f64,
    pub pack_resistance: f64,
    pub electronics_current: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotorKind {
    Electrical {
        r: f64,
        i0: f64,
        j: f64,
    },
    FirstOrder {
        tau_up: f64,
        tau_down: f64,
        fraction: f64,
        idle_rads: f64,
    },
}

impl SimProfile {
    pub fn params(&self) -> Params {
        let f = &self.frame;
        let half = f.wheelbase.value / 2.0 / std::f64::consts::SQRT_2;
        // Quad X, Betaflight numbering, body x forward and y left.
        let rotor_pos = [
            [-half, -half, 0.0],
            [half, -half, 0.0],
            [-half, half, 0.0],
            [half, half, 0.0],
        ];
        // Props in (Betaflight's default): M1 and M4 clockwise, M2 and M3 counter-clockwise.
        let mut rotor_spin = [-1.0, 1.0, 1.0, -1.0];
        if self.diff.yaw_motors_reversed {
            for s in &mut rotor_spin {
                *s = -*s;
            }
        }
        let radius = self.prop.diameter.value / 2.0;
        let kt = self.prop.kt.value;
        let kq = match &self.prop.kq {
            Some(k) => k.value,
            None => kq_from_fm(kt, radius, self.prop.fm.value),
        };
        let motor = match &self.motor.model {
            MotorModel::Electrical {
                r_winding,
                no_load_current,
                rotor_inertia,
            } => MotorKind::Electrical {
                r: r_winding.value,
                i0: no_load_current.value,
                j: rotor_inertia.value,
            },
            MotorModel::FirstOrder {
                tau_up,
                tau_down,
                loaded_fraction,
                idle_rpm,
            } => MotorKind::FirstOrder {
                tau_up: tau_up.value,
                tau_down: tau_down.value,
                fraction: loaded_fraction.value,
                idle_rads: idle_rpm.value * RPM_TO_RADS,
            },
        };
        Params {
            mass: f.mass.value,
            inertia: [f.inertia[0].value, f.inertia[1].value, f.inertia[2].value],
            cda: [f.cda[0].value, f.cda[1].value, f.cda[2].value],
            angular_damping: f.angular_damping.value,
            body_half: [
                f.body_half[0].value,
                f.body_half[1].value,
                f.body_half[2].value,
            ],
            rotor_pos,
            rotor_spin,
            kv_rads: self.motor.kv.value * RPM_TO_RADS,
            motor,
            idle_duty: self.diff.dshot_idle_value / 10_000.0,
            prop_radius: radius,
            prop_pitch: self.prop.pitch.value,
            kt,
            kq,
            j0: self.prop.j0.value,
            reverse_factor: self.prop.reverse_factor.value,
            ducted: self.duct.ducted,
            c_duct: self.duct.c_duct.value,
            c_h: self.duct.c_h.value,
            duct_radius: self.duct.radius.value,
            duct_height: self.duct.height.value,
            cells: self.battery.cells.value,
            chemistry: self.battery.chemistry,
            capacity_mah: self.battery.capacity_mah.value,
            pack_resistance: self.battery.resistance.value,
            electronics_current: self.battery.electronics_current.value,
        }
    }

    /// Apply a quad's `diff all`: rates, throttle curve, modes and guards come from it.
    pub fn with_diff(mut self, diff_text: &str) -> SimProfile {
        self.diff = FcConfig::from_diff(diff_text);
        self
    }
}

/// kQ from kT and the figure of merit: ideal power T^1.5 / √(2ρA), real power that over FM,
/// and Q = P / ω (momentum theory, sim-design 4.3).
pub fn kq_from_fm(kt: f64, radius: f64, fm: f64) -> f64 {
    let area = std::f64::consts::PI * radius * radius;
    kt.powf(1.5) / ((2.0 * RHO_SEA_LEVEL * area).sqrt() * fm)
}

/// The figure of merit a kT and kQ imply.
pub fn fm_from_kq(kt: f64, kq: f64, radius: f64) -> f64 {
    let area = std::f64::consts::PI * radius * radius;
    kt.powf(1.5) / ((2.0 * RHO_SEA_LEVEL * area).sqrt() * kq)
}

const PRESETS: &[(&str, &str)] = &[
    ("air65ii", include_str!("../presets/air65ii.json")),
    ("meteor75", include_str!("../presets/meteor75.json")),
    ("five_inch", include_str!("../presets/five_inch.json")),
    ("seven_inch", include_str!("../presets/seven_inch.json")),
];

/// The built-in presets.
pub fn presets() -> Vec<SimProfile> {
    PRESETS
        .iter()
        .map(|(id, json)| serde_json::from_str(json).unwrap_or_else(|e| panic!("preset {id}: {e}")))
        .collect()
}

/// One built-in preset by id: `meteor75` (75 mm whoop), `air65ii` (65 mm whoop),
/// `five_inch`, `seven_inch`.
pub fn preset(id: &str) -> Option<SimProfile> {
    presets().into_iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_load_and_mark_every_value() {
        let all = presets();
        assert_eq!(all.len(), 4);
        for p in &all {
            let json = serde_json::to_value(p).unwrap();
            let mut n = 0;
            walk(&json, &mut |v| {
                if let Some(src) = v.get("source").and_then(|s| s.as_str()) {
                    n += 1;
                    assert!(
                        ["spec", "estimate", "default", "diff"].contains(&src),
                        "{}: a preset value marked {src}",
                        p.id
                    );
                }
            });
            assert!(n > 40, "{}: {n} sourced values", p.id);
            let pr = p.params();
            assert!(pr.mass > 0.0 && pr.kt > 0.0 && pr.kq > 0.0);
        }
        assert_eq!(preset("meteor75").unwrap().id, "meteor75");
        assert!(preset("air65ii").unwrap().duct.ducted);
    }

    fn walk(v: &serde_json::Value, f: &mut dyn FnMut(&serde_json::Value)) {
        f(v);
        match v {
            serde_json::Value::Object(m) => m.values().for_each(|x| walk(x, f)),
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, f)),
            _ => {}
        }
    }

    #[test]
    fn kq_and_fm_round_trip() {
        let k = kq_from_fm(2.9e-8, 0.0225, 0.3);
        assert!((fm_from_kq(2.9e-8, k, 0.0225) - 0.3).abs() < 1e-12);
    }
}
