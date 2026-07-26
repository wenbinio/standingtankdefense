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
        0x1712c0e6eea71024, // re-baselined (FUN PASS, `docs/11-fun.md`): four balance changes landed together — rarity-weighted shop draws (Epic ~2% early / ~21% late, replacing a flat ~12%), a rebuilt WAVE_M0 that removes the flat 6-20 min plateau, boss contact_damage 45000 -> 1900 (it was an HP check, not a fight), RAMP_JUMP 1.14 -> 1.28, roster base_hp x1.5-4, and soft caps + an arsenal-breadth synergy in `modifiers`. All of these feed enemy HP/contact, offer identity and damage scaling, so the scripted M0 trace's tank HP / kills / economy move and the checksum with them. No determinism property changed: the sim is still integer/fixed-point, seeded, and bit-identical across machines — this constant is re-captured, not weakened. (Previous baseline 0x436067271f1956d8 was the interim balance pass.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
