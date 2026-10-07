//! Seeded random numbers. One stream per subsystem, drawn every step whether the subsystem
//! uses it or not, so a replay never depends on the flight path (sim-design 2.2).
//!
//! xoshiro256** seeded through SplitMix64: both are public-domain algorithms (Blackman and
//! Vigna), written here from their published definitions.

#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
    spare: Option<f64>,
}

fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut x = seed;
        let s = [
            splitmix(&mut x),
            splitmix(&mut x),
            splitmix(&mut x),
            splitmix(&mut x),
        ];
        Rng { s, spare: None }
    }

    /// An independent stream for a named subsystem.
    pub fn fork(seed: u64, name: &str) -> Rng {
        let mut h = seed ^ 0xA076_1D64_78BD_642F;
        for b in name.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
        }
        Rng::new(h)
    }

    pub fn next_u64(&mut self) -> u64 {
        let r = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Standard normal (Box-Muller, both values used).
    pub fn gaussian(&mut self) -> f64 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        let u1 = self.uniform().max(1e-300);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let a = std::f64::consts::TAU * u2;
        self.spare = Some(r * a.sin());
        r * a.cos()
    }

    pub fn state_words(&self) -> [u64; 4] {
        self.s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_and_forks_differ() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(
            Rng::fork(7, "wash").next_u64(),
            Rng::fork(7, "gyro").next_u64()
        );
    }

    #[test]
    fn gaussian_has_unit_variance() {
        let mut r = Rng::new(1);
        let n = 200_000;
        let (mut s, mut s2) = (0.0, 0.0);
        for _ in 0..n {
            let g = r.gaussian();
            s += g;
            s2 += g * g;
        }
        let mean = s / n as f64;
        let var = s2 / n as f64 - mean * mean;
        assert!(mean.abs() < 0.01, "{mean}");
        assert!((var - 1.0).abs() < 0.02, "{var}");
    }
}
