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

use crate::transport::{Channel, Inbound, Outbound, PeerId};
use crate::wire::{self, Msg};
use crate::{BEACON_INTERVAL, INPUT_LEAD_TICKS, START_LEAD};
use crate::Schedule;
use sim::ArenaState;
use std::collections::BTreeMap;

/// How many `arena_tick -> checksum` entries to retain per player.
const HISTORY_LEN: usize = 256;

/// Per-player authoritative state held by the director.
struct Player {
    /// Validating shadow-sim for this player.
    shadow: ArenaState,
    /// Input schedule (`arena_tick -> action`), kept in lockstep with the shadow.
    schedule: Schedule,
    /// Recent `arena_tick -> shadow checksum`, trimmed to `HISTORY_LEN`.
    history: BTreeMap<u32, u64>,
    /// Whether this shadow has already been recorded as dead (so we only emit
    /// `DeathConfirmed` once).
    death_recorded: bool,
    /// Optional self-imposed challenge filtering this player's purchases
    /// (cosmetic preview aid). Applied at apply-time on both director and
    /// client identically, so shadows stay in lockstep.
    challenge: sim::bot::Challenge,
}

pub struct Director {
    /// Driver iteration counter; arena tick = `iter - START_LEAD` (saturating).
    iter: u32,
    /// Match seed broadcast in `MatchStart` and used to build shadows.
    master_seed: u64,
    /// Players in sorted `PeerId` order (the only iteration order used).
    peers: Vec<PeerId>,
    /// Per-player state, index-aligned with `peers`.
    players: Vec<Player>,
    /// Death order: peers in the order they died (first to die at front).
    death_order: Vec<PeerId>,
    /// Final standings, set once the match resolves.
    result: Option<Vec<(PeerId, u32)>>,
}

impl Director {
    /// Build a director for `players` (peers 1..=N) seeded with `master_seed`.
    /// Constructs a shadow `ArenaState::new(master_seed, player_id)` per player.
    pub fn new(players: &[PeerId], master_seed: u64) -> Director {
        let mut peers: Vec<PeerId> = players.to_vec();
        peers.sort();
        peers.dedup();
        let players = peers
            .iter()
            .map(|p| Player {
                shadow: ArenaState::new(master_seed, p.0),
                schedule: Schedule::new(),
                history: BTreeMap::new(),
                death_recorded: false,
                challenge: sim::bot::Challenge::None,
            })
            .collect();
        Director {
            iter: 0,
            master_seed,
            peers,
            players,
            death_order: Vec::new(),
            result: None,
        }
    }

    /// Constrain player `i`'s purchases to a challenge (cosmetic preview aid).
    /// Must mirror the matching client's challenge so their shadows agree.
    pub fn set_challenge(&mut self, i: usize, c: sim::bot::Challenge) {
        if let Some(p) = self.players.get_mut(i) {
            p.challenge = c;
        }
    }

    /// Index of `p` within the sorted `peers`, if present.
    fn index_of(&self, p: PeerId) -> Option<usize> {
        self.peers.binary_search(&p).ok()
    }

