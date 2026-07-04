//! The M0 exit test (`docs/06`). CI-gated, runs on every platform in the matrix.
use harness::*;

#[test]
fn identical_runs_produce_identical_traces() {
    let sc = m0_scenario();
    assert_eq!(run_trace(&sc), run_trace(&sc), "replay diverged");
}

#[test]
fn trace_has_expected_length() {
    let sc = m0_scenario();
    assert_eq!(run_trace(&sc).len(), sc.total_ticks as usize);
}

#[test]
fn different_seed_changes_output() {
    let sc = m0_scenario();
    let mut sc2 = m0_scenario();
    sc2.master_seed ^= 0xFFFF_FFFF_FFFF_FFFF;
    assert_ne!(final_checksum(&sc), final_checksum(&sc2));
}

/// Cross-platform determinism tripwire. Every OS/CPU in CI must reproduce this
/// EXACT value — that is what proves bit-identical simulation across machines,
/// not just same-machine reproducibility. If you intentionally change content
/// or the M0 scenario, re-capture this constant from `cargo run -p harness`.
#[test]
fn golden_final_checksum_is_stable() {
    let sc = m0_scenario();
    assert_eq!(
        final_checksum(&sc),
        0x672269466cd58df0, // re-baselined (SOURCE-ARC RESTORATION — the largest legitimate trajectory change of the fidelity program): the match arc returned from the interim 30-minute / 3-min-stepped-ramp shape to the source's 15-minute arc (docs/01 §1.2) — BOSS_SPAWN_TICK 54000→27000, a smooth +25%/min curve with a 2-min opening grace, the +20% step at 10:00, the post-15:00 swift end (waves continue, ×1.5/min), the 15:00 shop close + ramp stop, wave gates compressed onto 15 min, the escort swarm removed, and the boss re-statted (30M HP / 16k fixed contact). Every scripted trajectory shifts; captured fresh from `cargo run -p harness`. (Previous baseline 0x20e2dabc7009c960 was the catalog-fidelity pass.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
