//! Sailing polar diagrams: boat speed through water as a function of true wind angle and speed.
//!
//! Text format (the common CSV/TSV layout used by many navigation tools):
//!
//! ```text
//! twa/tws;6;8;10;12
//! 52;4.4;5.4;6.1;6.5
//! 90;5.2;6.2;6.9;7.2
//! ```
//!
//! Separators may be `;`, `,`, tab or spaces. The first row lists wind speeds (knots); each
//! following row starts with a true wind angle (degrees, 0–180) followed by boat speeds.
//! Lookups interpolate bilinearly, assume 0 kn at TWA 0 and at TWS 0, and clamp above the
//! highest wind speed.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Polar {
    tws: Vec<f64>,
    /// Rows sorted by TWA, starting with a 0° row.
    rows: Vec<(f64, Vec<f64>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolarError(pub String);

impl fmt::Display for PolarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "polar: {}", self.0)
    }
}

impl std::error::Error for PolarError {}

fn split(line: &str) -> Vec<&str> {
    line.split([';', ',', '\t', ' '])
        .filter(|s| !s.is_empty())
        .collect()
}

impl Polar {
    pub fn parse(text: &str) -> Result<Polar, PolarError> {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'));
        let header = lines.next().ok_or_else(|| PolarError("empty file".into()))?;
        let tws: Vec<f64> = split(header)
            .into_iter()
            .skip(1)
            .map(|s| s.parse::<f64>().map_err(|_| PolarError(format!("bad wind speed `{s}` in header"))))
            .collect::<Result<_, _>>()?;
        if tws.is_empty() || tws.windows(2).any(|w| w[1] <= w[0]) || tws[0] <= 0.0 {
            return Err(PolarError("header wind speeds must be positive and increasing".into()));
        }
        let mut rows = Vec::new();
        for (n, line) in lines.enumerate() {
            let cells = split(line);
            let nums: Vec<f64> = cells
                .iter()
                .map(|s| s.parse::<f64>().map_err(|_| PolarError(format!("row {}: bad number `{s}`", n + 2))))
                .collect::<Result<_, _>>()?;
            if nums.len() != tws.len() + 1 {
                return Err(PolarError(format!(
                    "row {}: expected {} values, found {}",
                    n + 2,
                    tws.len() + 1,
                    nums.len()
                )));
            }
            let twa = nums[0];
            if !(0.0..=180.0).contains(&twa) {
                return Err(PolarError(format!("row {}: TWA {twa} outside 0–180", n + 2)));
            }
            rows.push((twa, nums[1..].iter().map(|v| v.max(0.0)).collect()));
        }
        if rows.is_empty() {
            return Err(PolarError("no data rows".into()));
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.dedup_by(|a, b| a.0 == b.0);
        if rows[0].0 > 0.0 {
            rows.insert(0, (0.0, vec![0.0; tws.len()]));
        }
        Ok(Polar { tws, rows })
    }

    /// A generic 12 m cruising yacht (illustrative values).
    pub fn default_cruiser() -> Polar {
        Polar::parse(DEFAULT_POLAR).expect("built-in polar is valid")
    }

    fn row_speed(&self, row: &[f64], tws: f64) -> f64 {
        let n = self.tws.len();
        if tws <= 0.0 {
            return 0.0;
        }
        if tws <= self.tws[0] {
            return row[0] * tws / self.tws[0];
        }
        if tws >= self.tws[n - 1] {
            return row[n - 1];
        }
        let i = self.tws.partition_point(|&t| t <= tws) - 1;
        let f = (tws - self.tws[i]) / (self.tws[i + 1] - self.tws[i]);
        row[i] + (row[i + 1] - row[i]) * f
    }

    /// Target boat speed (knots) at true wind angle `twa` (any sign) and speed `tws` (knots).
    pub fn speed(&self, twa: f64, tws: f64) -> f64 {
        let a = crate::geo::norm_180(twa).abs();
        let rows = &self.rows;
        if a >= rows[rows.len() - 1].0 {
            return self.row_speed(&rows[rows.len() - 1].1, tws);
        }
        let j = rows.partition_point(|r| r.0 <= a) - 1;
        let (a0, r0) = &rows[j];
        let (a1, r1) = &rows[j + 1];
        let f = (a - a0) / (a1 - a0);
        let s0 = self.row_speed(r0, tws);
        let s1 = self.row_speed(r1, tws);
        s0 + (s1 - s0) * f
    }

    /// True wind angle with the best upwind velocity-made-good at `tws`.
    pub fn beat_angle(&self, tws: f64) -> f64 {
        best_angle(|a| self.speed(a, tws) * a.to_radians().cos(), 25..=80)
    }

    /// True wind angle with the best downwind velocity-made-good at `tws`.
    pub fn run_angle(&self, tws: f64) -> f64 {
        best_angle(|a| -self.speed(a, tws) * a.to_radians().cos(), 100..=180)
    }

    pub fn wind_speeds(&self) -> &[f64] {
        &self.tws
    }
}

fn best_angle(vmg: impl Fn(f64) -> f64, range: std::ops::RangeInclusive<u32>) -> f64 {
    let mut best = (f64::MIN, 45.0);
    for a in range {
        let v = vmg(a as f64);
        if v > best.0 {
            best = (v, a as f64);
        }
    }
    best.1
}

pub const DEFAULT_POLAR: &str = "\
# Generic 12 m cruising yacht, illustrative numbers (knots).
twa/tws;6;8;10;12;14;16;20;25
42;3.6;4.6;5.3;5.8;6.1;6.3;6.4;6.3
52;4.4;5.4;6.1;6.5;6.8;7.0;7.1;7.1
60;4.8;5.8;6.4;6.8;7.0;7.2;7.4;7.4
75;5.1;6.1;6.7;7.0;7.3;7.5;7.8;7.9
90;5.2;6.2;6.9;7.2;7.5;7.8;8.1;8.3
110;5.1;6.2;6.9;7.3;7.6;8.0;8.5;8.9
120;4.9;6.0;6.8;7.2;7.6;8.0;8.7;9.2
135;4.4;5.5;6.4;7.0;7.4;7.9;8.8;9.5
150;3.8;4.8;5.7;6.5;7.0;7.5;8.4;9.2
165;3.3;4.3;5.2;6.0;6.6;7.1;8.0;8.8
180;3.1;4.0;4.9;5.7;6.3;6.8;7.7;8.5
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_points_and_interpolation() {
        let p = Polar::default_cruiser();
        assert!((p.speed(90.0, 10.0) - 6.9).abs() < 1e-9);
        assert!((p.speed(-90.0, 10.0) - 6.9).abs() < 1e-9, "symmetric");
        assert!((p.speed(90.0, 9.0) - 6.55).abs() < 1e-9);
        assert_eq!(p.speed(0.0, 12.0), 0.0);
        assert!((p.speed(21.0, 12.0) - 5.8 / 2.0).abs() < 1e-9, "linear into the no-go zone");
        assert!((p.speed(90.0, 40.0) - 8.3).abs() < 1e-9, "clamped above max tws");
        assert!((p.speed(90.0, 3.0) - 2.6).abs() < 1e-9, "scaled below min tws");
    }

    #[test]
    fn optimal_angles_are_plausible() {
        let p = Polar::default_cruiser();
        let beat = p.beat_angle(12.0);
        assert!((38.0..=55.0).contains(&beat), "beat {beat}");
        let run = p.run_angle(12.0);
        assert!((140.0..=180.0).contains(&run), "run {run}");
    }

    #[test]
    fn parse_errors_are_descriptive() {
        assert!(Polar::parse("twa/tws\t6\t8\n90\t5\n").unwrap_err().0.contains("expected 3"));
        assert!(Polar::parse("twa;8;6\n90;5;5\n").is_err());
        let tab = Polar::parse("twa/tws\t6\t8\n90\t5\t6\n").unwrap();
        assert_eq!(tab.speed(90.0, 7.0), 5.5);
    }
}
