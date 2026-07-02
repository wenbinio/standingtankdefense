//! M5 chaos network test (`docs/06` M5). The M2/M3 suites proved the netcode is
//! correct on a CLEAN link and that one client's *stall* can't perturb another.
//! M5 sharpens that thesis against genuinely ADVERSE links — packet loss,
//! reordering, and high-latency hosts — all injected through the deterministic
//! seeded chaos knobs on [`net::hub::Hub`] so every run is byte-reproducible.
//!
//! ## How this protocol reacts to chaos (why the scenarios are shaped this way)
//! The client is NON-PREDICTIVE: an action applies only at the `apply_tick` the
//! director returns in an `InputAck`, on BOTH the client and the director's
//! shadow. Two consequences drive these tests:
//!  - Dropping a CLIENT→director message (a lost `Input`/`Digest`) cannot cause
//!    divergence — the action is simply never applied on either side, so the
//!    client stays bit-identical to its shadow (it just "misses" that action).
//!  - Dropping a DIRECTOR→client message (a lost `InputAck`/`Snapshot`) DOES
//!    diverge them: the shadow applied the action, the client didn't. The client
//!    detects this at the next `Digest` checkpoint and the director corrects it
//!    with an authoritative `Snapshot` (the recovery path). So we drive loss on
//!    the DIRECTOR link to exercise recovery, and pilot only the affected player
//!    with inputs so the recovery is localised.
//!
//! Two guarantees are asserted throughout:
//!  1. **Survival** — a client on a bad link recovers via digest→snapshot and its
//!     arena reconverges to its shadow; the match still resolves validly.
//!  2. **Independence (sharpened)** — chaos on one arena's link leaves every other
//!     arena's per-iteration trajectory BYTE-IDENTICAL to a no-chaos baseline.
//!     This is the core "no WC3-style global-lockstep coupling" guarantee.

use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{PeerId, DIRECTOR};
use net::{INPUT_LEAD_TICKS, START_LEAD};
use sim::bot::Bot;
use sim::{ArenaState, Input};

const HASH: u64 = 0xC0DE_C0DE;
const SEED: u64 = 0x004D_3543_4841_4F53; // "M5CHAOS"-ish
const CHAOS_SEED: u64 = 0x9E37_79B9; // explicit chaos-PRNG seed

/// What a single link should suffer this run.
#[derive(Clone, Copy, Default)]
struct Link {
    delay: u32,
    stall_until: u32,
    loss_permille: u32,
    jitter: u32,
    reorder: u32,
}

impl Link {
    fn apply(&self, hub: &mut Hub, peer: PeerId) {
        if self.delay > 0 {
            hub.set_delay(peer, self.delay);
        }
        if self.stall_until > 0 {
            hub.stall(peer, self.stall_until);
        }
        if self.loss_permille > 0 {
            hub.set_loss(peer, self.loss_permille);
        }
        if self.jitter > 0 {
            hub.set_jitter(peer, self.jitter);
        }
        if self.reorder > 0 {
            hub.set_reorder(peer, self.reorder);
        }
    }
}

/// Driver-side per-player input source. A `Bot` pilot mirrors the canonical
/// trajectory (its own `ArenaState` stepped in lockstep with the client, same
/// applied inputs at the same apply ticks) and issues `bot.decide(&mirror)` —
/// exactly what the player "sees" locally — without the client exposing its
/// private state. A `Noop` pilot never acts (a quiet, isolated baseline peer).
// Test-harness enum: the `Bot` variant inlines a full `ArenaState` mirror. The
// size skew vs `Quiet` is irrelevant here (a handful of instances, no hot path),
// so boxing would only add noise.
#[allow(clippy::large_enum_variant)]
enum Pilot {
    Bot {
        mirror: ArenaState,
        bot: Bot,
        schedule: net::Schedule,
    },
    Quiet,
}

impl Pilot {
    fn bot(seed: u64, player: u32) -> Pilot {
        Pilot::Bot {
            mirror: ArenaState::new(seed, player),
            bot: Bot::default(),
            schedule: net::Schedule::new(),
        }
    }

    /// The action the player intends at the client's current arena tick.
    fn desired(&mut self, client_tick: Option<u32>) -> Input {
        match (self, client_tick) {
            (
                Pilot::Bot {
                    mirror,
                    bot,
                    schedule,
                },
                Some(t),
            ) => {
                let act = bot.decide(mirror);
                if act != Input::Noop {
                    schedule.set(t + INPUT_LEAD_TICKS, act);
                }
                act
            }
            _ => Input::Noop,
        }
    }

    /// Advance the mirror one arena tick (only meaningful for a bot pilot).
    fn step(&mut self) {
        if let Pilot::Bot {
            mirror, schedule, ..
        } = self
        {
            let t = mirror.tick;
            let act = schedule.take(t);
            sim::step(mirror, act);
        }
    }
}

