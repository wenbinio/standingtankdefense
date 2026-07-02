//! M3 exit test (part 2): **host migration**. A hot-standby director is fed the
//! same inbound as the primary in lockstep, so it holds bit-identical state. On
//! host loss the standby takes over and clients continue without interruption —
//! possible because the director's state is small and fully derived from
//! `master_seed` + the ordered input log (`docs/03 §3.10`, `docs/07 §7.5`).

use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{PeerId, DIRECTOR};
use sim::Input;

const SEED: u64 = 0x484F_5354_4D49_4752; // "HOSTMIGR"
const HASH: u64 = 0xBEEF;
const N: u32 = 3;
const ITERS: u32 = 600;
const MIGRATE_AT: u32 = 300; // iteration the primary host dies

fn peers() -> Vec<PeerId> {
    (1..=N).map(PeerId).collect()
}

/// A little input traffic on both sides of the handoff, to prove acks/inputs
/// keep working when the standby takes over.
fn desired(p: PeerId, at: Option<u32>) -> Input {
    match (p.0, at) {
        (3, Some(120)) => Input::BuyOffer { slot: 1 }, // before migration
        (1, Some(280)) => Input::BuyOffer { slot: 0 }, // just before migration
        (2, Some(360)) => Input::Reroll,               // after migration
        _ => Input::Noop,
    }
}

#[test]
fn standby_takes_over_on_host_loss() {
    let ps = peers();
    let mut primary = Director::new(&ps, SEED);
    let mut standby = Director::new(&ps, SEED);
    let mut clients: Vec<Client> = ps.iter().map(|p| Client::new(*p, HASH)).collect();
    let mut hub = Hub::new();
    let mut asserted_handoff = false;

    // Per-player history of the client's (arena_tick → checksum). The new
    // director's shadow lags the live clients by a few ticks, so after migration
    // we look the client's OWN past checksum up at the shadow's (lagging) tick and
    // compare — a tick-correct lockstep check (see the post-loop note).
    use std::collections::HashMap;
    let mut client_history: Vec<HashMap<u32, u64>> = ps.iter().map(|_| HashMap::new()).collect();

    for it in 0..ITERS {
        // The director's inbox is fed IDENTICALLY to both hosts each tick, so the
        // standby stays a bit-perfect hot copy.
        let in_d = hub.take(DIRECTOR);
        let out_standby = standby.tick(in_d.clone());
        let out_primary = if it <= MIGRATE_AT {
            primary.tick(in_d)
        } else {
            Vec::new() // primary host is gone
        };

        // At the handoff the two hosts must be bit-identical.
        if it == MIGRATE_AT && !asserted_handoff {
            for &p in &ps {
                assert_eq!(
                    primary.shadow_checksum(p),
                    standby.shadow_checksum(p),
                    "standby diverged from primary for player {} before handoff",
                    p.0
                );
            }
            asserted_handoff = true;
        }

        // Clients hear the primary until the handoff, the standby after.
        let director_out = if it < MIGRATE_AT {
            out_primary
        } else {
            out_standby
        };

        let mut client_out = Vec::new();
        for (i, c) in clients.iter_mut().enumerate() {
            let peer = PeerId(i as u32 + 1);
            let inbox = hub.take(peer);
            let act = desired(peer, c.arena_tick());
            client_out.push((peer, c.tick(inbox, act)));
            // Record this client's checksum at its current arena tick.
            if let (Some(t), Some(ck)) = (c.arena_tick(), c.arena_checksum()) {
                client_history[i].insert(t, ck);
            }
        }

        hub.send(DIRECTOR, director_out);
        for (peer, outs) in client_out {
            hub.send(peer, outs);
        }
        hub.advance();
    }

    assert!(asserted_handoff, "migration boundary was never exercised");

    // After migration, every client is still in lockstep with the NEW director
    // (the former standby) — the handoff was seamless.
    //
    // NOTE (iter-2 difficulty curve): the new director's shadow clock LAGS the
    // live clients by a few ticks (input acks still in flight at the handoff), so
    // `client.arena_checksum()` and `standby.shadow_checksum(p)` sit at DIFFERENT
    // arena ticks. Equating them directly only passed by accident: under the
    // sparse pre-iter-2 early board the checksum reached a steady state and stopped
    // changing across that small lag window, so two different ticks hashed equal.
    // The iter-2 brutal opening (denser grunt floor + early raider/bandit rush)
    // keeps the board churning through this window, exposing that the two ticks are
    // genuinely different states — the same latent tick-lag bug iter-1 fixed in
    // `eight_players_reconnect`.
    //
    // The fix is tick-correct: instead of equating the client's CURRENT checksum
    // with the new director's shadow checksum (different ticks ⇒ spurious
    // mismatch), we look the client's OWN recorded checksum up at the shadow's
    // (lagging) arena tick. If the client and the new director are truly in
    // lockstep, the shadow's state must equal the client's state at that same tick.
    for &p in &ps {
        let i = (p.0 - 1) as usize;
        let c = &clients[i];
        let s_tick = standby.shadow(p).unwrap().tick;
        let c_now = c.arena_tick().unwrap();
        assert!(
            s_tick <= c_now,
            "new-director shadow cannot lead the client for player {}",
            p.0
        );
        let expected = client_history[i].get(&s_tick).copied().unwrap_or_else(|| {
            panic!(
                "no client checksum recorded at shadow tick {s_tick} for player {}",
                p.0
            )
        });
        assert_eq!(
            standby.shadow_checksum(p),
            Some(expected),
            "player {} lost sync with the new director after migration (at shadow tick {s_tick})",
            p.0
        );
        assert!(
            c.corrections() == 0,
            "player {} needed a correction across migration",
            p.0
        );
    }

    // The match actually progressed past the handoff.
    assert!(standby.server_tick() > MIGRATE_AT);
}
