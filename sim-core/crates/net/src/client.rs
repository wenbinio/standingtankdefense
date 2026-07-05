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
use crate::wire::{self, GameSpeed, InputCode, Msg};
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
    /// Host-set match pace, learned from the authoritative `MatchStart`.
    /// Pure CADENCE (`docs/04 §4.4.1`): the client still steps exactly one sim
    /// tick per `tick()` call — the embedding DRIVER calls it
    /// `game_speed.ticks_per_second()` times per wall-clock second. Because
    /// the server-tick estimate free-runs +1 per STEPPED tick (not per
    /// wall-clock second), the clock-sync expected-rate is speed-correct by
    /// construction: at any speed both sides advance one tick per iteration,
    /// so beacons and the estimate stay in the same units. `None` until
    /// `MatchStart` arrives (a RECONNECTING client adopts a `Snapshot` and
    /// never sees `MatchStart`; it learns the speed out-of-band from the
    /// lobby data, like `content_hash`).
    game_speed: Option<GameSpeed>,
    /// Iteration of the last `Join` sent, to rate-limit retries so we don't
    /// trigger redundant snapshots while one is already in flight.
    last_join_iter: Option<u32>,
    /// Self-imposed purchase challenge, mirrored from the director so this
    /// client's shadow filters buys identically and stays in lockstep.
    challenge: sim::bot::Challenge,
    /// Client-side estimate of the AUTHORITATIVE server tick (`docs/03 §3.7`).
    /// Distinct from the local arena tick: a reconnecting client runs its arena
    /// consistently behind the director, yet still needs an honest read of server
    /// time for round/boss-timer UI and to notice if its local clock has drifted.
    /// Free-runs (+1 per stepped tick) between beacons and re-anchors on each
    /// `TimeBeacon`. `None` until the first beacon arrives (or the arena starts).
    server_tick_est: Option<u32>,
    /// Last measured drift `estimate - beacon` at the most recent `TimeBeacon`.
    /// Positive ⇒ our estimate ran ahead of the server; negative ⇒ behind.
    clock_drift: i32,
    /// Set when a beacon revealed drift past `DRIFT_SNAP_THRESHOLD` and we hard-
    /// snapped the estimate. A UI/resync layer can read+clear this to request an
    /// authoritative `Snapshot`; it never touches arena/input/snapshot logic here.
    clock_resync_flagged: bool,
}

/// Resend `Join` at most this often (in iterations) while awaiting state.
const JOIN_RETRY_TICKS: u32 = 30;

/// Hard cap on un-acked `pending` inputs. Without a bound, lost `InputAck`s
/// leak entries forever (the map is only drained by acks). A human emits at
/// most a few inputs per second, so 64 outstanding actions is already
/// pathological; beyond it the OLDEST entries are stalest and are dropped
/// first (their acks are overwhelmingly likely lost — and if one does arrive
/// late, the missed schedule slot is healed by the digest→snapshot
/// correction path, never by trusting the client).
const PENDING_CAP: usize = 64;

