//! Thin client (M2 + M3 reconnect). Runs the player's LOCAL arena, sends inputs,
//! reports digests, and applies authoritative corrections. Communicates ONLY via
//! `Msg` over the transport. Non-predictive: an input applies only at the
//! `apply_tick` the director returns, so a healthy client stays bit-identical to
//! its shadow.
//!
//! Two ways to enter a match:
//! - **Initial** (`Client::new`): constructed before the match with everyone
//!   else; learns the seed from the broadcast `MatchStart` and aligns its arena
//!   tick to `iter - START_LEAD` (all initial clients share `START_LEAD`, so they
//!   start stepping on the same global tick).
//! - **Reconnect** (`Client::reconnecting`): constructed MID-match with its own
//!   `iter` starting at 0. It sends `Join` until the director replies with a
//!   `Snapshot`; it adopts that authoritative state and sets `tick_base` so its
//!   arena tick tracks the snapshot's tick going forward. Because everything is
//!   indexed by ARENA tick (not wall-clock), running consistently "behind" the
//!   director is still bit-correct (`docs/03 §3.7`, `docs/04 §4.4.6`).

use crate::schedule::Schedule;
use crate::transport::{Channel, Inbound, Outbound, PeerId, DIRECTOR};
use crate::wire::{self, InputCode, Msg};
use crate::{DIGEST_INTERVAL, START_LEAD};
use sim::{ArenaState, Input};
use std::collections::BTreeMap;

pub struct Client {
    /// Driver iteration counter (starts 0, incremented at END of each `tick`).
    iter: u32,
    /// First iteration at which the arena steps. `START_LEAD` for an initial
    /// client (all initial clients share it, so they start on the same global
    /// tick); set to the next iteration when a reconnect snapshot is adopted.
    /// The arena's tick is read from `ArenaState::tick` directly — the only
    /// source of truth — so a reconnecting client (whose `iter` is far below the
    /// adopted tick) needs no iter↔tick arithmetic.
    step_gate: u32,
    /// Whether to actively send `Join` until started (reconnecting clients do).
    seek_join: bool,
    me: PeerId,
    #[allow(dead_code)]
    content_hash: u64,
    arena: Option<ArenaState>,
    schedule: Schedule,
    seq: u32,
    pending: BTreeMap<u32, Input>,
    corrections: u32,
    master_seed: Option<u64>,
}

impl Client {
    /// An INITIAL client (joins at match start via the `MatchStart` broadcast).
    pub fn new(me: PeerId, content_hash: u64) -> Client {
        Client::with_mode(me, content_hash, false)
    }

    /// A RECONNECTING client (joins mid-match; requests state via `Join`).
    pub fn reconnecting(me: PeerId, content_hash: u64) -> Client {
        Client::with_mode(me, content_hash, true)
    }

