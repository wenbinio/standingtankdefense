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

use crate::schedule::Schedule;
use crate::transport::{Channel, Inbound, Outbound, PeerId, DIRECTOR};
use crate::wire::{self, InputCode, Msg};
use crate::{DIGEST_INTERVAL, START_LEAD};
use sim::{ArenaState, Input};
use std::collections::BTreeMap;

pub struct Client {
    /// Driver iteration counter (starts 0, incremented at END of each `tick`).
    iter: u32,
    /// This client's peer id; its `player_id` for the arena is `me.0`.
    me: PeerId,
    /// Content/version hash for the `Join` version-gate. The M2 contract does
    /// not emit `Join`, so this is retained for the handshake's future use.
    #[allow(dead_code)]
    content_hash: u64,
    /// The local, non-predictive arena. `None` until `MatchStart` arrives.
    arena: Option<ArenaState>,
    /// `arena_tick -> action` schedule shared in form with the director so an
    /// input applies on the SAME tick on both sides.
    schedule: Schedule,
    /// Next sequence number to stamp on an outgoing `Input`.
    seq: u32,
    /// `seq -> action` we sent, awaiting an `InputAck` to schedule it.
    pending: BTreeMap<u32, Input>,
    /// Count of `Snapshot` corrections applied.
    corrections: u32,
    /// The match master seed, once known via `MatchStart`.
    master_seed: Option<u64>,
}

impl Client {
    /// A client for player `me`; `content_hash` is sent in `Join` for the
    /// version/content gate.
    pub fn new(me: PeerId, content_hash: u64) -> Client {
        Client {
            iter: 0,
            me,
            content_hash,
            arena: None,
            schedule: Schedule::new(),
            seq: 0,
            pending: BTreeMap::new(),
            corrections: 0,
            master_seed: None,
        }
    }

    /// Arena tick derived from the driver iteration (`iter - START_LEAD`,
    /// saturating). Arenas step only once `iter >= START_LEAD`.
    fn arena_tick_for(&self) -> u32 {
        self.iter.saturating_sub(START_LEAD)
    }

    /// One driver iteration: process `inbox`, step the local arena, and emit
    /// outbound messages. `desired` is the player's intended action THIS tick
    /// (`Input::Noop` when idle).
    pub fn tick(&mut self, inbox: Vec<Inbound>, desired: Input) -> Vec<Outbound> {
        let arena_tick = self.arena_tick_for();
        let mut out: Vec<Outbound> = Vec::new();

        // 1. Process inbox.
        for msg in inbox {
            let decoded = match wire::decode(&msg.bytes) {
                Ok(m) => m,
                Err(_) => continue, // ignore malformed frames
            };
            match decoded {
                Msg::MatchStart { master_seed, .. } => {
                    self.master_seed = Some(master_seed);
                    if self.arena.is_none() {
                        self.arena = Some(ArenaState::new(master_seed, self.me.0));
                    }
                }
                Msg::InputAck { seq, apply_tick } => {
                    if let Some(action) = self.pending.remove(&seq) {
                        self.schedule.set(apply_tick, action);
                    }
                }
                Msg::Snapshot { bytes, .. } => {
                    if let Ok(mut a) = sim::snapshot::deserialize(&bytes) {
                        // Fast-forward the authoritative state up to the current
                        // arena tick using the SAME schedule so it stays
                        // consistent with un-corrected clients.
                        while a.tick < arena_tick {
                            let action = self.schedule.take(a.tick);
                            sim::step(&mut a, action);
                        }
                        self.arena = Some(a);
                        self.corrections += 1;
                    }
                }
                // Director-clock beacon: M2 derives arena tick from `iter`.
                Msg::TimeBeacon { .. } => {}
                // Other director→client messages are not handled in M2.
                _ => {}
            }
        }

        // 2. Step the local arena one tick once we're past START_LEAD.
        if self.iter >= START_LEAD {
            if let Some(arena) = self.arena.as_mut() {
                let action = self.schedule.take(arena_tick);
                sim::step(arena, action);
            }
        }

        // 3. Emit the player's desired action (applies later, on ack).
        if desired != Input::Noop {
            let seq = self.seq;
            let bytes = wire::encode(&Msg::Input {
                seq,
                action: InputCode::from_input(desired),
            });
            out.push(Outbound {
                to: DIRECTOR,
                channel: Channel::Control,
                bytes,
            });
            self.pending.insert(seq, desired);
            self.seq += 1;
        }

        // 4. Periodic liveness/drift digest.
        if self.arena.is_some() && arena_tick % DIGEST_INTERVAL == 0 {
            let checksum = sim::checksum(self.arena.as_ref().unwrap());
            let bytes = wire::encode(&Msg::Digest {
                tick: arena_tick,
                checksum,
            });
            out.push(Outbound {
                to: DIRECTOR,
                channel: Channel::Telemetry,
                bytes,
            });
        }

        // 5. Advance the driver iteration.
        self.iter += 1;
        out
    }

