//! M3 exit test (part 1): **8 players + reconnect**. Eight independent arenas
//! run under one authoritative director. One player drops mid-match and rejoins
//! as a fresh `reconnecting` client; the director hands it an authoritative
//! snapshot, it adopts and continues on the canonical trajectory — while the
//! other seven are entirely unaffected (`docs/06` M3).

use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{PeerId, DIRECTOR};
use sim::Input;

const SEED: u64 = 0x38_504C_4159_4552; // "8PLAYER"-ish
const HASH: u64 = 0xC0DE;
const N: u32 = 8;
const ITERS: u32 = 700;
const DROP_AT: u32 = 100; // iteration P3 disconnects
const REJOIN_AT: u32 = 320; // iteration a fresh P3 client reconnects
const DROPPED: u32 = 3; // peer id that drops/reconnects

fn peers() -> Vec<PeerId> {
    (1..=N).map(PeerId).collect()
}

#[test]
fn eight_players_one_drops_and_reconnects() {
    let ps = peers();
    let mut d = Director::new(&ps, SEED);
    // clients[i] for peer i+1; None while disconnected.
    let mut clients: Vec<Option<Client>> = ps.iter().map(|p| Some(Client::new(*p, HASH))).collect();
    let mut hub = Hub::new();

    for it in 0..ITERS {
        // Disconnect P3: drop the client and discard its queued inbound.
        if it == DROP_AT {
            clients[(DROPPED - 1) as usize] = None;
        }
        // Reconnect P3 as a brand-new client that must request state via Join.
        if it == REJOIN_AT {
            clients[(DROPPED - 1) as usize] = Some(Client::reconnecting(PeerId(DROPPED), HASH));
        }

        let in_d = hub.take(DIRECTOR);
        let out_d = d.tick(in_d);

        let mut client_out = Vec::new();
        for (i, slot) in clients.iter_mut().enumerate() {
            let peer = PeerId(i as u32 + 1);
            let inbox = hub.take(peer);
            match slot {
                Some(c) => client_out.push((peer, c.tick(inbox, Input::Noop))),
                None => { /* disconnected: inbox discarded */ }
            }
        }

        hub.send(DIRECTOR, out_d);
        for (peer, outs) in client_out {
            hub.send(peer, outs);
        }
        hub.advance();
    }

    // The seven never-dropped players are on the canonical trajectory, and their
    // director shadow is a faithful (possibly lagging) prefix of it.
    //
    // NOTE: we compare each side to a fresh reference advanced to ITS OWN arena
    // tick rather than directly equating `client.arena_checksum()` with
    // `shadow_checksum()`. After P3 drops, the director's shadow clock stalls a few
    // ticks behind the live clients, so the client and shadow sit at DIFFERENT
    // arena ticks. The old direct equality only passed by accident — under Noop
    // inputs the sparse early-game board reached a steady state where the checksum
    // stopped changing, so the two different ticks happened to hash equal. The
    // difficulty-curve redesign makes the early board churn through this window
    // (denser swarm), exposing that the two ticks are genuinely different states.
    // Comparing each side at its own tick is the tick-correct lockstep check (it is
    // exactly how the reconnected P3 is validated below).
    for &p in &ps {
        if p.0 == DROPPED {
            continue;
        }
        let c = clients[(p.0 - 1) as usize].as_ref().unwrap();
        // Client is bit-equal to the canonical trajectory at its own tick.
        let c_tick = c.arena_tick().unwrap();
        let mut c_ref = sim::ArenaState::new(SEED, p.0);
        for _ in 0..c_tick {
            sim::step(&mut c_ref, Input::Noop);
        }
        assert_eq!(
            c.arena_checksum().unwrap(),
            sim::checksum(&c_ref),
            "player {} client drifted from the canonical trajectory",
            p.0
        );
        // Director's shadow is bit-equal to the canonical trajectory at ITS tick
        // (a prefix of the client's), proving shadow↔client lockstep modulo lag.
        let s_ck = d.shadow_checksum(p).unwrap();
        let mut s_ref = sim::ArenaState::new(SEED, p.0);
        let mut s_tick = 0u32;
        // Advance the reference until it matches the shadow checksum, bounded by
        // the client's tick (the shadow can never be ahead of the client).
        let matched = loop {
            if sim::checksum(&s_ref) == s_ck {
                break true;
            }
            if s_tick >= c_tick {
                break false;
            }
            sim::step(&mut s_ref, Input::Noop);
            s_tick += 1;
        };
        assert!(
            matched,
            "player {} shadow not on the canonical trajectory",
            p.0
        );
    }

    // The reconnected P3 adopted authoritative state and stayed on the canonical
    // trajectory: zero corrections, behind the live tick, and bit-equal to a
    // reference advanced to its (lower) tick.
    let p3 = clients[(DROPPED - 1) as usize].as_ref().unwrap();
    assert!(p3.started(), "P3 never reconnected");
    assert_eq!(
        p3.corrections(),
        0,
        "reconnect should need no in-sync corrections"
    );

    let p3_tick = p3.arena_tick().unwrap();
    assert!(p3_tick > 0, "P3 made no progress after reconnect");
    assert!(
        p3_tick < d.server_tick(),
        "P3 should be behind the live director tick after a mid-match rejoin"
    );

    let mut reference = sim::ArenaState::new(SEED, DROPPED);
    for _ in 0..p3_tick {
        sim::step(&mut reference, Input::Noop);
    }
    assert_eq!(
        p3.arena_checksum().unwrap(),
        sim::checksum(&reference),
        "reconnected P3 is not on the canonical trajectory"
    );
}
