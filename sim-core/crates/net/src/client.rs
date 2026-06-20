//! Thin client — AGENT M2B implements the `todo!()` bodies.
//!
//! Runs the player's LOCAL arena, sends inputs, reports digests, and applies
//! authoritative corrections. Communicates ONLY via `Msg` over the transport.
//! Non-predictive: an input is applied only at the `apply_tick` the director
//! returns, so a healthy client stays bit-identical to its shadow.
//!
//! ## Per-tick contract (`tick(inbox, desired) -> outbound`)
//! Maintain `iter: u32` (starts 0, increment at END), `me: PeerId`, optional
//! `arena: ArenaState`, an input `Schedule`, next `seq`, `corrections: u32`, and
//! the `master_seed` once known. `arena_tick = iter.saturating_sub(START_LEAD)`.
//!
//! Each call, in order:
//! 1. Process `inbox`:
//!    - `MatchStart { master_seed, .. }` → store seed; if no arena yet, create
//!      `ArenaState::new(master_seed, me.0)`.
//!    - `InputAck { seq, apply_tick }` → `schedule.set(apply_tick, pending[seq])`
//!      (the action you sent with that seq).
//!    - `Snapshot { tick, bytes }` → `deserialize` into the arena (this is the
//!      authoritative state at `tick`). Then FAST-FORWARD: while
//!      `arena.tick < arena_tick`, `sim::step(arena, schedule.take(arena.tick))`.
//!      Increment `corrections`. (`docs/04 §4.4.6`.)
//!    - `TimeBeacon` → may ignore in M2 (arena tick is `iter`-derived).
//! 2. If `arena` exists and `iter >= START_LEAD`: step the arena one tick with
//!    `schedule.take(arena_tick)`  (so `arena.tick` tracks `arena_tick`).
//! 3. If `desired != Input::Noop`: send `Msg::Input { seq, action:
//!    InputCode::from_input(desired) }` to `DIRECTOR`; remember `pending[seq] =
//!    desired`; `seq += 1`. (The player's intent; it applies later, on ack.)
//! 4. If `arena` exists and `arena_tick % DIGEST_INTERVAL == 0`: send
//!    `Msg::Digest { tick: arena_tick, checksum: checksum(arena) }` to `DIRECTOR`.
//! 5. Increment `iter`.
//!
//! Channels: `Input` on Control, `Digest` on Telemetry. Determinism: no
//! floats/HashMap/wall clock. Use `sim::{ArenaState, Input, step, checksum}`,
//! `sim::snapshot::deserialize`.

use crate::transport::{Inbound, Outbound, PeerId};
use sim::Input;

pub struct Client {
    // AGENT M2B: add fields.
}

impl Client {
    /// A client for player `me`; `content_hash` is sent in `Join` for the
    /// version/content gate.
    pub fn new(me: PeerId, content_hash: u64) -> Client {
        let _ = (me, content_hash);
        todo!("AGENT M2B: Client::new")
    }

    /// One driver iteration: process `inbox`, step the local arena, and emit
    /// outbound messages. `desired` is the player's intended action THIS tick
    /// (`Input::Noop` when idle).
    pub fn tick(&mut self, inbox: Vec<Inbound>, desired: Input) -> Vec<Outbound> {
        let _ = (inbox, desired);
        todo!("AGENT M2B: Client::tick")
    }

    /// Local arena tick, once the match has started.
    pub fn arena_tick(&self) -> Option<u32> {
        todo!("AGENT M2B: Client::arena_tick")
    }

    /// Checksum of the local arena, once started.
    pub fn arena_checksum(&self) -> Option<u64> {
        todo!("AGENT M2B: Client::arena_checksum")
    }

    /// Number of snapshot corrections applied so far.
    pub fn corrections(&self) -> u32 {
        todo!("AGENT M2B: Client::corrections")
    }

    /// Whether the local arena has been created (MatchStart received).
    pub fn started(&self) -> bool {
        todo!("AGENT M2B: Client::started")
    }
}
