//! M0 determinism harness — the gate that proves `(seed, input_log)` replays
//! bit-identically (`docs/06` M0 exit test). Owned by the integrator.

use sim::{checksum, step, ArenaState, Input};

/// A reproducible match script: a seed, a length, and scripted inputs.
pub struct Scenario {
    pub master_seed: u64,
    pub player_id: u32,
    pub total_ticks: u32,
    /// `(tick, input)` events; all other ticks are `Noop`. Kept sorted by tick.
    pub scripted: Vec<(u32, Input)>,
}

/// A representative M0 script: buys across two rounds, a reroll, and a Clear.
pub fn m0_scenario() -> Scenario {
    Scenario {
        master_seed: 0xA5A5_1234_DEAD_BEEF,
        player_id: 0,
        total_ticks: 2000, // ~2 rounds @ 900 ticks/round
        scripted: vec![
            (5, Input::BuyOffer { slot: 0 }),
            (50, Input::BuyOffer { slot: 1 }),
            (200, Input::Reroll),
            (205, Input::BuyOffer { slot: 0 }),
            (400, Input::Clear),
            (950, Input::BuyOffer { slot: 0 }),
            (1300, Input::Reroll),
            (1305, Input::BuyOffer { slot: 2 }),
        ],
    }
}

#[inline]
fn input_at(sc: &Scenario, tick: u32) -> Input {
    for (t, i) in &sc.scripted {
        if *t == tick {
            return *i;
        }
    }
    Input::Noop
}

/// Run the scenario, returning the per-tick `state_checksum` trace.
pub fn run_trace(sc: &Scenario) -> Vec<u64> {
    let mut s = ArenaState::new(sc.master_seed, sc.player_id);
    let mut trace = Vec::with_capacity(sc.total_ticks as usize);
    for tick in 0..sc.total_ticks {
        step(&mut s, input_at(sc, tick));
        trace.push(checksum(&s));
    }
    trace
}

/// Final-state checksum for the scenario.
pub fn final_checksum(sc: &Scenario) -> u64 {
    *run_trace(sc).last().expect("non-empty scenario")
}
