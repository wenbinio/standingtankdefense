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
        let director_out = if it < MIGRATE_AT { out_primary } else { out_standby };

        let mut client_out = Vec::new();
        for (i, c) in clients.iter_mut().enumerate() {
            let peer = PeerId(i as u32 + 1);
            let inbox = hub.take(peer);
            let act = desired(peer, c.arena_tick());
            client_out.push((peer, c.tick(inbox, act)));
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
    for &p in &ps {
        let c = &clients[(p.0 - 1) as usize];
        assert_eq!(
            c.arena_checksum(),
            standby.shadow_checksum(p),
            "player {} lost sync with the new director after migration",
            p.0
        );
        assert!(c.corrections() == 0, "player {} needed a correction across migration", p.0);
    }

    // The match actually progressed past the handoff.
    assert!(standby.server_tick() > MIGRATE_AT);
}
