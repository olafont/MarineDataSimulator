//! Overridable values. Every simulated quantity is a [`Param`]: the model computes
//! `modeled`, and the user can replace, offset or freeze what the rest of the system sees.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverrideMode {
    /// The effective value is the override value.
    #[default]
    Replace,
    /// The effective value is the modeled value plus the override value.
    Offset,
    /// The effective value stays at what it was when the freeze was set.
    Freeze,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub mode: OverrideMode,
    pub value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Param {
    pub modeled: f64,
    pub ovr: Option<Override>,
}

impl Param {
    pub const fn new(v: f64) -> Self {
        Self { modeled: v, ovr: None }
    }

    /// The value everything downstream should use.
    pub fn value(&self) -> f64 {
        match self.ovr {
            None => self.modeled,
            Some(Override { mode: OverrideMode::Replace | OverrideMode::Freeze, value }) => value,
            Some(Override { mode: OverrideMode::Offset, value }) => self.modeled + value,
        }
    }

    pub fn is_overridden(&self) -> bool {
        self.ovr.is_some()
    }

    /// Sets an override. A `Freeze` captures the current effective value and ignores `value`.
    pub fn set_override(&mut self, mode: OverrideMode, value: f64) {
        let value = match mode {
            OverrideMode::Freeze => self.value(),
            _ => value,
        };
        self.ovr = Some(Override { mode, value });
    }

    pub fn clear_override(&mut self) {
        self.ovr = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        let mut p = Param::new(10.0);
        assert_eq!(p.value(), 10.0);
        p.set_override(OverrideMode::Offset, 2.5);
        assert_eq!(p.value(), 12.5);
        p.set_override(OverrideMode::Freeze, 0.0);
        p.modeled = 99.0;
        assert_eq!(p.value(), 12.5);
        p.set_override(OverrideMode::Replace, 3.0);
        assert_eq!(p.value(), 3.0);
        p.clear_override();
        assert_eq!(p.value(), 99.0);
    }
}