    /// Advance the authoritative clock one iteration: process `inbox`, step
    /// shadows, return all outbound messages.
    pub fn tick(&mut self, inbox: Vec<Inbound>) -> Vec<Outbound> {
        let mut out: Vec<Outbound> = Vec::new();
        let arena_tick = self.iter.saturating_sub(START_LEAD);

        // 1. Pre-start: broadcast MatchStart so every arena starts on the same tick.
        if self.iter < START_LEAD {
            for p in &self.peers {
                out.push(send(
                    *p,
                    Channel::Control,
                    &Msg::MatchStart {
                        start_tick: START_LEAD,
                        master_seed: self.master_seed,
                    },
                ));
            }
        }

        // 2. Process inbox.
        for inb in inbox {
            let msg = match wire::decode(&inb.bytes) {
                Ok(m) => m,
                Err(_) => continue, // ignore undecodable traffic
            };
            let from = inb.from;
            match msg {
                Msg::Join { .. } => {
                    // (Re)join: hand the peer what it needs to start its arena.
                    // Pre-start → the MatchStart broadcast covers it; live →
                    // send an authoritative Snapshot of its shadow so a
                    // reconnecting client can adopt and catch up (docs/03 §3.7).
                    if self.iter >= START_LEAD {
                        if let Some(i) = self.index_of(from) {
                            let bytes = sim::snapshot::serialize(&self.players[i].shadow);
                            out.push(send(
                                from,
                                Channel::Bulk,
                                &Msg::Snapshot { tick: arena_tick, bytes },
                            ));
                        }
                    } else {
                        out.push(send(
                            from,
                            Channel::Control,
                            &Msg::MatchStart {
                                start_tick: START_LEAD,
                                master_seed: self.master_seed,
                            },
                        ));
                    }
                }
                Msg::Input { seq, action } => {
                    let apply_tick = arena_tick + INPUT_LEAD_TICKS;
                    if let Some(i) = self.index_of(from) {
                        self.players[i]
                            .schedule
                            .set(apply_tick, action.to_input());
                    }
                    out.push(send(
                        from,
                        Channel::Control,
                        &Msg::InputAck { seq, apply_tick },
                    ));
                }
                Msg::Digest { tick, checksum } => {
                    if let Some(i) = self.index_of(from) {
                        if let Some(&recorded) = self.players[i].history.get(&tick) {
                            if recorded != checksum {
                                let bytes = sim::snapshot::serialize(&self.players[i].shadow);
                                out.push(send(
                                    from,
                                    Channel::Bulk,
                                    &Msg::Snapshot {
                                        tick: arena_tick,
                                        bytes,
                                    },
                                ));
                            }
                        }
                    }
                }
                // Director→client messages are never expected inbound; ignore.
                _ => {}
            }
        }

        // 3. Step shadows (only once arenas are live and the match is unresolved).
        if self.iter >= START_LEAD && self.result.is_none() {
            // Collect newly-dead players this tick, in sorted peer order.
            let mut newly_dead: Vec<(PeerId, u32)> = Vec::new();

            for i in 0..self.peers.len() {
                let was_dead = self.players[i].death_recorded;
                if !was_dead {
                    let raw = self.players[i].schedule.take(arena_tick);
                    let inp = self.players[i].challenge.filter(raw, &self.players[i].shadow);
                    sim::step(&mut self.players[i].shadow, inp);
                    let cs = sim::checksum(&self.players[i].shadow);
                    self.players[i].history.insert(arena_tick, cs);
                    // Trim history to the most recent HISTORY_LEN entries.
                    while self.players[i].history.len() > HISTORY_LEN {
                        let oldest = *self.players[i].history.keys().next().unwrap();
                        self.players[i].history.remove(&oldest);
                    }
                    if self.players[i].shadow.dead {
                        let died_tick = self.players[i]
                            .shadow
                            .death_tick
                            .unwrap_or(arena_tick);
                        self.players[i].death_recorded = true;
                        newly_dead.push((self.peers[i], died_tick));
                    }
                }
            }

            // Assign placements from the back: last to die gets the best place.
            // place = total_players - (number that have died so far, counting
            // this one). Process deaths in sorted peer order for determinism.
            let total = self.peers.len() as u32;
            for (peer, died_tick) in &newly_dead {
                self.death_order.push(*peer);
                let died_so_far = self.death_order.len() as u32;
                let place = total - died_so_far + 1;
                broadcast(
                    &mut out,
                    &self.peers,
                    Channel::Control,
                    &Msg::DeathConfirmed {
                        player: peer.0,
                        died_tick: *died_tick,
                        place,
                    },
                );
            }

            // Finalize when every player is dead.
            if self.death_order.len() == self.peers.len() && !self.peers.is_empty() {
                // death_order: first entry died first (worst place). place is
                // computed the same way: index 0 -> place N, last -> place 1.
                let total = self.peers.len() as u32;
                let mut places: Vec<(PeerId, u32)> = Vec::with_capacity(self.peers.len());
                for (idx, peer) in self.death_order.iter().enumerate() {
                    let place = total - idx as u32;
                    places.push((*peer, place));
                }
                // Sort the published standings by peer for a stable, deterministic
                // ordering of the (player, place) pairs.
                places.sort_by_key(|(p, _)| p.0);
                self.result = Some(places.clone());
                let wire_places: Vec<(u32, u32)> =
                    places.iter().map(|(p, pl)| (p.0, *pl)).collect();
                broadcast(
                    &mut out,
                    &self.peers,
                    Channel::Control,
                    &Msg::MatchResult {
                        places: wire_places,
                    },
                );
            }
        }

        // 4. Time beacon on the interval.
        if self.iter >= START_LEAD && arena_tick % BEACON_INTERVAL == 0 {
            broadcast(
                &mut out,
                &self.peers,
                Channel::Telemetry,
                &Msg::TimeBeacon {
                    server_tick: arena_tick,
                },
            );
        }

        // 5. Advance the clock.
        self.iter += 1;
        out
    }

