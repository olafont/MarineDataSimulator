//! Seabed depth from scattered soundings (inverse-distance weighting).

use serde::{Deserialize, Serialize};

use crate::geo::LatLon;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sounding {
    pub lat: f64,
    pub lon: f64,
    /// Depth below chart datum, metres.
    pub depth: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bathymetry {
    /// When empty, the environment's `seabed_depth` is used everywhere.
    pub soundings: Vec<Sounding>,
    /// Number of nearest soundings blended for each lookup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neighbours: Option<usize>,
}

impl Bathymetry {
    pub fn is_empty(&self) -> bool {
        self.soundings.is_empty()
    }

    /// Interpolated chart depth at `p`, or `None` when there are no soundings.
    pub fn depth_at(&self, p: LatLon) -> Option<f64> {
        if self.soundings.is_empty() {
            return None;
        }
        let k = self.neighbours.unwrap_or(4).clamp(1, self.soundings.len());
        // Squared distances in a local flat projection are enough for ranking and weighting.
        let coslat = p.lat.to_radians().cos();
        let mut near: Vec<(f64, f64)> = self
            .soundings
            .iter()
            .map(|s| {
                let dy = (s.lat - p.lat) * 111_195.0;
                let dx = (s.lon - p.lon) * 111_195.0 * coslat;
                (dx * dx + dy * dy, s.depth)
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let near = &near[..k];
        if near[0].0 < 1.0 {
            return Some(near[0].1);
        }
        let (mut w_sum, mut d_sum) = (0.0, 0.0);
        for (d2, depth) in near {
            let w = 1.0 / d2; // inverse distance squared
            w_sum += w;
            d_sum += w * depth;
        }
        Some(d_sum / w_sum)
    }

    /// Parses `lat,lon,depth` lines (`;`, `,`, tab or space separated; `#` comments and a
    /// non-numeric header line are skipped).
    pub fn parse_csv(text: &str) -> Result<Vec<Sounding>, String> {
        let mut out = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cells: Vec<&str> = line
                .split([';', ',', '\t', ' '])
                .filter(|s| !s.is_empty())
                .collect();
            let nums: Result<Vec<f64>, _> = cells.iter().map(|s| s.parse::<f64>()).collect();
            match nums {
                Ok(v) if v.len() >= 3 => out.push(Sounding { lat: v[0], lon: v[1], depth: v[2] }),
                Err(_) if out.is_empty() && n == 0 => continue, // header
                _ => return Err(format!("line {}: expected `lat,lon,depth`", n + 1)),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_between_soundings() {
        let b = Bathymetry {
            soundings: vec![
                Sounding { lat: 0.0, lon: 0.0, depth: 10.0 },
                Sounding { lat: 0.0, lon: 0.01, depth: 30.0 },
            ],
            neighbours: None,
        };
        let mid = b.depth_at(LatLon::new(0.0, 0.005)).unwrap();
        assert!((mid - 20.0).abs() < 1e-6);
        let near_a = b.depth_at(LatLon::new(0.0, 0.001)).unwrap();
        assert!(near_a < 11.0);
        assert_eq!(b.depth_at(LatLon::new(0.0, 0.0)), Some(10.0));
    }

    #[test]
    fn csv() {
        let s = Bathymetry::parse_csv("lat,lon,depth\n43.2,5.3,40\n# c\n43.21 5.31 42.5\n").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].depth, 42.5);
        assert!(Bathymetry::parse_csv("1,2\n").is_err());
    }
}