    /// Local arena tick, once the match has started.
    pub fn arena_tick(&self) -> Option<u32> {
        self.arena.as_ref().map(|a| a.tick)
    }

    /// Checksum of the local arena, once started.
    pub fn arena_checksum(&self) -> Option<u64> {
        self.arena.as_ref().map(sim::checksum)
    }

    /// Number of snapshot corrections applied so far.
    pub fn corrections(&self) -> u32 {
        self.corrections
    }

    /// Whether the local arena has been created (MatchStart received).
    pub fn started(&self) -> bool {
        self.arena.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Channel;
    use crate::wire::Msg;

    const SEED: u64 = 0xC0FFEE_1234;

    fn me() -> PeerId {
        PeerId(1)
    }

    /// Wrap an encoded director message as an `Inbound` from the director.
    fn from_director(msg: &Msg, channel: Channel) -> Inbound {
        Inbound {
            from: DIRECTOR,
            channel,
            bytes: wire::encode(msg),
        }
    }

    fn match_start() -> Inbound {
        from_director(
            &Msg::MatchStart { start_tick: 0, master_seed: SEED },
            Channel::Control,
        )
    }

    /// Decode the outbound frames a client emitted into `Msg`s.
    fn decoded(out: &[Outbound]) -> Vec<Msg> {
        out.iter().map(|o| wire::decode(&o.bytes).unwrap()).collect()
    }

    #[test]
    fn before_match_start_not_started_and_no_step() {
        let mut c = Client::new(me(), 0);
        // Drive several iterations past START_LEAD with no MatchStart.
        for _ in 0..(START_LEAD + 5) {
            let out = c.tick(vec![], Input::Noop);
            assert!(out.is_empty(), "no outbound before match start");
        }
        assert!(!c.started());
        assert_eq!(c.arena_tick(), None);
        assert_eq!(c.arena_checksum(), None);
        assert_eq!(c.corrections(), 0);
    }

    #[test]
    fn match_start_creates_arena_and_steps_after_start_lead() {
        let mut c = Client::new(me(), 0);
        // First tick delivers MatchStart at iter 0. Arena now exists and
        // arena_tick == 0, so a Digest (only) is emitted this iteration.
        let out = c.tick(vec![match_start()], Input::Noop);
        let msgs = decoded(&out);
        assert!(msgs.iter().all(|m| matches!(m, Msg::Digest { .. })));
        assert!(c.started());
        // At iter 0..START_LEAD the arena exists but does NOT step.
        assert_eq!(c.arena_tick(), Some(0));

        // Continue ticking; arena should not advance until iter >= START_LEAD.
        // We already consumed iter 0. Keep going.
        for _ in 1..START_LEAD {
            c.tick(vec![], Input::Noop);
            assert_eq!(c.arena_tick(), Some(0), "no step before START_LEAD");
        }
        // Now iter == START_LEAD: this tick steps arena 0 -> 1.
        c.tick(vec![], Input::Noop);
        assert_eq!(c.arena_tick(), Some(1));
        // And it keeps tracking arena_tick.
        c.tick(vec![], Input::Noop);
        assert_eq!(c.arena_tick(), Some(2));
    }

    #[test]
    fn started_flag_only_after_match_start() {
        let mut c = Client::new(me(), 0);
        assert!(!c.started());
        c.tick(vec![match_start()], Input::Noop);
        assert!(c.started());
    }

    #[test]
    fn desired_input_emits_incrementing_seq_and_records_pending() {
        let mut c = Client::new(me(), 0);
        c.tick(vec![match_start()], Input::Noop);

        let out = c.tick(vec![], Input::Reroll);
        let msgs: Vec<Msg> = decoded(&out)
            .into_iter()
            .filter(|m| matches!(m, Msg::Input { .. }))
            .collect();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            Msg::Input { seq, action } => {
                assert_eq!(*seq, 0);
                assert_eq!(*action, InputCode::Reroll);
            }
            _ => unreachable!(),
        }
        // Sent on the Control channel to the director.
        let input_out = out
            .iter()
            .find(|o| matches!(wire::decode(&o.bytes).unwrap(), Msg::Input { .. }))
            .unwrap();
        assert_eq!(input_out.to, DIRECTOR);
        assert_eq!(input_out.channel, Channel::Control);

        // Next non-Noop input increments seq.
        let out = c.tick(vec![], Input::BuyOffer { slot: 1 });
        let m = decoded(&out)
            .into_iter()
            .find(|m| matches!(m, Msg::Input { .. }))
            .unwrap();
        match m {
            Msg::Input { seq, action } => {
                assert_eq!(seq, 1);
                assert_eq!(action, InputCode::BuyOffer(1));
            }
            _ => unreachable!(),
        }
        // A Noop tick emits no Input.
        let out = c.tick(vec![], Input::Noop);
        assert!(decoded(&out).iter().all(|m| !matches!(m, Msg::Input { .. })));
    }