    fn with_mode(me: PeerId, content_hash: u64, seek_join: bool) -> Client {
        Client {
            iter: 0,
            step_gate: START_LEAD,
            seek_join,
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

    /// One driver iteration: process `inbox`, step the local arena, emit outbound.
    /// `desired` is the player's intended action THIS tick (`Noop` when idle).
    pub fn tick(&mut self, inbox: Vec<Inbound>, desired: Input) -> Vec<Outbound> {
        let mut out: Vec<Outbound> = Vec::new();

        // 1. Process inbox.
        for msg in inbox {
            let decoded = match wire::decode(&msg.bytes) {
                Ok(m) => m,
                Err(_) => continue,
            };
            match decoded {
                Msg::MatchStart { master_seed, .. } => {
                    self.master_seed = Some(master_seed);
                    if self.arena.is_none() {
                        // Initial alignment: arenas start at tick 0 on global tick
                        // START_LEAD (step_gate already START_LEAD).
                        self.arena = Some(ArenaState::new(master_seed, self.me.0));
                    }
                }
                Msg::InputAck { seq, apply_tick } => {
                    if let Some(action) = self.pending.remove(&seq) {
                        self.schedule.set(apply_tick, action);
                    }
                }
                Msg::Snapshot { bytes, .. } => {
                    if let Ok(restored) = sim::snapshot::deserialize(&bytes) {
                        if self.arena.is_none() {
                            // RECONNECT: adopt the authoritative state as-is and
                            // begin stepping NEXT iteration. We run consistently
                            // behind the director in wall-time, which is
                            // bit-correct because state is indexed by arena tick.
                            self.step_gate = self.iter + 1;
                            self.arena = Some(restored);
                        } else {
                            // IN-SYNC CORRECTION: fast-forward the authoritative
                            // state up to our current (pre-step) arena tick.
                            let target = self.arena.as_ref().unwrap().tick;
                            let mut a = restored;
                            while a.tick < target {
                                let action = self.schedule.take(a.tick);
                                sim::step(&mut a, action);
                            }
                            self.arena = Some(a);
                            self.corrections += 1;
                        }
                    }
                }
                Msg::TimeBeacon { .. } => {}
                _ => {}
            }
        }

        // 2. Step the local arena one tick once past the step gate. The pre-step
        //    arena tick keys the schedule and labels the digest.
        let mut stepped_tick: Option<u32> = None;
        if self.iter >= self.step_gate {
            if let Some(arena) = self.arena.as_mut() {
                let pre = arena.tick;
                let action = self.schedule.take(pre);
                sim::step(arena, action);
                stepped_tick = Some(pre);
            }
        }

        // 3. Reconnecting clients ask for state until they have an arena.
        if self.seek_join && self.arena.is_none() {
            out.push(Outbound {
                to: DIRECTOR,
                channel: Channel::Control,
                bytes: wire::encode(&Msg::Join { content_hash: self.content_hash }),
            });
        }

        // 4. Emit the player's desired action (applies later, on ack).
        if desired != Input::Noop && self.arena.is_some() {
            let seq = self.seq;
            out.push(Outbound {
                to: DIRECTOR,
                channel: Channel::Control,
                bytes: wire::encode(&Msg::Input {
                    seq,
                    action: InputCode::from_input(desired),
                }),
            });
            self.pending.insert(seq, desired);
            self.seq += 1;
        }

        // 5. Periodic liveness/drift digest, labelled by the pre-step tick (the
        //    same convention the director records history under).
        if let (Some(pre), Some(arena)) = (stepped_tick, self.arena.as_ref()) {
            if pre % DIGEST_INTERVAL == 0 {
                out.push(Outbound {
                    to: DIRECTOR,
                    channel: Channel::Telemetry,
                    bytes: wire::encode(&Msg::Digest {
                        tick: pre,
                        checksum: sim::checksum(arena),
                    }),
                });
            }
        }

        // 6. Advance the driver iteration.
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

    /// Number of in-sync snapshot corrections applied (NOT counting a reconnect
    /// adoption).
    pub fn corrections(&self) -> u32 {
        self.corrections
    }

    /// Whether the local arena exists (MatchStart received or reconnect adopted).
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

    fn from_director(msg: &Msg, channel: Channel) -> Inbound {
        Inbound { from: DIRECTOR, channel, bytes: wire::encode(msg) }
    }

    fn match_start() -> Inbound {
        from_director(&Msg::MatchStart { start_tick: 0, master_seed: SEED }, Channel::Control)
    }

    fn decoded(out: &[Outbound]) -> Vec<Msg> {
        out.iter().map(|o| wire::decode(&o.bytes).unwrap()).collect()
    }

    #[test]
    fn before_match_start_not_started_and_no_step() {
        let mut c = Client::new(me(), 0);
        for _ in 0..(START_LEAD + 5) {
            let out = c.tick(vec![], Input::Noop);
            // An initial client emits nothing until it has an arena.
            assert!(out.is_empty());
            assert!(!c.started());
            assert_eq!(c.arena_tick(), None);
        }
    }

    #[test]
    fn starts_and_steps_after_match_start() {
        let mut c = Client::new(me(), 0);
        c.tick(vec![match_start()], Input::Noop);
        assert!(c.started());
        for _ in 0..(START_LEAD + 10) {
            c.tick(vec![], Input::Noop);
        }
        assert!(c.arena_tick().unwrap() > 0);
    }

    #[test]
    fn emits_input_with_incrementing_seq_and_records_pending() {
        let mut c = Client::new(me(), 0);
        c.tick(vec![match_start()], Input::Noop);
        for _ in 0..START_LEAD {
            c.tick(vec![], Input::Noop);
        }
        let out = c.tick(vec![], Input::BuyOffer { slot: 1 });
        let inputs: Vec<&Msg> = {
            let d = decoded(&out);
            d.into_iter()
                .filter(|m| matches!(m, Msg::Input { .. }))
                .map(|m| Box::leak(Box::new(m)) as &Msg)
                .collect()
        };
        assert_eq!(inputs.len(), 1);
        if let Msg::Input { seq, action } = inputs[0] {
            assert_eq!(*seq, 0);
            assert_eq!(*action, InputCode::BuyOffer(1));
        }
    }

    #[test]
    fn ack_applies_action_at_apply_tick() {
        // Two clients: one receives the ack, one does not. They diverge only at
        // and after the apply tick.
        let mut a = Client::new(me(), 0);
        let mut b = Client::new(me(), 0);
        a.tick(vec![match_start()], Input::Noop);
        b.tick(vec![match_start()], Input::Noop);
        let apply = 20u32;
        let ack = from_director(&Msg::InputAck { seq: 0, apply_tick: apply }, Channel::Control);
        // 'a' learns of an action it 'sent' (seq 0) — fake the pending entry by
        // sending a desired first.
        for _ in 0..START_LEAD {
            a.tick(vec![], Input::Noop);
            b.tick(vec![], Input::Noop);
        }
        a.tick(vec![], Input::Clear); // seq 0 pending on 'a'
        b.tick(vec![], Input::Noop);
        a.tick(vec![ack], Input::Noop); // schedule Clear at tick 20
        b.tick(vec![], Input::Noop);
        // Step until both pass tick 20.
        let mut diverged_at = None;
        for _ in 0..40 {
            a.tick(vec![], Input::Noop);
            b.tick(vec![], Input::Noop);
            if a.arena_checksum() != b.arena_checksum() && diverged_at.is_none() {
                diverged_at = a.arena_tick();
            }
        }
        assert!(diverged_at.is_some(), "Clear never took effect");
        assert!(diverged_at.unwrap() > apply, "diverged before apply tick");
    }

    #[test]
    fn reconnecting_client_seeks_join_then_adopts_snapshot() {
        // A reconnecting client sends Join until it gets a Snapshot, then adopts.
        let mut c = Client::reconnecting(me(), 0xABCD);
        let out = c.tick(vec![], Input::Noop);
        assert!(
            decoded(&out).iter().any(|m| matches!(m, Msg::Join { .. })),
            "reconnecting client must send Join"
        );
        assert!(!c.started());

        // Build an authoritative state at tick 300 and hand it over.
        let mut auth = ArenaState::new(SEED, me().0);
        for _ in 0..300 {
            sim::step(&mut auth, Input::Noop);
        }
        let snap = from_director(
            &Msg::Snapshot { tick: 300, bytes: sim::snapshot::serialize(&auth) },
            Channel::Bulk,
        );
        c.tick(vec![snap], Input::Noop);
        assert!(c.started());
        assert_eq!(c.arena_tick(), Some(300));

        // It then advances in lockstep, staying on the canonical trajectory.
        for _ in 0..50 {
            c.tick(vec![], Input::Noop);
            sim::step(&mut auth, Input::Noop);
        }
        assert_eq!(c.arena_checksum(), Some(sim::checksum(&auth)));
        assert_eq!(c.corrections(), 0, "reconnect adoption is not a correction");
    }
}