/// Drift magnitude (ticks) beyond which the server-time estimate is HARD-SNAPPED
/// to the beacon (and `clock_resync_flagged` is set) instead of eased. Sized
/// above normal jitter-induced wobble (a beacon's apparent age varies by the
/// link's one-way delay spread) so ordinary jitter eases, but a genuine clock
/// divergence snaps. Independent of sim determinism — pure UI/clock bookkeeping.
const DRIFT_SNAP_THRESHOLD: i32 = 8;

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
            game_speed: None,
            last_join_iter: None,
            challenge: sim::bot::Challenge::None,
            server_tick_est: None,
            clock_drift: 0,
            clock_resync_flagged: false,
        }
    }

    /// Mirror the director's challenge for this client so buy-filtering matches.
    pub fn set_challenge(&mut self, c: sim::bot::Challenge) {
        self.challenge = c;
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
                Msg::MatchStart {
                    master_seed,
                    game_speed,
                    ..
                } => {
                    self.master_seed = Some(master_seed);
                    self.game_speed = Some(game_speed);
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
                    // Acks arrive in order on the Control channel, so anything
                    // still pending at or below the acked seq is an orphan
                    // whose ack was lost; drop it (bounds `pending` under ack
                    // loss). A missed apply is healed by the correction path.
                    self.pending = self.pending.split_off(&seq.saturating_add(1));
                }
                Msg::Snapshot { bytes, .. } => {
                    if let Ok(restored) = sim::snapshot::deserialize(&bytes) {
                        if self.arena.is_none() {
                            // RECONNECT: adopt the authoritative state as-is and
                            // begin stepping NEXT iteration. We run consistently
                            // behind the director in wall-time, which is
                            // bit-correct because state is indexed by arena tick.
                            self.step_gate = self.iter + 1;
                            // Anything scheduled before the adopted tick can
                            // never be consumed by `take` again — drop it.
                            self.schedule.discard_before(restored.tick);
                            self.arena = Some(restored);
                        } else {
                            // IN-SYNC CORRECTION: fast-forward the authoritative
                            // state up to our current (pre-step) arena tick.
                            let current = self.arena.as_ref().unwrap();
                            let target = current.tick;
                            let before = sim::checksum(current);
                            let mut a = restored;
                            while a.tick < target {
                                let action = self.challenge.filter(self.schedule.take(a.tick), &a);
                                sim::step(&mut a, action);
                            }
                            // Only count a correction that actually changed state.
                            // A redundant snapshot (e.g. a second reply to a
                            // duplicate Join) fast-forwards to the same state and
                            // is a harmless no-op, not a real divergence fix.
                            let changed = sim::checksum(&a) != before;
                            self.arena = Some(a);
                            if changed {
                                self.corrections += 1;
                            }
                            // Housekeeping: schedule entries below the
                            // corrected tick were consumed by the fast-forward
                            // (or are unreachable) — discard them so the map
                            // cannot accumulate stale slots across corrections.
                            self.schedule.discard_before(target);
                        }
                    }
                }
                Msg::TimeBeacon { server_tick } => {
                    // Re-anchor the server-time estimate. A beacon describes the
                    // server tick AT SEND; by the time it arrives the server has
                    // moved on by the one-way delay, so a healthy estimate runs a
                    // little AHEAD of the beacon value — small positive drift is
                    // normal and is eased, not snapped.
                    match self.server_tick_est {
                        None => {
                            // First fix: adopt the beacon outright.
                            self.server_tick_est = Some(server_tick);
                            self.clock_drift = 0;
                        }
                        Some(est) => {
                            let drift = est as i32 - server_tick as i32;
                            self.clock_drift = drift;
                            if drift.abs() > DRIFT_SNAP_THRESHOLD {
                                // Genuine divergence: hard-snap and flag a resync.
                                self.server_tick_est = Some(server_tick);
                                self.clock_resync_flagged = true;
                            } else if drift != 0 {
                                // Ease one tick toward the beacon so the estimate
                                // converges smoothly without a visible UI jump.
                                let adj = if drift > 0 { -1 } else { 1 };
                                self.server_tick_est = Some((est as i32 + adj) as u32);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // 2. Step the local arena one tick once past the step gate. The pre-step
        //    arena tick keys the schedule and labels the digest.
        let mut stepped_tick: Option<u32> = None;
        if self.iter >= self.step_gate {
            if let Some(arena) = self.arena.as_mut() {
                let pre = arena.tick;
                let action = self.challenge.filter(self.schedule.take(pre), arena);
                sim::step(arena, action);
                stepped_tick = Some(pre);
                // Free-run the server-time estimate one tick per stepped tick.
                // It re-anchors on each TimeBeacon (above); between beacons it just
                // advances at the same 30 Hz cadence the server does.
                if let Some(est) = self.server_tick_est.as_mut() {
                    *est += 1;
                }
            }
        }

        // 3. Reconnecting clients ask for state until they have an arena,
        //    rate-limited so we don't draw redundant snapshots while one is in
        //    flight (a second, later snapshot would shove a behind-client forward).
        if self.seek_join && self.arena.is_none() {
            let due = self
                .last_join_iter
                .is_none_or(|last| self.iter - last >= JOIN_RETRY_TICKS);
            if due {
                out.push(Outbound {
                    to: DIRECTOR,
                    channel: Channel::Control,
                    bytes: wire::encode(&Msg::Join {
                        content_hash: self.content_hash,
                    }),
                });
                self.last_join_iter = Some(self.iter);
            }
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
            // Bound `pending` even if every ack is lost: evict the oldest
            // (stalest) entries first.
            while self.pending.len() > PENDING_CAP {
                self.pending.pop_first();
            }
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

    /// Client-side estimate of the authoritative SERVER tick — for round/boss
    /// timer UI. `None` until the first `TimeBeacon` re-anchors it. This is NOT
    /// the local arena tick (see [`Client::arena_tick`]); a reconnecting client
    /// runs its arena behind the server yet still tracks true server time here.
    pub fn server_tick(&self) -> Option<u32> {
        self.server_tick_est
    }

    /// Drift `estimate - beacon` measured at the most recent `TimeBeacon`
    /// (positive ⇒ estimate ahead of the server). 0 before any beacon.
    pub fn clock_drift(&self) -> i32 {
        self.clock_drift
    }

    /// The host-set match pace, once `MatchStart` delivered it. `None` before
    /// the match starts (and for a reconnecting client, which learns it from
    /// the lobby data instead).
    pub fn game_speed(&self) -> Option<GameSpeed> {
        self.game_speed
    }

    /// Driver cadence: how many times per wall-clock second the embedding
    /// driver must call [`Client::tick`] (30/45/60/90 for
    /// Normal/Fast/Faster/Hyper). `None` until the speed is known.
    pub fn ticks_per_second(&self) -> Option<u32> {
        self.game_speed.map(GameSpeed::ticks_per_second)
    }

    /// Take (read+clear) the "clock resync needed" flag, set when a beacon showed
    /// drift past the snap threshold and the estimate was hard-snapped. A UI/net
    /// layer can poll this to request a fresh `Snapshot`; it does not affect the
    /// deterministic arena/input/snapshot paths.
    pub fn take_clock_resync(&mut self) -> bool {
        std::mem::take(&mut self.clock_resync_flagged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Channel;
    use crate::wire::Msg;

    const SEED: u64 = 0x00C0_FFEE_1234;

    fn me() -> PeerId {
        PeerId(1)
    }

    fn from_director(msg: &Msg, channel: Channel) -> Inbound {
        Inbound {
            from: DIRECTOR,
            channel,
            bytes: wire::encode(msg),
        }
    }

    fn match_start() -> Inbound {
        from_director(
            &Msg::MatchStart {
                start_tick: 0,
                master_seed: SEED,
                game_speed: GameSpeed::Normal,
            },
            Channel::Control,
        )
    }

    fn decoded(out: &[Outbound]) -> Vec<Msg> {
        out.iter()
            .map(|o| wire::decode(&o.bytes).unwrap())
            .collect()
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
        let ack = from_director(
            &Msg::InputAck {
                seq: 0,
                apply_tick: apply,
            },
            Channel::Control,
        );
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
    fn pending_is_bounded_under_total_ack_loss() {
        // Every InputAck is lost: `pending` must still never exceed the cap,
        // and the oldest (stalest) seqs must be the ones evicted.
        let mut c = Client::new(me(), 0);
        c.tick(vec![match_start()], Input::Noop);
        for _ in 0..START_LEAD {
            c.tick(vec![], Input::Noop);
        }
        let n = (PENDING_CAP as u32) * 3;
        for _ in 0..n {
            c.tick(vec![], Input::Reroll); // emitted, never acked
            assert!(c.pending.len() <= PENDING_CAP, "pending grew past the cap");
        }
        assert_eq!(c.pending.len(), PENDING_CAP);
        // Oldest evicted, newest kept.
        let oldest_kept = *c.pending.keys().next().unwrap();
        assert_eq!(oldest_kept, n - PENDING_CAP as u32);
    }

    #[test]
    fn ack_prunes_orphaned_older_pending() {
        // Acks 0 and 1 are lost; ack 2 arrives. Scheduling seq 2 must also
        // drop the orphaned seqs 0 and 1 (their acks can no longer be pending
        // on the ordered Control channel).
        let mut c = Client::new(me(), 0);
        c.tick(vec![match_start()], Input::Noop);
        for _ in 0..START_LEAD {
            c.tick(vec![], Input::Noop);
        }
        c.tick(vec![], Input::Reroll); // seq 0
        c.tick(vec![], Input::Reroll); // seq 1
        c.tick(vec![], Input::Clear); // seq 2
        assert_eq!(c.pending.len(), 3);
        let ack = from_director(
            &Msg::InputAck {
                seq: 2,
                apply_tick: 60,
            },
            Channel::Control,
        );
        c.tick(vec![ack], Input::Noop);
        assert!(
            c.pending.is_empty(),
            "acked + orphaned entries must be gone"
        );
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
            &Msg::Snapshot {
                tick: 300,
                bytes: sim::snapshot::serialize(&auth),
            },
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