struct Trace {
    /// Per-iteration arena checksum of each client (index-aligned with `peers`).
    per_iter: Vec<Vec<Option<u64>>>,
    /// Final arena tick reached by each client.
    final_tick: Vec<Option<u32>>,
    /// Final arena checksum of each client.
    final_cs: Vec<Option<u64>>,
    /// Director's final shadow checksum per player.
    shadow_cs: Vec<Option<u64>>,
    /// In-sync corrections each client applied.
    corrections: Vec<u32>,
    /// The match result, if it resolved within the budget.
    result: Option<Vec<(PeerId, u32)>>,
}

/// Whether each player is bot-piloted (true) or quiet (false). `heal_at`, if set,
/// is the iteration at which all chaos knobs are dropped (the link recovers) —
/// used to assert the netcode reconverges once an outage ends. Delay/stall stay.
fn run(
    peers: &[PeerId],
    piloted: &[bool],
    links: &[Link],
    host_link: Link,
    iters: u32,
    heal_at: Option<u32>,
) -> Trace {
    let mut d = Director::with_content_hash(peers, SEED, HASH);
    let mut clients: Vec<Client> = peers.iter().map(|p| Client::new(*p, HASH)).collect();
    let mut pilots: Vec<Pilot> = peers
        .iter()
        .zip(piloted)
        .map(|(p, &on)| {
            if on {
                Pilot::bot(SEED, p.0)
            } else {
                Pilot::Quiet
            }
        })
        .collect();

    let mut hub = Hub::with_chaos_seed(CHAOS_SEED);
    host_link.apply(&mut hub, DIRECTOR);
    for (p, l) in peers.iter().zip(links) {
        l.apply(&mut hub, *p);
    }

    let mut per_iter: Vec<Vec<Option<u64>>> = peers.iter().map(|_| Vec::new()).collect();

    for it in 0..iters {
        if heal_at == Some(it) {
            hub.clear_chaos();
        }
        let in_d = hub.take(DIRECTOR);
        let out_d = d.tick(in_d);

        let mut client_out = Vec::new();
        for (i, c) in clients.iter_mut().enumerate() {
            let peer = peers[i];
            let inbox = hub.take(peer);
            let act = pilots[i].desired(c.arena_tick());
            let out = c.tick(inbox, act);
            client_out.push((peer, out));
        }

        // Advance each bot pilot's mirror once its client's arena is live.
        for (i, c) in clients.iter().enumerate() {
            if c.arena_tick().is_some() {
                pilots[i].step();
            }
        }

        hub.send(DIRECTOR, out_d);
        for (peer, outs) in client_out {
            hub.send(peer, outs);
        }
        hub.advance();

        for (i, c) in clients.iter().enumerate() {
            per_iter[i].push(c.arena_checksum());
        }
    }

    Trace {
        per_iter,
        final_tick: clients.iter().map(|c| c.arena_tick()).collect(),
        final_cs: clients.iter().map(|c| c.arena_checksum()).collect(),
        shadow_cs: peers.iter().map(|p| d.shadow_checksum(*p)).collect(),
        corrections: clients.iter().map(|c| c.corrections()).collect(),
        result: d.result(),
    }
}

fn peers(n: u32) -> Vec<PeerId> {
    (1..=n).map(PeerId).collect()
}

const NO_LINK: Link = Link {
    delay: 0,
    stall_until: 0,
    loss_permille: 0,
    jitter: 0,
    reorder: 0,
};

/// 40%-loss recovery + isolation. P1 is bot-piloted and the DIRECTOR link drops
/// 40% of its traffic — so P1's `InputAck`s are lost, the client diverges from
/// its shadow, and the digest→snapshot path drags it back. P2 is quiet, so it
/// has no ack traffic to lose: its per-iteration trace must be byte-identical to
/// a fully clean run (the lossy link is contained to P1's arena).
#[test]
fn lossy_link_recovers_and_does_not_couple() {
    let ps = peers(2);
    let lossy_iters = 400; // outage window
    let iters = 460; // + a healed tail so the final correction completes
    let piloted = [true, false]; // P1 acts; P2 is the quiet isolated baseline.

    // Clean run is also healed at the same iteration (a no-op on a clean hub), so
    // P2's per-iteration trace lines up tick-for-tick with the chaos run.
    let clean = run(
        &ps,
        &piloted,
        &[NO_LINK, NO_LINK],
        NO_LINK,
        iters,
        Some(lossy_iters),
    );

    // Director link drops 40% — this is the direction (acks/snapshots) that can
    // actually diverge a non-predictive client — then heals at `lossy_iters`.
    let lossy_host = Link {
        loss_permille: 400,
        ..NO_LINK
    };
    let chaos = run(
        &ps,
        &piloted,
        &[NO_LINK, NO_LINK],
        lossy_host,
        iters,
        Some(lossy_iters),
    );

    // SURVIVAL: P1 was forced to recover (lost acks → divergence → snapshot) and,
    // once the outage ended, its arena reconverged to the authoritative shadow.
    assert!(
        chaos.corrections[0] > 0,
        "40% director-link loss should have forced ≥1 snapshot correction on P1"
    );
    assert_eq!(
        chaos.final_cs[0], chaos.shadow_cs[0],
        "lossy P1 never reconverged with its authoritative shadow after the outage"
    );

    // INDEPENDENCE: quiet P2's whole per-iteration trace is byte-identical with
    // and without the lossy link. One arena's bad link cannot perturb another.
    assert_eq!(
        clean.per_iter[1], chaos.per_iter[1],
        "P2's trajectory was perturbed by the lossy link — sharding is broken"
    );
    assert_eq!(chaos.final_cs[1], chaos.shadow_cs[1]);
    assert_eq!(
        chaos.corrections[1], 0,
        "quiet P2 should need no corrections"
    );
}

