//! Small geodesy and angle helpers. Distances in metres, angles in degrees.

use serde::{Deserialize, Serialize};

/// Mean Earth radius (IUGG), metres.
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;
/// Metres per second in one knot.
pub const MPS_PER_KNOT: f64 = 1852.0 / 3600.0;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub const fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// Great-circle distance in metres (haversine).
    pub fn distance_to(&self, other: &LatLon) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), other.lat.to_radians());
        let dp = p2 - p1;
        let dl = (other.lon - self.lon).to_radians();
        let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M * a.sqrt().asin()
    }

    /// Initial great-circle bearing to `other`, degrees true in [0, 360).
    pub fn bearing_to(&self, other: &LatLon) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), other.lat.to_radians());
        let dl = (other.lon - self.lon).to_radians();
        let y = dl.sin() * p2.cos();
        let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
        norm_360(y.atan2(x).to_degrees())
    }

    /// Moves by a local north/east displacement in metres. Accurate for the small steps the
    /// simulator takes; longitude is wrapped to (-180, 180].
    pub fn offset_ne(&self, north_m: f64, east_m: f64) -> LatLon {
        let lat = self.lat + (north_m / EARTH_RADIUS_M).to_degrees();
        let coslat = self.lat.to_radians().cos().max(1e-9);
        let lon = self.lon + (east_m / (EARTH_RADIUS_M * coslat)).to_degrees();
        LatLon { lat: lat.clamp(-89.999_999, 89.999_999), lon: norm_180(lon) }
    }

    /// Point reached travelling `distance_m` along `bearing_deg` (great circle).
    pub fn destination(&self, bearing_deg: f64, distance_m: f64) -> LatLon {
        let d = distance_m / EARTH_RADIUS_M;
        let b = bearing_deg.to_radians();
        let p1 = self.lat.to_radians();
        let l1 = self.lon.to_radians();
        let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * b.cos()).asin();
        let l2 = l1 + (b.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
        LatLon { lat: p2.to_degrees(), lon: norm_180(l2.to_degrees()) }
    }
}

/// Normalises an angle to [0, 360).
pub fn norm_360(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    if r >= 360.0 { 0.0 } else { r }
}

/// Normalises an angle to (-180, 180].
pub fn norm_180(deg: f64) -> f64 {
    let r = norm_360(deg);
    if r > 180.0 { r - 360.0 } else { r }
}

/// A 2-D vector in the local north/east frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ne {
    pub n: f64,
    pub e: f64,
}

impl Ne {
    /// Vector pointing *towards* `dir_deg` with magnitude `mag`.
    pub fn from_polar(mag: f64, dir_deg: f64) -> Ne {
        let r = dir_deg.to_radians();
        Ne { n: mag * r.cos(), e: mag * r.sin() }
    }
    pub fn magnitude(&self) -> f64 {
        self.n.hypot(self.e)
    }
    /// Direction the vector points towards, degrees in [0, 360).
    pub fn direction(&self) -> f64 {
        norm_360(self.e.atan2(self.n).to_degrees())
    }
}

impl std::ops::Add for Ne {
    type Output = Ne;
    fn add(self, o: Ne) -> Ne {
        Ne { n: self.n + o.n, e: self.e + o.e }
    }
}
impl std::ops::Sub for Ne {
    type Output = Ne;
    fn sub(self, o: Ne) -> Ne {
        Ne { n: self.n - o.n, e: self.e - o.e }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation() {
        assert_eq!(norm_360(-10.0), 350.0);
        assert_eq!(norm_360(720.0), 0.0);
        assert_eq!(norm_180(190.0), -170.0);
        assert_eq!(norm_180(-180.0), 180.0);
    }

    #[test]
    fn one_minute_of_latitude_is_about_a_nautical_mile() {
        let a = LatLon::new(45.0, 0.0);
        let b = LatLon::new(45.0 + 1.0 / 60.0, 0.0);
        assert!((a.distance_to(&b) - 1853.2).abs() < 2.0);
        assert!((a.bearing_to(&b) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn destination_round_trip() {
        let a = LatLon::new(43.3, 5.35);
        let b = a.destination(73.0, 10_000.0);
        assert!((a.distance_to(&b) - 10_000.0).abs() < 0.01);
        assert!((a.bearing_to(&b) - 73.0).abs() < 0.01);
    }

    #[test]
    fn offset_matches_destination_for_small_steps() {
        let a = LatLon::new(50.0, -1.0);
        let v = Ne::from_polar(100.0, 130.0);
        let b = a.offset_ne(v.n, v.e);
        let c = a.destination(130.0, 100.0);
        assert!(b.distance_to(&c) < 0.01);
    }
}
