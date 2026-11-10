//! Wave-induced motion (roll, pitch, heave).
//!
//! The sea is a handful of sinusoidal components around the peak period. Each component is
//! seen at its encounter frequency (Doppler-shifted by the vessel's speed and heading relative to
//! the waves). Roll goes through a damped resonance at the vessel's natural roll period.
//! This is phenomenological, chosen to give plausible instrument data rather than to model
//! seakeeping in detail.

use std::f64::consts::TAU;

use rand::RngExt as _;
use rand::rngs::Xoshiro256PlusPlus as StdRng;

use crate::geo::norm_180;

const G: f64 = 9.81;
/// Relative frequencies and energy weights of the components.
const COMPONENTS: [(f64, f64); 5] = [(0.75, 0.15), (0.9, 0.3), (1.0, 0.3), (1.15, 0.15), (1.35, 0.1)];

#[derive(Debug, Clone)]
pub struct WaveState {
    phases: [f64; COMPONENTS.len()],
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaveMotion {
    /// Degrees, + = starboard side down.
    pub roll: f64,
    /// Degrees, + = bow up.
    pub pitch: f64,
    /// Metres, + = up.
    pub heave: f64,
}

pub struct WaveInput {
    pub hs_m: f64,
    pub period_s: f64,
    /// Direction the waves come from, degrees true.
    pub from_deg: f64,
    pub heading: f64,
    pub stw_kn: f64,
    pub roll_period_s: f64,
}

impl WaveState {
    pub fn new(rng: &mut StdRng) -> Self {
        let mut phases = [0.0; COMPONENTS.len()];
        for p in &mut phases {
            *p = rng.random_range(0.0..TAU);
        }
        Self { phases }
    }

    /// Advances by `dt` seconds and returns the motion.
    pub fn step(&mut self, dt: f64, w: &WaveInput) -> WaveMotion {
        if w.hs_m <= 0.0 || w.period_s <= 0.0 {
            return WaveMotion::default();
        }
        // 0° = head seas, 90° = beam seas from starboard.
        let rel = norm_180(w.from_deg - w.heading).to_radians();
        let v = w.stw_kn * crate::geo::MPS_PER_KNOT;
        let wp = TAU / w.period_s;
        let wn = TAU / w.roll_period_s.max(1.0);
        let zeta = 0.12;
        let mut m = WaveMotion::default();
        for (i, (f, weight)) in COMPONENTS.iter().enumerate() {
            let omega = wp * f;
            // Encounter frequency: head seas are met faster, following seas slower.
            let we = (omega + omega * omega * v / G * rel.cos()).abs().max(0.05);
            self.phases[i] = (self.phases[i] + we * dt) % TAU;
            let s = self.phases[i].sin();
            let amp = w.hs_m / 2.0 * weight.sqrt();
            let slope = omega * omega / G * amp; // wave slope, radians
            let r = we / wn;
            let resonance = 1.0 / ((1.0 - r * r).powi(2) + (2.0 * zeta * r).powi(2)).sqrt();
            m.heave += amp * s;
            m.pitch += (slope * rel.cos().abs() * 0.8 * s).to_degrees();
            m.roll += (slope * rel.sin() * resonance.min(3.0) * 0.3 * self.phases[i].cos()).to_degrees();
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rms(f: impl Fn(&WaveMotion) -> f64, input: &WaveInput) -> f64 {
        let mut rng = StdRng::seed_from_u64(3);
        let mut s = WaveState::new(&mut rng);
        let n = 6000;
        (0..n).map(|_| f(&s.step(0.1, input)).powi(2)).sum::<f64>().sqrt() / (n as f64).sqrt()
    }

    fn input(from: f64) -> WaveInput {
        WaveInput { hs_m: 2.0, period_s: 7.0, from_deg: from, heading: 0.0, stw_kn: 6.0, roll_period_s: 5.0 }
    }

    #[test]
    fn beam_seas_roll_head_seas_pitch() {
        let beam_roll = rms(|m| m.roll, &input(90.0));
        let head_roll = rms(|m| m.roll, &input(0.0));
        let head_pitch = rms(|m| m.pitch, &input(0.0));
        let beam_pitch = rms(|m| m.pitch, &input(90.0));
        assert!(beam_roll > 5.0 * head_roll.max(0.01), "roll beam {beam_roll} head {head_roll}");
        assert!(head_pitch > 5.0 * beam_pitch.max(0.01), "pitch head {head_pitch} beam {beam_pitch}");
        assert!((1.0..25.0).contains(&beam_roll), "beam roll rms {beam_roll}");
    }

    #[test]
    fn calm_sea_is_still() {
        let mut rng = StdRng::seed_from_u64(1);
        let mut s = WaveState::new(&mut rng);
        let mut i = input(90.0);
        i.hs_m = 0.0;
        assert_eq!(s.step(0.1, &i), WaveMotion::default());
    }
}