/// Director input-ordering tolerance + isolation. P1's OWN link (player→director)
/// reorders the `Input`/`Digest` messages it sends within a small window, so the
/// director sees P1's inputs out of send order. The director pins each input's
/// `apply_tick` from the arena tick at RECEIPT and acks it individually; the
/// client applies each on its own ack. Because every action's apply tick is
/// agreed per-input (never derived from cross-message order), P1's client stays
/// bit-identical to its shadow with no corrections, and the quiet P2 arena is
/// untouched byte-for-byte.
#[test]
fn reordered_inputs_tolerated_and_isolated() {
    let ps = peers(2);
    let iters = 400;
    let piloted = [true, false];

    let clean = run(&ps, &piloted, &[NO_LINK, NO_LINK], NO_LINK, iters, None);

    // Reorder P1's uplink + a little base delay so its messages overlap in flight
    // and genuinely cross. INPUT_LEAD_TICKS (8) absorbs the extra arrival skew, so
    // every input still lands before its apply tick on both sides.
    let mut links = [NO_LINK, NO_LINK];
    links[0] = Link {
        reorder: 4,
        delay: 1,
        ..NO_LINK
    };
    let chaos = run(&ps, &piloted, &links, NO_LINK, iters, None);

    // Reorder never drops, and apply-ticks are agreed per-input, so a tick-keyed
    // protocol stays bit-identical: P1 matches its shadow with zero corrections.
    assert_eq!(
        chaos.final_cs[0], chaos.shadow_cs[0],
        "reordered inputs diverged P1 from its shadow — ordering is not tick-keyed"
    );
    assert_eq!(
        chaos.corrections[0], 0,
        "reorder (no loss, lead absorbs skew) must not require any correction"
    );

    // Quiet P2 untouched, byte-for-byte.
    assert_eq!(
        clean.per_iter[1], chaos.per_iter[1],
        "P2's trajectory was perturbed by P1's input reordering"
    );
    assert_eq!(chaos.corrections[1], 0);
}

/// A host (director) on a HIGH-LATENCY link must not change any client's LOCAL
/// tick RATE: once a client's arena exists it steps exactly once per driver
/// iteration regardless of how slowly authoritative traffic arrives. A bad host
/// link can only shift the match START uniformly (a delayed `MatchStart`); it can
/// never make one arena's clock run at a different rate than another's — that is
/// precisely the cross-arena coupling this architecture removes.
#[test]
fn high_latency_host_does_not_change_local_tick_rate() {
    let ps = peers(3);
    let iters = 350;
    let piloted = vec![false; ps.len()];
    let no_links = vec![NO_LINK; ps.len()];

    let clean = run(&ps, &piloted, &no_links, NO_LINK, iters, None);

    // ~300ms one-way at 30 Hz on the HOST link.
    let slow_host = Link {
        delay: 9,
        ..NO_LINK
    };
    let chaos = run(&ps, &piloted, &no_links, slow_host, iters, None);

    // The slow host delays every arena's START by the SAME amount — uniform, not
    // per-arena (the coupling-free property): all three clients reach the identical
    // final tick on each run, and the chaos run is uniformly behind the clean one.
    let clean_t = clean.final_tick[0].expect("clean clients started");
    let chaos_t = chaos.final_tick[0].expect("chaos clients started");
    assert!(
        clean.final_tick.iter().all(|x| *x == Some(clean_t)),
        "clean: arenas not uniform, got {:?}",
        clean.final_tick
    );
    assert!(
        chaos.final_tick.iter().all(|x| *x == Some(chaos_t)),
        "slow host must delay all arenas UNIFORMLY (no per-arena coupling), got {:?}",
        chaos.final_tick
    );
    // RATE unchanged: the ONLY difference is a constant START offset (the latency
    // the bootstrapping `MatchStart` now pays before arenas can begin). After that
    // one-time offset every arena still steps exactly once per iteration — so the
    // deficit is a single small constant bounded by the link delay, NOT a
    // compounding per-tick slowdown (which is what cross-arena coupling would be).
    assert!(chaos_t < clean_t, "the host latency cost must be real");
    let deficit = clean_t - chaos_t;
    assert!(
        deficit > 0 && deficit <= slow_host.delay,
        "tick deficit {deficit} must be a one-time start offset bounded by the \
         host delay {} — a larger deficit would mean the local step rate slowed",
        slow_host.delay
    );
    // The deficit is the SAME for every arena (uniform start shift, no per-arena
    // coupling) — already implied by the uniform finals above, asserted explicitly:
    for (i, p) in ps.iter().enumerate() {
        assert_eq!(
            clean.final_tick[i].unwrap() - chaos.final_tick[i].unwrap(),
            deficit,
            "player {}'s start offset differs — the host latency coupled arenas",
            p.0
        );
    }
}