    /// Current authoritative arena tick (`iter - START_LEAD`, saturating).
    pub fn server_tick(&self) -> u32 {
        self.iter.saturating_sub(START_LEAD)
    }

    /// Whether player `p`'s shadow is still alive.
    pub fn is_alive(&self, p: PeerId) -> bool {
        match self.index_of(p) {
            Some(i) => !self.players[i].shadow.dead,
            None => false,
        }
    }

    /// Read-only access to player `p`'s authoritative shadow arena — for
    /// spectator/lobby rendering (e.g. the multi-arena view). Never mutated.
    pub fn shadow(&self, p: PeerId) -> Option<&ArenaState> {
        self.index_of(p).map(|i| &self.players[i].shadow)
    }

    /// The match's peers, in the sorted order the director iterates them.
    pub fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    /// Current checksum of player `p`'s shadow (for tests/leaderboard).
    pub fn shadow_checksum(&self, p: PeerId) -> Option<u64> {
        self.index_of(p)
            .map(|i| sim::checksum(&self.players[i].shadow))
    }

    /// Final placements `(player, place)` once the match is resolved.
    pub fn result(&self) -> Option<Vec<(PeerId, u32)>> {
        self.result.clone()
    }
}

/// Build a single addressed `Outbound`.
fn send(to: PeerId, channel: Channel, msg: &Msg) -> Outbound {
    Outbound {
        to,
        channel,
        bytes: wire::encode(msg),
    }
}

