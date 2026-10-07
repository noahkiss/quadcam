//! The flight pack (sim-design 4.8).
//!
//! Off by default: an ideal pack at a constant voltage per cell (3.9 V, sim-design 12 #4),
//! no depletion. With sag on: open-circuit voltage from state of charge, minus total current
//! times the pack-plus-wiring resistance, solved exactly each step against the motors'
//! affine current draw. The equivalent-circuit model is fact; propwash's `Battery.ts` and
//! `FlightBattery.ts` (MIT, `LICENSES/propwash.txt`) were the reference.

use crate::profile::{Chemistry, Params};

/// Constant voltage per cell with sag off (sim-design 12, decision 4).
pub const IDEAL_CELL_V: f64 = 3.9;
/// Low-voltage warning per cell, held this long (s), with sag on.
const LOW_CELL_V: f64 = 3.3;
const LOW_HOLD_S: f64 = 0.5;

/// Resting voltage per cell against state of charge (0..1), typical 1S curves (estimate).
const LIPO_OCV: [(f64, f64); 12] = [
    (0.0, 3.27),
    (0.05, 3.61),
    (0.1, 3.69),
    (0.2, 3.73),
    (0.3, 3.77),
    (0.4, 3.79),
    (0.5, 3.82),
    (0.6, 3.87),
    (0.7, 3.93),
    (0.8, 4.0),
    (0.9, 4.08),
    (1.0, 4.2),
];
const LIHV_OCV: [(f64, f64); 12] = [
    (0.0, 3.3),
    (0.05, 3.63),
    (0.1, 3.72),
    (0.2, 3.77),
    (0.3, 3.81),
    (0.4, 3.84),
    (0.5, 3.88),
    (0.6, 3.94),
    (0.7, 4.02),
    (0.8, 4.1),
    (0.9, 4.2),
    (1.0, 4.35),
];

pub fn ocv_per_cell(chem: Chemistry, soc: f64) -> f64 {
    let t: &[(f64, f64)] = match chem {
        Chemistry::Lipo => &LIPO_OCV,
        Chemistry::Lihv => &LIHV_OCV,
    };
    let s = soc.clamp(0.0, 1.0);
    for w in t.windows(2) {
        let ((s0, v0), (s1, v1)) = (w[0], w[1]);
        if s <= s1 {
            return v0 + (v1 - v0) * (s - s0) / (s1 - s0);
        }
    }
    t[t.len() - 1].1
}

/// The state of charge whose resting voltage is `v_cell`.
pub fn soc_for_cell_voltage(chem: Chemistry, v_cell: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if ocv_per_cell(chem, mid) < v_cell {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

#[derive(Debug, Clone)]
pub struct Battery {
    pub sag: bool,
    pub cells: f64,
    pub chemistry: Chemistry,
    pub capacity_mah: f64,
    pub resistance: f64,
    pub electronics_current: f64,
    /// Constant voltage per cell with sag off.
    pub ideal_cell_v: f64,
    pub used_mah: f64,
    /// Loaded pack voltage and total current, last step.
    pub voltage: f64,
    pub current: f64,
    pub low_warning: bool,
    low_for: f64,
}

impl Battery {
    pub fn new(p: &Params, sag: bool) -> Battery {
        let mut b = Battery {
            sag,
            cells: p.cells,
            chemistry: p.chemistry,
            capacity_mah: p.capacity_mah,
            resistance: p.pack_resistance,
            electronics_current: p.electronics_current,
            ideal_cell_v: IDEAL_CELL_V,
            used_mah: 0.0,
            voltage: 0.0,
            current: 0.0,
            low_warning: false,
            low_for: 0.0,
        };
        b.voltage = b.resting_voltage();
        b
    }

    pub fn soc(&self) -> f64 {
        (1.0 - self.used_mah / self.capacity_mah).max(0.0)
    }

    /// Start a sag-on pack at this resting voltage per cell.
    pub fn set_resting_cell_voltage(&mut self, v_cell: f64) {
        let soc = soc_for_cell_voltage(self.chemistry, v_cell);
        self.used_mah = (1.0 - soc) * self.capacity_mah;
        self.voltage = self.resting_voltage();
    }

    pub fn resting_voltage(&self) -> f64 {
        if self.sag {
            ocv_per_cell(self.chemistry, self.soc()) * self.cells
        } else {
            self.ideal_cell_v * self.cells
        }
    }

    /// The loaded voltage for a motor draw of `slope · V + offset` amps (see
    /// `Motors::pack_current_affine`), plus the electronics.
    pub fn loaded_voltage(&self, slope: f64, offset: f64) -> f64 {
        let ocv = self.resting_voltage();
        if !self.sag {
            return ocv;
        }
        let r = self.resistance;
        // V = OCV − R · (slope·V + offset + Ie)
        let v = (ocv - r * (offset + self.electronics_current)) / (1.0 + r * slope);
        v.max(0.0)
    }

    /// Book the step's current (A) at voltage `v`.
    pub fn update(&mut self, dt: f64, v: f64, motor_current: f64) {
        self.voltage = v;
        self.current = motor_current + self.electronics_current;
        if self.sag {
            self.used_mah += self.current * dt / 3.6;
            let cell = v / self.cells;
            self.low_for = if cell < LOW_CELL_V {
                self.low_for + dt
            } else {
                0.0
            };
            self.low_warning = self.low_for >= LOW_HOLD_S;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motor::Motors;
    use crate::profile::preset;

    #[test]
    fn ideal_pack_holds_its_voltage() {
        let p = preset("meteor75").unwrap().params();
        let mut b = Battery::new(&p, false);
        assert_eq!(b.loaded_voltage(400.0, -100.0), 3.9);
        b.update(1.0, 3.9, 30.0);
        assert_eq!(b.used_mah, 0.0);
        assert_eq!(b.resting_voltage(), 3.9);
    }

    #[test]
    fn sag_solves_ohms_law_and_counts_mah() {
        let p = preset("meteor75").unwrap().params();
        let mut b = Battery::new(&p, true);
        b.set_resting_cell_voltage(4.0);
        assert!((b.resting_voltage() - 4.0).abs() < 1e-6);
        let mut m = Motors::new(&p);
        for _ in 0..4000 {
            let (s, o) = m.pack_current_affine(&[1.0; 4], true, b.voltage);
            let v = b.loaded_voltage(s, o);
            m.step(5e-4, &[1.0; 4], v, true);
            let i: f64 = m.pack_current.iter().sum();
            b.update(5e-4, v, i);
            // Exact: V = OCV − R · I.
            let ocv = b.resting_voltage();
            assert!((v - (ocv - b.resistance * b.current)).abs() < 0.02);
        }
        // A 1S punch sags 20-25 % (sim-design 4.8).
        let sag = 1.0 - b.voltage / 4.0;
        assert!((0.12..0.3).contains(&sag), "sag {sag}");
        assert!(b.used_mah > 0.0);
    }

    #[test]
    fn ocv_curve_round_trips() {
        for chem in [Chemistry::Lipo, Chemistry::Lihv] {
            for v in [3.7, 3.9, 4.1] {
                let s = soc_for_cell_voltage(chem, v);
                assert!((ocv_per_cell(chem, s) - v).abs() < 1e-6);
            }
        }
    }
}
