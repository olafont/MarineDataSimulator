//! Named, overridable quantities.
//!
//! * [`EnvParam`] — environment *inputs* to the physics (wind, current, seabed depth…).
//!   Overriding one changes how the vessel behaves.
//! * [`Channel`] — *observed* values the physics produces and the instruments report
//!   (position, heading, apparent wind…). Overriding one changes only what is reported.

use serde::{Deserialize, Serialize};

use crate::param::Param;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Plain linear value.
    Linear,
    /// Angle in [0, 360).
    Bearing,
    /// Angle in (-180, 180].
    Relative,
    Latitude,
    Longitude,
}

#[derive(Debug, Clone, Copy)]
pub struct Meta {
    /// Stable identifier used in scenario files.
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub kind: Kind,
    /// Suggested UI range.
    pub min: f64,
    pub max: f64,
}

macro_rules! quantity_enum {
    ($(#[$m:meta])* $name:ident, $store:ident {
        $($var:ident => ($key:literal, $label:literal, $unit:literal, $kind:ident, $min:expr, $max:expr)),* $(,)?
    }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($var),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),*];
            pub const COUNT: usize = Self::ALL.len();

            pub fn meta(self) -> Meta {
                match self {
                    $($name::$var => Meta {
                        key: $key, label: $label, unit: $unit, kind: Kind::$kind, min: $min, max: $max,
                    }),*
                }
            }

            pub fn from_key(key: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|c| c.meta().key == key)
            }
        }

        /// Fixed-size storage with one [`Param`] per variant.
        #[derive(Debug, Clone, Default)]
        pub struct $store([Param; $name::COUNT]);

        impl $store {
            pub fn get(&self, c: $name) -> f64 {
                self.0[c as usize].value()
            }
            pub fn param(&self, c: $name) -> &Param {
                &self.0[c as usize]
            }
            pub fn param_mut(&mut self, c: $name) -> &mut Param {
                &mut self.0[c as usize]
            }
            /// Sets the modeled (non-overridden) value.
            pub fn set_modeled(&mut self, c: $name, v: f64) {
                self.0[c as usize].modeled = v;
            }
        }
    };
}

quantity_enum!(
    /// Environment inputs.
    EnvParam, EnvParams {
        WindSpeed        => ("wind_speed", "True wind speed", "kn", Linear, 0.0, 80.0),
        WindDirection    => ("wind_direction", "True wind direction (from)", "°T", Bearing, 0.0, 360.0),
        CurrentSpeed     => ("current_speed", "Current speed (drift)", "kn", Linear, 0.0, 8.0),
        CurrentSet       => ("current_set", "Current set (towards)", "°T", Bearing, 0.0, 360.0),
        SeabedDepth      => ("seabed_depth", "Water depth (chart + tide)", "m", Linear, 0.0, 500.0),
        WaterTemp        => ("water_temp", "Water temperature", "°C", Linear, -2.0, 35.0),
        AirTemp          => ("air_temp", "Air temperature", "°C", Linear, -20.0, 45.0),
        Pressure         => ("pressure", "Barometric pressure", "hPa", Linear, 950.0, 1050.0),
        Humidity         => ("humidity", "Relative humidity", "%", Linear, 0.0, 100.0),
        MagVariation     => ("magnetic_variation", "Magnetic variation (+E)", "°", Relative, -30.0, 30.0),
        WaveHeight       => ("wave_height", "Significant wave height", "m", Linear, 0.0, 12.0),
        WavePeriod       => ("wave_period", "Wave peak period", "s", Linear, 1.0, 20.0),
        WaveDirection    => ("wave_direction", "Wave direction (from)", "°T", Bearing, 0.0, 360.0),
    }
);

/// User-set base values of the environment. The model adds dynamics (gusts, tide, bathymetry…)
/// on top to produce the modeled [`EnvParams`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvBase(pub [f64; EnvParam::COUNT]);

impl EnvBase {
    pub fn get(&self, p: EnvParam) -> f64 {
        self.0[p as usize]
    }
    pub fn set(&mut self, p: EnvParam, v: f64) {
        self.0[p as usize] = v;
    }
}

quantity_enum!(
    /// Observed values produced by the physics, before sensor noise.
    Channel, Channels {
        Latitude      => ("latitude", "Latitude", "°", Latitude, -90.0, 90.0),
        Longitude     => ("longitude", "Longitude", "°", Longitude, -180.0, 180.0),
        HeadingTrue   => ("heading_true", "Heading", "°T", Bearing, 0.0, 360.0),
        MagVariation  => ("magnetic_variation", "Magnetic variation (+E)", "°", Relative, -30.0, 30.0),
        RateOfTurn    => ("rate_of_turn", "Rate of turn", "°/min", Linear, -720.0, 720.0),
        Rudder        => ("rudder", "Rudder angle (+stbd)", "°", Relative, -45.0, 45.0),
        Roll          => ("roll", "Roll / heel (+stbd down)", "°", Relative, -60.0, 60.0),
        Pitch         => ("pitch", "Pitch (+bow up)", "°", Relative, -30.0, 30.0),
        Heave         => ("heave", "Heave (+up)", "m", Linear, -10.0, 10.0),
        Leeway        => ("leeway", "Leeway (+to stbd)", "°", Relative, -20.0, 20.0),
        EngineRpm     => ("engine_rpm", "Engine speed", "rpm", Linear, 0.0, 6000.0),
        Stw           => ("stw", "Speed through water", "kn", Linear, 0.0, 50.0),
        Sog           => ("sog", "Speed over ground", "kn", Linear, 0.0, 50.0),
        Cog           => ("cog", "Course over ground", "°T", Bearing, 0.0, 360.0),
        Depth         => ("depth", "Depth below surface", "m", Linear, 0.0, 500.0),
        Tws           => ("tws", "True wind speed", "kn", Linear, 0.0, 80.0),
        Twa           => ("twa", "True wind angle", "°", Relative, -180.0, 180.0),
        Twd           => ("twd", "True wind direction", "°T", Bearing, 0.0, 360.0),
        Aws           => ("aws", "Apparent wind speed", "kn", Linear, 0.0, 80.0),
        Awa           => ("awa", "Apparent wind angle", "°", Relative, -180.0, 180.0),
        WaterTemp     => ("water_temp", "Water temperature", "°C", Linear, -2.0, 35.0),
        AirTemp       => ("air_temp", "Air temperature", "°C", Linear, -20.0, 45.0),
        Pressure      => ("pressure", "Barometric pressure", "hPa", Linear, 950.0, 1050.0),
        Humidity      => ("humidity", "Relative humidity", "%", Linear, 0.0, 100.0),
    }
);
