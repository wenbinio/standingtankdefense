//! In-process shadow-sim driver (M1) — AGENT M1B implements the `todo!()` bodies.
//!
//! Models `docs/03 §3.9` locally: a `client` sim and an authoritative `shadow`
//! sim advance from the same seed+inputs. Every `digest_interval` ticks their
//! `state_checksum` are compared; on mismatch the client is CORRECTED from the
//! shadow via a byte snapshot (`docs/04 §4.4.6`). Also provides
//! `replay_from_snapshot` (the reconnect path).
use sim::snapshot::{deserialize, serialize};
use sim::{checksum, step, ArenaState, Input};

/// Runs a client sim alongside an authoritative shadow, detecting and
/// correcting divergence at digest boundaries.
pub struct ShadowRunner {
    pub client: ArenaState,
    pub shadow: ArenaState,
    pub digest_interval: u32,
    /// Number of snapshot corrections applied so far.
    pub corrections: u32,
    /// Tick of the most recent correction, if any.
    pub last_correction_tick: Option<u32>,
}

impl ShadowRunner {
    /// Both sims start from the same seed/player (so they begin in sync).
    pub fn new(master_seed: u64, player_id: u32, digest_interval: u32) -> ShadowRunner {
        let client = ArenaState::new(master_seed, player_id);
        let shadow = ArenaState::new(master_seed, player_id);
        ShadowRunner {
            client,
            shadow,
            digest_interval,
            corrections: 0,
            last_correction_tick: None,
        }
    }

    /// Advance BOTH sims one tick with `input`. Then, if the new tick is on a
    /// digest boundary (`tick % digest_interval == 0`), compare
    /// `checksum(client)` vs `checksum(shadow)`; if they differ, correct the
    /// client by `deserialize(serialize(shadow))`, increment `corrections`, and
    /// set `last_correction_tick`.
    pub fn step(&mut self, input: Input) {
        step(&mut self.client, input);
        step(&mut self.shadow, input);
        if self.client.tick % self.digest_interval == 0
            && checksum(&self.client) != checksum(&self.shadow)
        {
            self.client = deserialize(&serialize(&self.shadow)).unwrap();
            self.corrections += 1;
            self.last_correction_tick = Some(self.client.tick);
        }
    }

    /// True iff client and shadow currently agree (by checksum).
    pub fn in_sync(&self) -> bool {
        checksum(&self.client) == checksum(&self.shadow)
    }
}

/// Reconnect path: rebuild an arena from `snapshot` bytes, then apply `inputs`
/// in order (the inputs from the snapshot tick up to the desired tick). Returns
/// the caught-up arena. Panics if the snapshot fails to deserialize.
pub fn replay_from_snapshot(snapshot: &[u8], inputs: &[Input]) -> ArenaState {
    let mut s = deserialize(snapshot).expect("snapshot failed to deserialize");
    for input in inputs {
        step(&mut s, *input);
    }
    s
}
