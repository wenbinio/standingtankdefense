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
        0xa1938d079a0f035b, // re-baselined (ECONOMY-FIDELITY PASS): (1) the shop draws every slot independently — category, then weighted rarity (`shop::RARITY_WEIGHTS`), then uniform-in-bucket — changing the RNG draw pattern and the offered items; (2) %-income multipliers now scale only BONUS income above the 20/tick base (source rule, `docs/02 §2.4`); (3) Magic Treasure is a held +2/s pool banked at the shop roll instead of instant gold + an income ramp; (4) new authoritative fields `Economy::treasure_pool` and `PendingPerk::scope` feed the checksum (snapshot v21). (Previous baseline 0x09058789ad2df90a was the §9.3 event-stream pass.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