/// THE SHARPENED M5 THESIS (named): a ~2s-equivalent stall (60 ticks @ 30 Hz)
/// AND a ~300ms-RTT-equivalent delay (≈9 ticks one-way) plus jitter on player A
/// leaves player B's per-iteration trace AND final tick count BYTE-IDENTICAL to a
/// no-chaos baseline. This is the explicit "no WC3-style global-lockstep
/// coupling" guarantee — A's adverse link is wholly contained to A's arena.
#[test]
fn player_a_chaos_leaves_player_b_byte_identical() {
    let ps = peers(2);
    let iters = 500;
    let piloted = [true, true]; // both act — B is a fully live arena, not a stub.

    let baseline = run(&ps, &piloted, &[NO_LINK, NO_LINK], NO_LINK, iters, None);

    // Player A (P1): ~2s stall + ~300ms one-way + jitter, all on A's own uplink.
    let a_chaos = Link {
        delay: 9,        // ≈300ms one-way at 30 Hz
        stall_until: 60, // ≈2s stall window
        jitter: 4,
        ..NO_LINK
    };
    let chaos = run(&ps, &piloted, &[a_chaos, NO_LINK], NO_LINK, iters, None);

    // B's entire per-iteration trajectory is byte-identical.
    assert_eq!(
        baseline.per_iter[1], chaos.per_iter[1],
        "player B's arena trajectory changed because of player A's lag — \
         WC3-style coupling has crept in"
    );
    // B's tick count/rate is identical: it never waited on A.
    assert_eq!(baseline.final_tick[1], chaos.final_tick[1]);
    assert_eq!(baseline.final_tick[1], Some(iters - START_LEAD));
    // B stayed on the canonical line throughout (matches its shadow, no fixes).
    assert_eq!(chaos.final_cs[1], chaos.shadow_cs[1]);
    assert_eq!(
        chaos.corrections[1], 0,
        "player B should never need a correction"
    );

    // Sanity that the chaos was REAL, not a no-op: A's stalled/delayed uplink lands
    // its inputs at later apply ticks, so A's own arena trajectory genuinely
    // differs from the baseline — yet A still converges to its own shadow (the
    // protocol absorbs the lag without a correction, since acks pin apply ticks).
    assert_ne!(
        baseline.per_iter[0], chaos.per_iter[0],
        "player A's lagged uplink should have changed A's own trajectory"
    );
    assert_eq!(
        chaos.final_cs[0], chaos.shadow_cs[0],
        "player A drifted from its own shadow despite the lag being absorbable"
    );
}

/// The match still RESOLVES to a valid `MatchResult` under sustained chaos on one
/// link: every place is assigned exactly once and the standings are a permutation
/// of `1..=N`. Run long enough that the bot-piloted tanks fight to the death.
#[test]
fn match_resolves_under_chaos() {
    let ps = peers(2);
    // QUIET pilots: with no defensive buys the lone tanks die to the swarm within
    // ~1k ticks, so both arenas resolve well inside this budget even under chaos.
    let iters = 2500;
    let piloted = [false, false];
    let a_chaos = Link {
        delay: 6,
        loss_permille: 150,
        jitter: 3,
        reorder: 2,
        ..NO_LINK
    };
    let host = Link {
        loss_permille: 80,
        ..NO_LINK
    };

    let tr = run(&ps, &piloted, &[a_chaos, NO_LINK], host, iters, None);

    let result = tr
        .result
        .expect("match must resolve to a MatchResult under chaos");
    assert_eq!(result.len(), ps.len(), "every player must be placed");
    let mut places: Vec<u32> = result.iter().map(|(_, pl)| *pl).collect();
    places.sort();
    assert_eq!(
        places,
        (1..=ps.len() as u32).collect::<Vec<_>>(),
        "places must be a permutation of 1..=N"
    );
}
