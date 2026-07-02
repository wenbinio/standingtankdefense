//! M5 soak test (`docs/06` M5): eight players, full content, a LONG match driven
//! well past the 30-minute boss spawn (`content::BOSS_SPAWN_TICK == 54000`), on a
//! CLEAN link. Where `chaos.rs` proves the netcode survives adverse links, the
//! soak proves it stays correct and BOUNDED over a very long, fully-loaded run —
//! the kind of marathon that surfaces drift, leaks, and slow-growth bugs.
//!
//! All eight tanks are bot-piloted (the full shop / weapon / modifier / boss
//! pipeline exercised), driven through the real director↔client message transport
//! via the deterministic [`Hub`]. The whole run is a pure function of the seed.
//!
//! Asserted:
//!  1. **No checksum drift** — sampled throughout the match, every client's local
//!     arena is bit-identical to its authoritative director shadow.
//!  2. **Correction rate ≈ 0** — on a clean link no client ever needs a snapshot
//!     correction, all the way past the boss (the R1/R7 "quiet healthy path").
//!  3. **No unbounded growth** — the director's per-player `arena_tick → checksum`
//!     history is CAPPED (`HISTORY_LEN`), proven behaviorally: a digest for a tick
//!     older than the cap is silently ignored (its entry was trimmed) while a
//!     recent tick still triggers a correction. The buffer does not grow with
//!     match length.

use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{Channel, Inbound, PeerId, DIRECTOR};
use net::wire::{self, Msg};
use net::{INPUT_LEAD_TICKS, START_LEAD};
use sim::bot::Bot;
use sim::{content, ArenaState, Input};

const N: u32 = 8;
const HASH: u64 = 0x5DA1_C0FF_EE15_500A; // a representative content version
/// Seed chosen so the bot builds carry the tanks deep into the boss phase, so the
/// late-game content (boss + escort flood) is actually exercised under the net
/// transport rather than every arena dying in the opening minute.
const SEED: u64 = 7;
/// Run past the boss so the boss FIGHT (spawn + escort flood), not just its
/// arrival, runs through the netcode. ~30.5 minutes of match @ 30 Hz.
const BUDGET: u32 = content::BOSS_SPAWN_TICK + 800; // 54800

fn peers() -> Vec<PeerId> {
    (1..=N).map(PeerId).collect()
}

/// Driver-side bot pilot: a canonical-trajectory mirror arena + a `Bot`, used to
/// produce each client's `desired` input without the client exposing its private
/// `ArenaState`. The mirror is stepped in lockstep with the client (same applied
/// inputs at the same apply ticks), so `bot.decide(&mirror)` is what the player
/// sees locally.
struct Pilot {
    mirror: ArenaState,
    bot: Bot,
    schedule: net::Schedule,
}

impl Pilot {
    fn new(player: u32) -> Pilot {
        Pilot {
            mirror: ArenaState::new(SEED, player),
            bot: Bot::default(),
            schedule: net::Schedule::new(),
        }
    }
    fn desired(&mut self, client_tick: Option<u32>) -> Input {
        match client_tick {
            Some(t) => {
                let act = self.bot.decide(&self.mirror);
                if act != Input::Noop {
                    self.schedule.set(t + INPUT_LEAD_TICKS, act);
                }
                act
            }
            None => Input::Noop,
        }
    }
    fn step(&mut self) {
        let t = self.mirror.tick;
        let act = self.schedule.take(t);
        sim::step(&mut self.mirror, act);
    }
}

