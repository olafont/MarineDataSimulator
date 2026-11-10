//! Simulation core: physics, environment and observed channels. No I/O.

pub mod ais;
pub mod bathymetry;
pub mod channels;
pub mod dynamics;
pub mod geo;
pub mod param;
pub mod polar;
pub mod sim;
pub mod vessel;
pub mod waves;

pub use ais::{AisClass, AisMessage, AisTarget, AisTargetConfig, OwnAis, TargetMotion};
pub use bathymetry::{Bathymetry, Sounding};
pub use channels::{Channel, Channels, EnvBase, EnvParam, EnvParams, Kind, Meta};
pub use dynamics::{Dynamics, TideConfig, WindDynamics};
pub use geo::{LatLon, Ne};
pub use param::{OverrideMode, Param};
pub use polar::Polar;
pub use sim::{Command, InitialState, Simulation};
pub use vessel::{Propulsion, Steering, VesselConfig};
