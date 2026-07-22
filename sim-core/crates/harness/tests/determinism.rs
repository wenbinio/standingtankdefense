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
        0x0c0ba40d0da8aa23, // re-baselined (MERGED SP-DIFFICULTY + DAMAGE-ATTRIBUTION passes, snapshot v24): both passes proved field-domain-only shifts independently (difficulty via disable-the-write reproduction of the prior golden; the ledger via a bit-identical behavior probe over the full M0 scenario); the merged digest domain contains both field sets, so this value was captured fresh from `cargo run -p harness` on the merged tree. (Previous baselines: 0xb7c1308925f50f6d difficulty-only, 0x7b9690c6d0e661f0 pre-both.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
