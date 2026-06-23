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
        0xcab3bbf00f8b3da1, // re-baselined (content-fidelity pass): the modifier catalog was rebuilt (restored source names, re-bundled split multi-effect upgrades, merged "Golden Vitality" into "Entangled Gold Mine", added MaxHpPct/HpRegenPct/ManaRegenPct riders). The M0 bot buys from the catalog, so the trace shifts; the RAMP/boss curve is unchanged. The interim difficulty re-tune (RAMP_JUMP 1.22→3.5, boss endpoint ×11→×413863) does NOT drift this: the M0 scenario runs only 2000 ticks, all within the k=0 interval before the first cliff (tick 5400), where the curve uses the UNCHANGED gentle/warning factors
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