#[test]
fn eight_player_full_match_soak_stays_correct_and_bounded() {
    let ps = peers();
    let mut d = Director::with_content_hash(&ps, SEED, HASH);
    let mut clients: Vec<Client> = ps.iter().map(|p| Client::new(*p, HASH)).collect();
    let mut pilots: Vec<Pilot> = ps.iter().map(|p| Pilot::new(p.0)).collect();
    let mut hub = Hub::new();

    // Iteration to fire the in-soak liveness probe — deep into the match (the
    // director has logged tens of thousands of ticks, so its history has been
    // trimmed thousands of times) but comfortably before any tank can die (the
    // boss arrives at 54000), so P1 is certainly alive and its shadow clock is
    // live. It injects ONE mismatching digest for a RECENT, logged tick from P1
    // and confirms the director still answers with a Snapshot. The snapshot
    // reflects P1's true state, which a healthy P1 already holds, so it is a no-op
    // correction and does NOT perturb the "0 corrections" assertion.
    let probe_iter = 30_000 + START_LEAD;

    let total = BUDGET + START_LEAD;
    let mut sampled_checks = 0u32;
    let mut probe_fired = false;
    let mut recent_triggered_snapshot = false;
    let mut boss_seen = false;

    // Per-client `arena_tick → checksum` history. Once a player's shadow DIES the
    // director freezes it (stops stepping), so the shadow's tick falls behind the
    // still-running client clock. Comparing both at the *same call* would then read
    // two different ticks. The tick-correct lockstep check (same one `host_migration`
    // uses) looks the client's OWN recorded checksum up at the shadow's (possibly
    // frozen) tick: if they are truly in lockstep, those must be equal.
    use std::collections::HashMap;
    let mut client_history: Vec<HashMap<u32, u64>> = ps.iter().map(|_| HashMap::new()).collect();

    for it in 0..total {
        let mut in_d = hub.take(DIRECTOR);

        if it == probe_iter && !probe_fired {
            assert!(d.is_alive(PeerId(1)), "probe must fire while P1 is alive");
            // Derive the recent tick from P1's LIVE shadow clock (a logged,
            // DIGEST-aligned tick a couple of intervals back, firmly inside the
            // retained window).
            let s_tick = d.shadow(PeerId(1)).unwrap().tick;
            let recent_tick = s_tick.saturating_sub(net::DIGEST_INTERVAL * 2)
                / net::DIGEST_INTERVAL
                * net::DIGEST_INTERVAL;
            in_d.push(digest_from(PeerId(1), recent_tick, 0x0BAD_0BAD_0BAD_0BAD));
            probe_fired = true;
        }

        let out_d = d.tick(in_d);

        if it == probe_iter {
            recent_triggered_snapshot = out_d.iter().any(|o| {
                o.to == PeerId(1) && matches!(wire::decode(&o.bytes), Ok(Msg::Snapshot { .. }))
            });
        }

        let mut client_out = Vec::new();
        for (i, c) in clients.iter_mut().enumerate() {
            let inbox = hub.take(ps[i]);
            let act = pilots[i].desired(c.arena_tick());
            client_out.push((ps[i], c.tick(inbox, act)));
        }
        for (i, c) in clients.iter().enumerate() {
            if c.arena_tick().is_some() {
                pilots[i].step();
                if pilots[i]
                    .mirror
                    .enemies
                    .iter()
                    .any(|e| content::ENEMIES[e.def as usize].boss)
                {
                    boss_seen = true;
                }
            }
        }

        hub.send(DIRECTOR, out_d);
        for (peer, outs) in client_out {
            hub.send(peer, outs);
        }
        hub.advance();

        // Record each client's checksum at its current arena tick every iteration,
        // so the shadow's (possibly frozen) tick is always available to compare.
        for (i, c) in clients.iter().enumerate() {
            if let (Some(t), Some(cs)) = (c.arena_tick(), c.arena_checksum()) {
                client_history[i].insert(t, cs);
            }
        }

        // --- No-drift sampling (tick-correct) -----------------------------------
        // Sample every ~1000 iterations across the whole match. For each player,
        // look up the client's recorded checksum at the SHADOW's current tick and
        // require it to equal the shadow's checksum — true iff client and shadow
        // are in lockstep at that tick. The shadow tick is always ≤ the client tick
        // (it can never lead), so the entry is always present.
        if it % 1_000 == 0 && it >= START_LEAD {
            for (i, _c) in clients.iter().enumerate() {
                if let (Some(shadow), Some(sh_cs)) = (d.shadow(ps[i]), d.shadow_checksum(ps[i])) {
                    let s_tick = shadow.tick;
                    if let Some(&recorded) = client_history[i].get(&s_tick) {
                        assert_eq!(
                            recorded, sh_cs,
                            "checksum DRIFT for player {} at iteration {it}: client's \
                             state at shadow tick {s_tick} != the shadow",
                            ps[i].0
                        );
                        sampled_checks += 1;
                    }
                }
            }
        }
    }

    // The stale-vs-recent trim (the actual BOUND) is verified by an isolated,
    // fast director-only probe: a far-past digest is ignored (trimmed) while a
    // current one triggers a snapshot. Keeping it separate avoids perturbing the
    // long soak's clean state.
    let (stale_ignored, recent_corrected) = probe_history_bound();

    // 1. We actually sampled the drift check many times across the long match.
    assert!(
        sampled_checks > 50,
        "too few drift samples ({sampled_checks}) — soak too short?"
    );
    // The boss phase was genuinely reached and exercised through the netcode.
    assert!(
        boss_seen,
        "the boss never appeared — the soak did not reach the boss phase"
    );

    // 2. Correction rate ≈ 0 on a clean link, all the way past the boss.
    for (i, c) in clients.iter().enumerate() {
        assert_eq!(
            c.corrections(),
            0,
            "player {} needed {} snapshot correction(s) on a CLEAN link — the \
             healthy path must be quiet (R1/R7)",
            ps[i].0,
            c.corrections()
        );
    }

    // 3. Bounded history: a digest older than the cap is trimmed (ignored), while
    //    a current one is still actionable. The director's per-player history does
    //    not grow with match length.
    assert!(
        stale_ignored,
        "a far-past digest still drew a correction — director history is UNBOUNDED"
    );
    assert!(
        recent_corrected,
        "a current mismatching digest drew no correction — the live window is broken \
         (so the 'stale ignored' result would be meaningless)"
    );
    // The probe also fired inside the live soak (the recent injected digest drew a
    // snapshot there), confirming the bound check is exercised in-context too.
    assert!(
        recent_triggered_snapshot,
        "the in-soak recent-digest probe never triggered a snapshot"
    );
}

