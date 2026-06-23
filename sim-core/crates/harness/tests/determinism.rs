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
        0x935c95ffa9386f92, // re-baselined (EXPANSION batch E1): four NEW source upgrades appended to the modifier catalog (Mastercrafted Masonry, Golden Ring, Arcane Mark, Maw of Death) with their four dynamic mechanics (DamagePerMaxHp / DamagePerBountyPct / ShieldActiveDamagePct / ManaOnKill). The M0 bot buys from the catalog, so adding entries shifts its shop draw and the trace. No EXISTING entry's effects changed, and the RAMP/boss/wave curve is untouched. (Previous baseline 0xcab3bbf00f8b3da1 was the content-fidelity pass.) SNAPSHOT_VERSION 17→18 for the new checksum-feeding fields
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
