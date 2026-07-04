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
        0x5767f463a5dfe375, // re-baselined (COMBINED fidelity passes): the ECONOMY pass (weighted independent shop draws, bonus-only income scaling, held Magic Treasure pool, `Economy::treasure_pool`/`PendingPerk::scope` in the digest) and the E3 MECHANICS pass (opt-in Deep Freeze trajectory change, 6 mechanic-anchor weapons + Deep Freeze modifier growing the shop pool, new tank/enemy-status/modifier fields + rotating-wave `sweeps` in the digest, snapshot v21) merged; the combined trajectory differs from either pass alone, so this value was captured fresh from `cargo run -p harness` on the merged tree. (Previous baseline 0x09058789ad2df90a was the §9.3 event-stream pass.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