/// Build an inbound `Digest` message as if sent by `from`.
fn digest_from(from: PeerId, tick: u32, checksum: u64) -> Inbound {
    Inbound {
        from,
        channel: Channel::Telemetry,
        bytes: wire::encode(&Msg::Digest { tick, checksum }),
    }
}

/// Isolated, fast check of the director's history BOUND: drive a single player far
/// enough that early ticks are trimmed, then feed (a) a digest for a long-trimmed
/// tick and (b) a digest for a still-recorded recent tick, both with wrong
/// checksums. Returns `(stale_ignored, recent_corrected)`.
fn probe_history_bound() -> (bool, bool) {
    let p = PeerId(1);
    let mut d = Director::with_content_hash(&[p], SEED, HASH);
    // Advance past the (≤256-entry) history cap but while the lone tank is still
    // ALIVE (seed 7's idle tank dies ~tick 755), so the shadow clock is live and
    // `server_tick` equals the shadow tick — the early ticks are already trimmed.
    let run = START_LEAD + 600;
    for _ in 0..run {
        d.tick(vec![]);
    }
    let now = d.server_tick();
    assert!(
        d.is_alive(p),
        "probe tank must still be alive ({now} ticks)"
    );
    // A tick from the very start of the match (long since trimmed from history).
    let stale_tick = 0u32;
    // A tick guaranteed inside the recent (retained) window and DIGEST-aligned.
    let recent_tick =
        (now - net::DIGEST_INTERVAL * 2) / net::DIGEST_INTERVAL * net::DIGEST_INTERVAL;

    let stale_out = d.tick(vec![digest_from(p, stale_tick, 0x1234_5678)]);
    let recent_out = d.tick(vec![digest_from(p, recent_tick, 0x8765_4321)]);

    let any_snapshot = |out: &[net::Outbound]| {
        out.iter()
            .any(|o| matches!(wire::decode(&o.bytes), Ok(Msg::Snapshot { .. })))
    };
    (!any_snapshot(&stale_out), any_snapshot(&recent_out))
}
