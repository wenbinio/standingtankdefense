//! Authoritative match director — AGENT M2A implements the `todo!()` bodies.
//!
//! Owns the clock, per-player shadow-sims, input ordering/acks, digest
//! validation + snapshot correction, and death/placement (`docs/03 §3.4/§3.9`).
//! Communicates ONLY via `Msg` over the transport.
//!
//! ## Per-tick contract (`tick(inbox) -> outbound`)
//! Maintain `iter: u32` (starts 0, increment at the END of each call) and, for
//! each player, a shadow `ArenaState`, an input `Schedule`, the next ack `seq`,
//! and a short history of `arena_tick -> shadow checksum` (for digest checks).
//! `arena_tick = iter.saturating_sub(START_LEAD)`.
//!
//! Each call, in order:
//! 1. While `iter < START_LEAD`: send `Msg::MatchStart { start_tick: START_LEAD,
//!    master_seed }` to every player (so it arrives before arenas start).
//! 2. Process `inbox`:
//!    - `Join` → (M2: just note it; no gating needed for the test).
//!    - `Input { seq, action }` from player P → `apply_tick = arena_tick +
//!      INPUT_LEAD_TICKS`; `schedule[P].set(apply_tick, action.to_input())`;
//!      reply `Msg::InputAck { seq, apply_tick }` to P.
//!    - `Digest { tick, checksum }` from P → look up `history[P][tick]`; if
//!      present and `!= checksum`, the client diverged: send
//!      `Msg::Snapshot { tick: arena_tick, bytes: serialize(shadow[P]) }` to P
//!      (a forward correction the client fast-forwards from).
//! 3. If `iter >= START_LEAD` and the match is live: for each ALIVE player, step
//!    their shadow one tick with `schedule[P].take(arena_tick)`; record
//!    `history[P][arena_tick] = checksum(shadow[P])` (keep ~256 ticks). If a
//!    shadow's `dead` just became true, assign the next placement from the back
//!    (last to die = best place) and broadcast `Msg::DeathConfirmed`. When all
//!    players are dead (or one remains, your call for M2) finalize placements and
//!    broadcast `Msg::MatchResult`.
//! 4. Every `BEACON_INTERVAL` ticks, broadcast `Msg::TimeBeacon { server_tick:
//!    arena_tick }`.
//! 5. Increment `iter`.
//!
//! Determinism: iterate players in sorted `PeerId` order; no floats/HashMap/wall
//! clock. Use `sim::{step, checksum}`, `sim::snapshot::{serialize}`,
//! `sim::ArenaState`.

use crate::transport::{Inbound, Outbound, PeerId};

pub struct Director {
    // AGENT M2A: add fields.
}

impl Director {
    /// Build a director for `players` (peers 1..=N) seeded with `master_seed`.
    /// Constructs a shadow `ArenaState::new(master_seed, player_id)` per player.
    pub fn new(players: &[PeerId], master_seed: u64) -> Director {
        let _ = (players, master_seed);
        todo!("AGENT M2A: Director::new")
    }

    /// Advance the authoritative clock one iteration: process `inbox`, step
    /// shadows, return all outbound messages.
    pub fn tick(&mut self, inbox: Vec<Inbound>) -> Vec<Outbound> {
        let _ = inbox;
        todo!("AGENT M2A: Director::tick")
    }

    /// Current authoritative arena tick (`iter - START_LEAD`, saturating).
    pub fn server_tick(&self) -> u32 {
        todo!("AGENT M2A: Director::server_tick")
    }

    /// Whether player `p`'s shadow is still alive.
    pub fn is_alive(&self, p: PeerId) -> bool {
        let _ = p;
        todo!("AGENT M2A: Director::is_alive")
    }

    /// Current checksum of player `p`'s shadow (for tests/leaderboard).
    pub fn shadow_checksum(&self, p: PeerId) -> Option<u64> {
        let _ = p;
        todo!("AGENT M2A: Director::shadow_checksum")
    }

    /// Final placements `(player, place)` once the match is resolved.
    pub fn result(&self) -> Option<Vec<(PeerId, u32)>> {
        todo!("AGENT M2A: Director::result")
    }
}
