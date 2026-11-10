//! Wire-format encoders. Pure functions: measurements in, sentences/frames out.
//! NMEA 0183 and AIS (`!AIVDM`/`!AIVDO`). NMEA 2000 gateway formats (`n2k`), Signal K deltas (`signalk`).

pub mod ais;
pub mod n2k;
pub mod nmea0183;
pub mod signalk;
