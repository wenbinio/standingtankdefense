//! Clock-synchronization tests (`docs/03 §3.7`). The director emits a
//! `TimeBeacon { server_tick }` on an interval; a non-predictive client uses it
//! to keep an honest estimate of AUTHORITATIVE server time (for round/boss-timer
//! UI and to detect local-clock drift) without touching its arena/input/snapshot
//! paths. These tests run the real director + a client wired through the
//! deterministic `Hub` under latency/jitter and assert the estimate stays close
//! to the director's true `server_tick()`.

use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{PeerId, DIRECTOR};
use net::{BEACON_INTERVAL, START_LEAD};
use sim::Input;

const P1: PeerId = PeerId(1);
const SEED: u64 = 0x5747_5354_4432;
const HASH: u64 = 0xC0DE_BEEF;

/// Acceptable gap between the client's `server_tick()` estimate and the
/// director's real `server_tick()` once the clock has locked. The estimate
/// trails by roughly the one-way delay (the beacon's travel time) plus a tick of
/// easing slack, so a few ticks is expected; we just bound it.
const MAX_GAP: u32 = 10;

/// One driver iteration shared by all tests: pump the hub, step director+client,
/// flush outbound. Returns the gap |director.server_tick - client.server_tick|
/// once the client has a clock estimate (else `None`).
fn step(d: &mut Director, c: &mut Client, hub: &mut Hub) -> Option<u32> {
    let in_d = hub.take(DIRECTOR);
    let in_c = hub.take(P1);

    let out_d = d.tick(in_d);
    let out_c = c.tick(in_c, Input::Noop);

    hub.send(DIRECTOR, out_d);
    hub.send(P1, out_c);
    hub.advance();

    c.server_tick().map(|est| {
        let truth = d.server_tick();
        truth.abs_diff(est)
    })
}

/// Under a laggy + jittery link, an INITIAL client's server-time estimate locks
/// onto the director's real clock and stays within `MAX_GAP` for the whole run.
#[test]
fn estimate_tracks_director_under_latency_and_jitter() {
    let mut d = Director::new(&[P1], SEED);
    let mut c = Client::new(P1, HASH);
    let mut hub = Hub::with_chaos_seed(0xC10C);
    hub.set_delay(DIRECTOR, 3); // beacons take 3 steps to reach the client
    hub.set_jitter(DIRECTOR, 2); // ...with +0..=2 jitter on top

    let mut worst_after_lock: u32 = 0;
    let mut locked = false;
    for _ in 0..600 {
        if let Some(gap) = step(&mut d, &mut c, &mut hub) {
            // After the first couple of beacons the clock is "locked"; track the
            // worst gap from then on (the very first fix can be a few ticks off).
            if locked {
                worst_after_lock = worst_after_lock.max(gap);
            }
            // Two beacon intervals of warm-up before we start bounding the gap.
            if c.server_tick().unwrap() >= 2 * BEACON_INTERVAL {
                locked = true;
            }
        }
    }

    assert!(locked, "client never acquired a server-time estimate");
    assert!(
        worst_after_lock <= MAX_GAP,
        "post-lock gap {worst_after_lock} exceeded {MAX_GAP} ticks under jitter"
    );
    // Finishes well-synced: within a single beacon interval of the truth.
    let final_gap = d.server_tick().abs_diff(c.server_tick().unwrap());
    assert!(
        final_gap <= BEACON_INTERVAL,
        "ended out of sync by {final_gap}"
    );
}

/// A client whose estimate STARTS far behind the truth catches up via beacons:
/// the snap-on-large-drift path drags it back to the director's clock and raises
/// the resync flag. We model "starts behind" by feeding the client a stale beacon
/// first, then letting real beacons correct it.
#[test]
fn behind_client_catches_up_via_beacons() {
    use net::transport::{Channel, Inbound};
    use net::wire::{self, Msg};

    let mut d = Director::new(&[P1], SEED);
    let mut c = Client::new(P1, HASH);
    let mut hub = Hub::new();

    // Run the director forward so its clock is well past 0, and start the client.
    let mut warm = 0;
    while d.server_tick() < 200 {
        step(&mut d, &mut c, &mut hub);
        warm += 1;
        assert!(warm < 1000, "director failed to advance");
    }

    // Inject a STALE beacon (server_tick far below the truth) straight into the
    // client, simulating a client that anchored its clock way behind.
    let stale = Inbound {
        from: DIRECTOR,
        channel: Channel::Telemetry,
        bytes: wire::encode(&Msg::TimeBeacon { server_tick: 50 }),
    };
    c.tick(vec![stale], Input::Noop);
    let behind = d.server_tick().abs_diff(c.server_tick().unwrap());
    assert!(
        behind > 100,
        "setup: client should start far behind, was {behind}"
    );

    // Now let real beacons flow. The next genuine beacon's drift exceeds the snap
    // threshold, so the estimate snaps forward (and flags a resync).
    let mut converged = false;
    for _ in 0..(BEACON_INTERVAL * 4) {
        step(&mut d, &mut c, &mut hub);
        let gap = d.server_tick().abs_diff(c.server_tick().unwrap());
        if gap <= MAX_GAP {
            converged = true;
            break;
        }
    }
    assert!(
        converged,
        "behind client never caught up to the director's clock"
    );
    assert!(
        c.take_clock_resync(),
        "a large-drift snap must flag a resync"
    );
}

/// With NO beacons arriving (the director's telemetry is black-holed), the client
/// free-runs gracefully: once it has a single fix it keeps advancing at the sim
/// rate, never panics, and stays a plausible monotonically-increasing estimate.
#[test]
fn no_beacons_free_runs_gracefully() {
    let mut d = Director::new(&[P1], SEED);
    let mut c = Client::new(P1, HASH);
    let mut hub = Hub::new();

    // Let exactly one beacon through to seed the estimate, then black-hole the
    // director's outbound entirely (the client gets no more beacons/acks).
    let mut seeded_at: Option<u32> = None;
    for _ in 0..(START_LEAD + BEACON_INTERVAL + 5) {
        step(&mut d, &mut c, &mut hub);
        if seeded_at.is_none() && c.server_tick().is_some() {
            seeded_at = Some(c.server_tick().unwrap());
            break;
        }
    }
    assert!(seeded_at.is_some(), "client never got an initial beacon");
    hub.set_loss(DIRECTOR, 1000); // black-hole all further director traffic

    let mut last = c.server_tick().unwrap();
    for _ in 0..300 {
        step(&mut d, &mut c, &mut hub);
        let now = c.server_tick().unwrap();
        // Estimate keeps advancing monotonically (free-run), by at most 1/tick.
        assert!(now >= last, "free-run estimate went backwards");
        assert!(now - last <= 1, "free-run jumped by more than a tick");
        last = now;
    }
    // No beacon ⇒ drift was never re-measured ⇒ no spurious resync flag.
    assert!(
        !c.take_clock_resync(),
        "free-running client must not flag a resync"
    );
    // And the free-run stayed sane: still within a beacon-ish band of the truth,
    // since both advance at the same rate from a shared anchor.
    let gap = d.server_tick().abs_diff(c.server_tick().unwrap());
    assert!(
        gap <= MAX_GAP,
        "free-run drifted by {gap} despite matched rates"
    );
}
