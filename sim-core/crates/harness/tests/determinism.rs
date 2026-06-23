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
        0x436067271f1956d8, // re-baselined (BALANCE PASS): the enemy HP/contact CURVE (`enemy_hp_mult`) was reshaped — the interim ×413863 hack replaced by a smooth integer-parametrized ramp to a ≈ ×5.56 boss endpoint (`RAMP_JUMP` 3.5→1.14) — and the early wave cadences + the staggered SPAWN_RING radii changed. All of these feed enemy HP/contact, which changes the scripted M0 trace's tank HP / kills / economy → the checksum. The Fixed arithmetic is now SATURATING (a deterministic, platform-stable clamp) but that is bit-identical for every in-range value, so it does NOT contribute to this drift. No item/weapon EFFECT changed; the M0 scenario inputs are unchanged. (Previous baseline 0x7f221f1dd37b0233 was the E2 expansion.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
