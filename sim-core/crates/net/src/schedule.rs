//! Per-player input schedule: `arena_tick -> action`. Used identically by the
//! director (per shadow) and the client so an input applies on the SAME tick on
//! both sides (`lib.rs` input-timing invariant). Sharing this exact helper is
//! what prevents the two independent implementations from drifting.

use sim::Input;
use std::collections::BTreeMap;

#[derive(Default, Clone, Debug)]
pub struct Schedule {
    map: BTreeMap<u32, Input>,
}

impl Schedule {
    pub fn new() -> Schedule {
        Schedule::default()
    }

    /// Schedule `action` to apply at arena tick `apply_tick`. If two actions
    /// land on the same tick (rare), the later insert wins.
    pub fn set(&mut self, apply_tick: u32, action: Input) {
        self.map.insert(apply_tick, action);
    }

    /// The action to apply when stepping arena tick `tick` (consuming it).
    /// Returns `Input::Noop` if nothing is scheduled.
    pub fn take(&mut self, tick: u32) -> Input {
        self.map.remove(&tick).unwrap_or(Input::Noop)
    }

    /// Drop scheduled actions strictly before `tick` (housekeeping after a
    /// snapshot fast-forward).
    pub fn discard_before(&mut self, tick: u32) {
        self.map = self.map.split_off(&tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_take_roundtrip() {
        let mut s = Schedule::new();
        s.set(10, Input::Clear);
        assert_eq!(s.take(9), Input::Noop);
        assert_eq!(s.take(10), Input::Clear);
        assert_eq!(s.take(10), Input::Noop); // consumed
    }
}
