//! M2 exit test (`docs/06`): with the authoritative director + two thin clients
//! wired through the deterministic Hub, prove the core thesis —
//! **one client's stall cannot affect another client's arena** — and that a
//! healthy client stays bit-identical to its authoritative shadow through the
//! real message transport.

use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{PeerId, DIRECTOR};
use sim::Input;

const P1: PeerId = PeerId(1);
const P2: PeerId = PeerId(2);
const SEED: u64 = 0x5747_5354_4431; // "STD" ish
const HASH: u64 = 0xC0DE_C0DE;
const ITERS: u32 = 600;

/// Deterministic per-player input intents, keyed by the client's arena tick.
fn desired_p1(at: Option<u32>) -> Input {
    match at {
        Some(15) => Input::BuyOffer { slot: 0 },
        Some(90) => Input::Reroll,
        Some(95) => Input::BuyOffer { slot: 0 },
        Some(250) => Input::Clear,
        _ => Input::Noop,
    }
}
fn desired_p2(at: Option<u32>) -> Input {
    match at {
        Some(20) => Input::BuyOffer { slot: 1 },
        Some(120) => Input::Reroll,
        Some(125) => Input::BuyOffer { slot: 0 },
        Some(300) => Input::Clear,
        _ => Input::Noop,
    }
}

struct Outcome {
    c2_trace: Vec<Option<u64>>,
    c1_final: u64,
    c2_final: u64,
    shadow1: Option<u64>,
    shadow2: Option<u64>,
    c1_corrections: u32,
    c2_corrections: u32,
    c2_final_tick: Option<u32>,
}

/// Run the 2-player match. If `stall_p1_until` is set, P1's outbound messages are
/// held until that hub step (a lag window), simulating a laggy player 1.
fn run(stall_p1_until: Option<u32>) -> Outcome {
    let mut d = Director::new(&[P1, P2], SEED);
    let mut c1 = Client::new(P1, HASH);
    let mut c2 = Client::new(P2, HASH);
    let mut hub = Hub::new();
    if let Some(until) = stall_p1_until {
        hub.stall(P1, until);
    }

    let mut c2_trace = Vec::with_capacity(ITERS as usize);
    for _ in 0..ITERS {
        let in_d = hub.take(DIRECTOR);
        let in_1 = hub.take(P1);
        let in_2 = hub.take(P2);

        let out_d = d.tick(in_d);
        let out_1 = c1.tick(in_1, desired_p1(c1.arena_tick()));
        let out_2 = c2.tick(in_2, desired_p2(c2.arena_tick()));

        hub.send(DIRECTOR, out_d);
        hub.send(P1, out_1);
        hub.send(P2, out_2);
        hub.advance();

        c2_trace.push(c2.arena_checksum());
    }

    Outcome {
        c2_trace,
        c1_final: c1.arena_checksum().unwrap(),
        c2_final: c2.arena_checksum().unwrap(),
        shadow1: d.shadow_checksum(P1),
        shadow2: d.shadow_checksum(P2),
        c1_corrections: c1.corrections(),
        c2_corrections: c2.corrections(),
        c2_final_tick: c2.arena_tick(),
    }
}

/// THE M2 thesis: P2's entire arena trace is byte-identical whether or not P1 is
/// stalled. One client's lag cannot perturb another client's simulation — the
/// exact failure mode of WC3's global lockstep that this architecture removes.
#[test]
fn one_clients_stall_does_not_affect_the_other() {
    let healthy = run(None);
    let p1_stalled = run(Some(200)); // P1 lagging for the first 200 hub steps

    assert_eq!(
        healthy.c2_trace, p1_stalled.c2_trace,
        "P2's arena diverged because of P1's stall — the independence thesis is broken"
    );
    // And P2 actually progressed the whole match in both cases: it steps its
    // arena once per iteration from iter == START_LEAD onward.
    assert_eq!(healthy.c2_final_tick, p1_stalled.c2_final_tick);
    assert_eq!(healthy.c2_final_tick, Some(ITERS - net::START_LEAD));
}

/// Through the real transport, each healthy client ends bit-identical to its
/// authoritative shadow, and no corrections were needed (inputs flow + apply at
/// the agreed tick on both sides).
#[test]
fn healthy_clients_match_their_shadows() {
    let o = run(None);
    assert_eq!(o.c1_final, o.shadow1.unwrap(), "P1 client != its shadow");
    assert_eq!(o.c2_final, o.shadow2.unwrap(), "P2 client != its shadow");
    assert_eq!(o.c1_corrections, 0, "P1 needed corrections on a clean link");
    assert_eq!(o.c2_corrections, 0, "P2 needed corrections on a clean link");
}

/// P2 (healthy) is unaffected even while P1 is stalled: P2 still matches its
/// shadow and never needs a correction.
#[test]
fn healthy_peer_unaffected_while_other_is_stalled() {
    let o = run(Some(200));
    assert_eq!(o.c2_final, o.shadow2.unwrap(), "stalled P1 perturbed P2 vs its shadow");
    assert_eq!(o.c2_corrections, 0, "P2 needed a correction due to P1's stall");
}
