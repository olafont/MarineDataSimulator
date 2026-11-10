//! Time-varying environment: wind gusts and shifts, tide (height and stream), pressure trend.
//!
//! Gusts and shifts are Ornstein–Uhlenbeck processes: mean-reverting random signals with a
//! given standard deviation and correlation time, which is how real wind fluctuates.

use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus as StdRng;
use rand_distr::{Distribution, StandardNormal};
use serde::{Deserialize, Serialize};

use crate::geo::Ne;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindDynamics {
    /// Gust standard deviation as a fraction of the mean wind speed (0.15 = ±15 %).
    pub gust_fraction: f64,
    /// Gust correlation time, seconds.
    pub gust_period_s: f64,
    /// Direction shift standard deviation, degrees.
    pub shift_deg: f64,
    /// Shift correlation time, seconds.
    pub shift_period_s: f64,
}

impl Default for WindDynamics {
    fn default() -> Self {
        Self { gust_fraction: 0.12, gust_period_s: 20.0, shift_deg: 6.0, shift_period_s: 180.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TideConfig {
    pub enabled: bool,
    /// High-to-low water range, metres.
    pub range_m: f64,
    /// Tidal period, hours (12.42 = principal lunar semi-diurnal).
    pub period_h: f64,
    /// Time of a high water, Unix seconds.
    pub high_water_unix: f64,
    /// Direction the flood stream sets towards, degrees true.
    pub flood_set_deg: f64,
    /// Maximum stream rate (mid-flood / mid-ebb), knots.
    pub max_rate_kn: f64,
}

impl Default for TideConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            range_m: 2.0,
            period_h: 12.42,
            high_water_unix: 0.0,
            flood_set_deg: 90.0,
            max_rate_kn: 1.0,
        }
    }
}

impl TideConfig {
    fn phase(&self, t: f64) -> f64 {
        std::f64::consts::TAU * (t - self.high_water_unix) / (self.period_h.max(0.1) * 3600.0)
    }

    /// Height above chart datum (lowest water = 0), metres.
    pub fn height(&self, t: f64) -> f64 {
        if !self.enabled {
            return 0.0;
        }
        self.range_m / 2.0 * (1.0 + self.phase(t).cos())
    }

    /// Tidal stream vector, knots (towards). Strongest halfway between high and low water,
    /// setting towards `flood_set_deg` while the tide rises.
    pub fn stream(&self, t: f64) -> Ne {
        if !self.enabled {
            return Ne::default();
        }
        let rate = -self.max_rate_kn * self.phase(t).sin();
        Ne::from_polar(rate, self.flood_set_deg)
    }

    /// Seconds until the next high water after `t`.
    pub fn next_high_water_in(&self, t: f64) -> f64 {
        let p = self.period_h.max(0.1) * 3600.0;
        (self.high_water_unix - t).rem_euclid(p)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Dynamics {
    pub wind: WindDynamics,
    pub tide: TideConfig,
    /// Barometric trend, hPa per hour.
    pub pressure_trend_hpa_h: f64,
}

/// Mean-reverting random process with unit variance, scaled by the caller.
#[derive(Debug, Clone)]
pub struct Ou {
    pub x: f64,
}

impl Ou {
    pub fn step(&mut self, dt: f64, tau: f64, rng: &mut StdRng) -> f64 {
        if tau <= 0.0 {
            self.x = 0.0;
            return 0.0;
        }
        let n: f64 = StandardNormal.sample(rng);
        let a = (-dt / tau).exp();
        self.x = self.x * a + (1.0 - a * a).sqrt() * n;
        self.x
    }
}

/// Random state behind the dynamics.
#[derive(Debug, Clone)]
pub struct DynamicsState {
    pub gust: Ou,
    pub shift: Ou,
    pub rng: StdRng,
}

impl DynamicsState {
    pub fn new(seed: u64) -> Self {
        Self { gust: Ou { x: 0.0 }, shift: Ou { x: 0.0 }, rng: StdRng::seed_from_u64(seed ^ 0x0005_EED0_FA1D) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ou_has_unit_variance() {
        let mut s = DynamicsState::new(7);
        let (mut sum, mut sq, n) = (0.0, 0.0, 200_000);
        for _ in 0..n {
            let x = s.gust.step(0.1, 2.0, &mut s.rng);
            sum += x;
            sq += x * x;
        }
        let mean = sum / n as f64;
        let var = sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((var - 1.0).abs() < 0.1, "var {var}");
    }

    #[test]
    fn tide_height_and_stream() {
        let t = TideConfig { enabled: true, range_m: 4.0, high_water_unix: 0.0, ..Default::default() };
        assert!((t.height(0.0) - 4.0).abs() < 1e-9, "high water");
        let half = t.period_h * 1800.0;
        assert!(t.height(half).abs() < 1e-9, "low water");
        // Rising (flood) a quarter period before high water: stream sets towards flood_set.
        let flood = t.stream(-half / 2.0);
        assert!((flood.magnitude() - 1.0).abs() < 1e-9);
        assert!((flood.direction() - 90.0).abs() < 1e-6);
        assert!(t.stream(0.0).magnitude() < 1e-9, "slack at high water");
        assert!((t.next_high_water_in(10.0) - (t.period_h * 3600.0 - 10.0)).abs() < 1e-6);
    }
}