/// Fan out `msg` to every peer on `channel` (the transport addresses one
/// recipient per `Outbound`, so a "broadcast" is one message per peer). The
/// message is encoded once per peer; peers are already in sorted order.
fn broadcast(out: &mut Vec<Outbound>, peers: &[PeerId], channel: Channel, msg: &Msg) {
    let bytes = wire::encode(msg);
    for peer in peers {
        out.push(Outbound {
            to: *peer,
            channel,
            bytes: bytes.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{self, InputCode, Msg};
    use crate::{BEACON_INTERVAL, INPUT_LEAD_TICKS, START_LEAD};
    use sim::Input;

    fn p(n: u32) -> PeerId {
        PeerId(n)
    }

    fn inbound(from: PeerId, channel: Channel, msg: &Msg) -> Inbound {
        Inbound {
            from,
            channel,
            bytes: wire::encode(msg),
        }
    }

    /// Decode all outbound messages addressed to `to`.
    fn decoded_to(out: &[Outbound], to: PeerId) -> Vec<Msg> {
        out.iter()
            .filter(|o| o.to == to)
            .map(|o| wire::decode(&o.bytes).unwrap())
            .collect()
    }

    /// Decode every outbound message regardless of recipient.
    fn decoded_all(out: &[Outbound]) -> Vec<Msg> {
        out.iter().map(|o| wire::decode(&o.bytes).unwrap()).collect()
    }

    #[test]
    fn emits_match_start_during_start_lead() {
        let mut d = Director::new(&[p(1), p(2)], 0xABCD);
        // Every iteration before START_LEAD broadcasts MatchStart to each player.
        for it in 0..START_LEAD {
            let out = d.tick(vec![]);
            for peer in [p(1), p(2)] {
                let msgs = decoded_to(&out, peer);
                assert!(
                    msgs.contains(&Msg::MatchStart {
                        start_tick: START_LEAD,
                        master_seed: 0xABCD,
                    }),
                    "expected MatchStart at iter {it} for {peer:?}, got {msgs:?}"
                );
            }
        }
        // After START_LEAD, no more MatchStart.
        let out = d.tick(vec![]);
        for m in decoded_all(&out) {
            assert!(!matches!(m, Msg::MatchStart { .. }), "no MatchStart after start");
        }
    }

    #[test]
    fn acks_input_and_applies_to_shadow_at_apply_tick() {
        let mut d = Director::new(&[p(1)], 7);
        // Advance to the first live tick (arena_tick 0 happens when iter==START_LEAD).
        for _ in 0..START_LEAD {
            d.tick(vec![]);
        }
        assert_eq!(d.server_tick(), 0);

        // Feed an Input at arena_tick 0; it must be acked with apply_tick = 0 + LEAD.
        let inb = inbound(p(1), Channel::Control, &Msg::Input { seq: 42, action: InputCode::Reroll });
        let out = d.tick(vec![inb]);
        let acks: Vec<Msg> = decoded_to(&out, p(1))
            .into_iter()
            .filter(|m| matches!(m, Msg::InputAck { .. }))
            .collect();
        assert_eq!(
            acks,
            vec![Msg::InputAck { seq: 42, apply_tick: INPUT_LEAD_TICKS }],
            "ack must echo seq and pin apply_tick = arena_tick + LEAD"
        );

        // The action must take effect on the shadow at exactly apply_tick.
        // Build an independent reference shadow that applies Reroll at the same
        // arena tick and compare checksums after stepping past apply_tick.
        // arena_tick was 0 when we submitted; tick() above already stepped tick 0.
        // Drive the director to apply_tick and capture its checksum.
        let target = INPUT_LEAD_TICKS;
        while d.server_tick() < target {
            d.tick(vec![]);
        }
        // d.server_tick() == target now; the step that consumed apply_tick=target
        // happened on the tick() call that moved server_tick from target-? ...
        // Drive one more so the Reroll-at-`target` step is consumed.
        d.tick(vec![]);
        let dir_cs = d.shadow_checksum(p(1)).unwrap();

        // Reference: a fresh shadow stepped identically, Reroll applied at tick==target.
        let mut reference = ArenaState::new(7, 1);
        for t in 0..=target {
            let inp = if t == target { Input::Reroll } else { Input::Noop };
            sim::step(&mut reference, inp);
        }
        let ref_cs = sim::checksum(&reference);
        assert_eq!(dir_cs, ref_cs, "director shadow must match reference with Reroll at apply_tick");
    }

    #[test]
    fn emits_time_beacon_on_interval() {
        let mut d = Director::new(&[p(1)], 1);
        // Reach arena_tick 0.
        for _ in 0..START_LEAD {
            d.tick(vec![]);
        }
        // arena_tick 0 -> beacon (0 % INTERVAL == 0).
        let out = d.tick(vec![]);
        assert!(
            decoded_to(&out, p(1)).iter().any(|m| matches!(m, Msg::TimeBeacon { server_tick: 0 })),
            "expected beacon at arena_tick 0"
        );

        // Step until the next interval boundary; collect when beacons occur.
        let mut seen_at = Vec::new();
        for _ in 1..(BEACON_INTERVAL * 2 + 1) {
            let t = d.server_tick();
            let out = d.tick(vec![]);
            if decoded_to(&out, p(1))
                .iter()
                .any(|m| matches!(m, Msg::TimeBeacon { server_tick } if *server_tick == t))
            {
                seen_at.push(t);
            }
        }
        assert!(seen_at.contains(&BEACON_INTERVAL), "beacon at {BEACON_INTERVAL}");
        assert!(seen_at.contains(&(BEACON_INTERVAL * 2)), "beacon at {}", BEACON_INTERVAL * 2);
        for t in seen_at {
            assert_eq!(t % BEACON_INTERVAL, 0, "beacon only on interval, saw {t}");
        }
    }

    #[test]
    fn snapshot_on_digest_mismatch_but_not_on_match() {
        let mut d = Director::new(&[p(1)], 99);
        for _ in 0..START_LEAD {
            d.tick(vec![]);
        }
        // Step one live tick so history[arena_tick 0] is recorded.
        d.tick(vec![]); // server_tick now 1, history has tick 0
        let recorded = d.shadow_checksum(p(1));
        assert!(recorded.is_some());

        // Find the recorded checksum at tick 0 by replaying a reference.
        let mut reference = ArenaState::new(99, 1);
        sim::step(&mut reference, Input::Noop);
        let cs0 = sim::checksum(&reference);

        // Matching digest -> no Snapshot.
        let good = inbound(p(1), Channel::Telemetry, &Msg::Digest { tick: 0, checksum: cs0 });
        let out = d.tick(vec![good]);
        assert!(
            !decoded_to(&out, p(1)).iter().any(|m| matches!(m, Msg::Snapshot { .. })),
            "no snapshot when digest matches"
        );

        // Mismatching digest -> Snapshot on Bulk.
        let bad = inbound(p(1), Channel::Telemetry, &Msg::Digest { tick: 0, checksum: cs0 ^ 0x1 });
        let out = d.tick(vec![bad]);
        let snaps: Vec<&Outbound> = out
            .iter()
            .filter(|o| matches!(wire::decode(&o.bytes), Ok(Msg::Snapshot { .. })))
            .collect();
        assert_eq!(snaps.len(), 1, "exactly one snapshot on mismatch");
        assert_eq!(snaps[0].to, p(1));
        assert_eq!(snaps[0].channel, Channel::Bulk);
        // The snapshot is the authoritative shadow as-of its labeled tick (state
        // at the start of the tick, before this tick's step). Verify it is
        // internally consistent: the deserialized arena's tick matches the label
        // and its checksum matches a reference advanced that many (Noop) ticks.
        // The client fast-forwards from this tick, so being one step "behind" the
        // director's post-tick shadow is correct, not a bug.
        if let Ok(Msg::Snapshot { bytes, tick }) = wire::decode(&snaps[0].bytes) {
            let restored = sim::snapshot::deserialize(&bytes).unwrap();
            assert_eq!(restored.tick, tick, "snapshot contents must match its label");
            let mut reference = ArenaState::new(99, 1);
            for _ in 0..restored.tick {
                sim::step(&mut reference, Input::Noop);
            }
            assert_eq!(sim::checksum(&restored), sim::checksum(&reference));
        } else {
            panic!("not a snapshot");
        }
        // Unknown tick in digest -> no snapshot.
        let unknown = inbound(p(1), Channel::Telemetry, &Msg::Digest { tick: 999_999, checksum: 0 });
        let out = d.tick(vec![unknown]);
        assert!(!decoded_to(&out, p(1)).iter().any(|m| matches!(m, Msg::Snapshot { .. })));
    }

    #[test]
    fn records_death_and_match_result() {
        // Single player: the default arena takes contact damage and eventually
        // dies, so the match must resolve with that player placed 1st.
        let mut d = Director::new(&[p(1)], 0x5151);
        for _ in 0..START_LEAD {
            d.tick(vec![]);
        }
        let mut death_seen: Option<Msg> = None;
        let mut result_seen: Option<Msg> = None;
        // Run a generous number of ticks; the lone tank should die from contact.
        for _ in 0..200_000u32 {
            let out = d.tick(vec![]);
            for m in decoded_all(&out) {
                match m {
                    Msg::DeathConfirmed { .. } if death_seen.is_none() => death_seen = Some(m),
                    Msg::MatchResult { .. } if result_seen.is_none() => result_seen = Some(m.clone()),
                    _ => {}
                }
            }
            if d.result().is_some() {
                break;
            }
        }
        let death = death_seen.expect("a DeathConfirmed must be emitted");
        match death {
            Msg::DeathConfirmed { player, place, .. } => {
                assert_eq!(player, 1);
                assert_eq!(place, 1, "last (only) to die is place 1");
            }
            _ => unreachable!(),
        }
        let result = result_seen.expect("a MatchResult must be emitted");
        assert_eq!(result, Msg::MatchResult { places: vec![(1, 1)] });
        assert!(!d.is_alive(p(1)), "shadow must be dead after resolution");
        assert_eq!(d.result(), Some(vec![(p(1), 1)]));
    }

    #[test]
    fn two_player_placement_orders_by_death() {
        // Force player 1 to die earlier than player 2 by giving player 2 a much
        // larger seed difference is not reliable; instead we drive both with the
        // default contact-death arena but stop player 1's stepping early by...
        // Simplest deterministic approach: run until both die and check that the
        // earlier death_tick gets the worse (higher) place.
        let mut d = Director::new(&[p(1), p(2)], 0x2222);
        for _ in 0..START_LEAD {
            d.tick(vec![]);
        }
        let mut deaths: Vec<(u32, u32, u32)> = Vec::new(); // (player, died_tick, place)
        for _ in 0..500_000u32 {
            let out = d.tick(vec![]);
            for m in decoded_all(&out) {
                if let Msg::DeathConfirmed { player, died_tick, place } = m {
                    if !deaths.iter().any(|(pl, _, _)| *pl == player) {
                        deaths.push((player, died_tick, place));
                    }
                }
            }
            if d.result().is_some() {
                break;
            }
        }
        assert!(d.result().is_some(), "match must resolve");
        let places = d.result().unwrap();
        // Both players placed; places are a permutation of {1, 2}.
        assert_eq!(places.len(), 2);
        let mut pl: Vec<u32> = places.iter().map(|(_, x)| *x).collect();
        pl.sort();
        assert_eq!(pl, vec![1, 2]);
        // The player that died last (max died_tick among deaths) must hold place 1.
        if deaths.len() == 2 {
            let last = deaths.iter().max_by_key(|(_, dt, _)| *dt).unwrap();
            assert_eq!(last.2, 1, "last to die is place 1");
        }
    }
}