    #[test]
    fn ack_applies_action_at_exactly_apply_tick() {
        // Two clients: both receive the same MatchStart and the same desired
        // input on the same iteration (so the same seq is produced). Only `ack`
        // receives the InputAck. They must be identical UNTIL apply_tick, then
        // diverge at/after apply_tick.
        let mut ack = Client::new(me(), 0);
        let mut noack = Client::new(me(), 0);

        ack.tick(vec![match_start()], Input::Noop);
        noack.tick(vec![match_start()], Input::Noop);

        // Both send a Reroll on iter 1 → seq 0.
        let out = ack.tick(vec![], Input::Reroll);
        noack.tick(vec![], Input::Reroll);
        // Confirm the seq the director would ack.
        let seq = match decoded(&out)
            .into_iter()
            .find(|m| matches!(m, Msg::Input { .. }))
            .unwrap()
        {
            Msg::Input { seq, .. } => seq,
            _ => unreachable!(),
        };
        assert_eq!(seq, 0);

        // Choose an apply tick comfortably in the future.
        let apply_tick: u32 = 20;
        // Deliver the ack to `ack` only, on the next iteration.
        ack.tick(
            vec![from_director(
                &Msg::InputAck { seq, apply_tick },
                Channel::Control,
            )],
            Input::Noop,
        );
        noack.tick(vec![], Input::Noop);

        // Drive both forward. They should stay identical until the apply_tick,
        // then diverge once the scheduled Reroll is consumed.
        let mut diverged_at: Option<u32> = None;
        while ack.arena_tick().unwrap() < apply_tick + 3 {
            ack.tick(vec![], Input::Noop);
            noack.tick(vec![], Input::Noop);
            let at = ack.arena_tick().unwrap();
            if ack.arena_checksum() != noack.arena_checksum() && diverged_at.is_none() {
                diverged_at = Some(at);
            }
        }
        // Reroll changes shop RNG state, so the checksum must diverge, and only
        // at/after the apply tick.
        let d = diverged_at.expect("checksums must diverge after the ack applies");
        assert!(
            d > apply_tick,
            "divergence first observed at tick {d}; the step that consumed apply_tick={apply_tick} \
             produces a state whose tick is apply_tick+1"
        );
    }

