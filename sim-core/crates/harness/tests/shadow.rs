//! M1 exit tests: in-process shadow-sim divergence detection + snapshot
//! correction, and the reconnect-replay path (`docs/04 §4.4.6`).

use harness::shadow::*;
use harness::*;
use sim::snapshot::serialize;
use sim::*;

/// 1. The detector must not fire when client and shadow are identical sims.
#[test]
fn no_false_positive_corrections() {
    let sc = m0_scenario();
    let mut runner = ShadowRunner::new(sc.master_seed, sc.player_id, 30);
    for tick in 0..sc.total_ticks {
        runner.step(input_at(&sc, tick));
        assert_eq!(
            runner.corrections, 0,
            "spurious correction at tick {tick}: client and shadow are identical sims"
        );
    }
    assert!(
        runner.in_sync(),
        "client and shadow drifted with no input divergence"
    );
    assert_eq!(runner.last_correction_tick, None);
}

/// Drive a fresh runner to `target_tick` (exclusive) on the m0 scenario,
/// stopping right before that tick is processed. Returns the runner and the
/// scenario.
fn runner_at(target_tick: u32, digest_interval: u32) -> (ShadowRunner, Scenario) {
    let sc = m0_scenario();
    let mut runner = ShadowRunner::new(sc.master_seed, sc.player_id, digest_interval);
    for tick in 0..target_tick {
        runner.step(input_at(&sc, tick));
    }
    (runner, sc)
}

/// Apply a divergence to the client. Returns false if the injection kind was
/// not applicable to the current state (e.g. no enemies to corrupt).
fn inject(runner: &mut ShadowRunner, kind: usize) -> bool {
    match kind {
        // (a) economy gold drift.
        0 => {
            runner.client.economy.gold += 1;
            true
        }
        // (b) corrupt an enemy's hp, if any exist.
        1 => {
            if let Some(e) = runner.client.enemies.first_mut() {
                e.hp -= 1;
                true
            } else {
                false
            }
        }
        // (c) tank hp drift.
        2 => {
            runner.client.tank.hp -= 1;
            true
        }
        // (d) perturb an rng cursor.
        3 => {
            runner.client.economy.income_per_tick += 1;
            true
        }
        _ => false,
    }
}

/// 2. THE core M1 gate: an injected divergence at a non-digest-boundary tick is
///    detected and corrected within `digest_interval` ticks, and the client
///    reconverges stably for the rest of the scenario.
#[test]
fn injected_divergence_is_detected_and_corrected() {
    let digest_interval = 30u32;
    // 200 is not a multiple of 30, so the injection lands off a digest boundary.
    let inject_tick = 200u32;
    assert_ne!(inject_tick % digest_interval, 0);

    let kind_names = ["gold", "enemy_hp", "tank_hp", "income_per_tick"];

    for (kind, &kind_name) in kind_names.iter().enumerate() {
        let (mut runner, sc) = runner_at(inject_tick, digest_interval);
        assert_eq!(runner.corrections, 0);
        assert!(
            runner.in_sync(),
            "out of sync before injection (kind {kind})"
        );

        if !inject(&mut runner, kind) {
            // Not applicable to current state (e.g. no enemies). Skip — other
            // kinds cover the gate.
            continue;
        }

        // Immediately after injection the client and shadow must disagree.
        assert!(
            !runner.in_sync(),
            "injection kind {} ({}) did not actually diverge the client",
            kind,
            kind_name
        );

        // Step forward; within `digest_interval` ticks the next digest boundary
        // must detect and correct exactly once.
        let mut next_tick = inject_tick;
        let mut detected = false;
        for _ in 0..digest_interval {
            runner.step(input_at(&sc, next_tick));
            next_tick += 1;
            if runner.corrections == 1 {
                detected = true;
                break;
            }
        }
        assert!(
            detected,
            "kind {} ({}): divergence not corrected within {} ticks",
            kind, kind_name, digest_interval
        );
        assert_eq!(runner.corrections, 1);
        assert_eq!(runner.last_correction_tick, Some(next_tick));
        assert_eq!(
            next_tick % digest_interval,
            0,
            "correction must occur on a digest boundary"
        );
        assert!(
            runner.in_sync(),
            "kind {} ({}): not in sync after correction",
            kind,
            kind_name
        );

        // Reconvergence must be stable: continue to the end with no further
        // corrections.
        for tick in next_tick..sc.total_ticks {
            runner.step(input_at(&sc, tick));
            assert_eq!(
                runner.corrections, 1,
                "kind {} ({}): unexpected extra correction at tick {tick}",
                kind, kind_name
            );
        }
        assert!(
            runner.in_sync(),
            "kind {} ({}): drifted after reconvergence",
            kind,
            kind_name
        );
    }
}

/// 4. After a correction the client must be byte-identical to the shadow.
#[test]
fn corrected_client_byte_identical_to_shadow() {
    let (mut runner, sc) = runner_at(200, 30);
    runner.client.economy.gold += 12345;
    assert!(!runner.in_sync());

    // Advance to the next digest boundary, which corrects.
    let mut tick = 200u32;
    while runner.corrections == 0 {
        runner.step(input_at(&sc, tick));
        tick += 1;
        assert!(tick <= sc.total_ticks, "correction never occurred");
    }
    assert_eq!(runner.corrections, 1);
    assert_eq!(
        serialize(&runner.client),
        serialize(&runner.shadow),
        "corrected client must be byte-identical to the shadow"
    );
}

/// 3. Reconnect-replay (`docs/04 §4.4.6`): replaying from a snapshot at tick T
///    with the input log T..TARGET must reproduce the reference checksum at TARGET.
#[test]
fn reconnect_replay_matches() {
    let sc = m0_scenario();
    let t = 600u32;
    let target = 1500u32;
    assert!(t < target && target <= sc.total_ticks);

    // Reference sim: advance to T, snapshot, then continue to TARGET.
    let mut reference = ArenaState::new(sc.master_seed, sc.player_id);
    for tick in 0..t {
        step(&mut reference, input_at(&sc, tick));
    }
    let snapshot = serialize(&reference);

    // The inputs that drive ticks T..TARGET (each `step` consumes `input_at(tick)`).
    let inputs: Vec<Input> = (t..target).map(|tick| input_at(&sc, tick)).collect();

    for tick in t..target {
        step(&mut reference, input_at(&sc, tick));
    }

    let replayed = replay_from_snapshot(&snapshot, &inputs);

    assert_eq!(replayed.tick, reference.tick);
    assert_eq!(
        checksum(&replayed),
        checksum(&reference),
        "replay-from-snapshot diverged from the reference sim at tick {target}"
    );
}