    #[test]
    fn digest_emitted_every_interval() {
        let mut c = Client::new(me(), 0);
        c.tick(vec![match_start()], Input::Noop);
        // After the first tick (iter 0, arena_tick 0) a digest at tick 0 was
        // emitted. Now collect the digest ticks across a long run.
        let mut digest_ticks: Vec<u32> = Vec::new();
        for _ in 0..(START_LEAD + DIGEST_INTERVAL * 3) {
            let out = c.tick(vec![], Input::Noop);
            for m in decoded(&out) {
                if let Msg::Digest { tick, .. } = m {
                    digest_ticks.push(tick);
                }
            }
        }
        // Every recorded digest tick must be a multiple of DIGEST_INTERVAL.
        assert!(digest_ticks.iter().all(|t| t % DIGEST_INTERVAL == 0));
        // We must have seen at least DIGEST_INTERVAL and 2*DIGEST_INTERVAL.
        assert!(digest_ticks.contains(&DIGEST_INTERVAL));
        assert!(digest_ticks.contains(&(DIGEST_INTERVAL * 2)));
    }

    #[test]
    fn digest_uses_telemetry_channel() {
        let mut c = Client::new(me(), 0);
        let out = c.tick(vec![match_start()], Input::Noop);
        // arena_tick 0 → digest emitted on the Telemetry channel.
        let d = out
            .iter()
            .find(|o| matches!(wire::decode(&o.bytes).unwrap(), Msg::Digest { .. }))
            .expect("a digest at arena_tick 0");
        assert_eq!(d.to, DIRECTOR);
        assert_eq!(d.channel, Channel::Telemetry);
    }

    #[test]
    fn snapshot_replaces_state_fast_forwards_and_bumps_corrections() {
        // Build a "reference" client advanced to some arena tick, capture its
        // authoritative serialized state at that tick, then feed that snapshot
        // to a fresh client mid-run and confirm it fast-forwards to the same
        // arena tick (matching a control client that never got the snapshot).
        let mut reference = Client::new(me(), 0);
        reference.tick(vec![match_start()], Input::Noop);
        // Advance reference to arena_tick == SNAP_TICK.
        const SNAP_TICK: u32 = 5;
        while reference.arena_tick().unwrap() < SNAP_TICK {
            reference.tick(vec![], Input::Noop);
        }
        // Serialize its authoritative state at SNAP_TICK.
        // (Re-create the underlying arena state via the public arena accessors:
        //  we step a fresh ArenaState identically to obtain serializable bytes.)
        let mut authoritative = ArenaState::new(SEED, me().0);
        while authoritative.tick < SNAP_TICK {
            sim::step(&mut authoritative, Input::Noop);
        }
        let snap_bytes = sim::snapshot::serialize(&authoritative);
        assert_eq!(authoritative.tick, SNAP_TICK);

        // A "control" client that progresses normally with no snapshot.
        let mut control = Client::new(me(), 0);
        control.tick(vec![match_start()], Input::Noop);

        // The client under test: same path, but receives the snapshot at a
        // point where its arena_tick is already past SNAP_TICK so a
        // fast-forward is exercised.
        let mut victim = Client::new(me(), 0);
        victim.tick(vec![match_start()], Input::Noop);

        // Advance control and victim in lockstep to arena_tick TARGET-1, then on
        // the next tick feed victim the (stale) snapshot.
        const TARGET: u32 = 12;
        while control.arena_tick().unwrap() < TARGET - 1 {
            control.tick(vec![], Input::Noop);
            victim.tick(vec![], Input::Noop);
        }
        assert_eq!(victim.corrections(), 0);

        // Deliver the snapshot on the Bulk channel on the next tick. The client
        // deserializes (arena.tick == SNAP_TICK), fast-forwards to the current
        // arena_tick, and bumps corrections.
        let snap = from_director(
            &Msg::Snapshot { tick: SNAP_TICK, bytes: snap_bytes },
            Channel::Bulk,
        );
        control.tick(vec![], Input::Noop);
        victim.tick(vec![snap], Input::Noop);

        assert_eq!(victim.corrections(), 1);
        // After fast-forward + this iteration's normal step, victim must match
        // the control client tick-for-tick and checksum-for-checksum.
        assert_eq!(victim.arena_tick(), control.arena_tick());
        assert_eq!(victim.arena_checksum(), control.arena_checksum());
    }
}
