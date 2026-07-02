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
        0x58d77dad0cc11999, // re-baselined (B4 CHECKSUM-COMPLETENESS PASS): `checksum()` now covers every snapshot-carried, behavior-relevant field it previously missed — `Projectile::last_target_pos` (splash detonation point after target death) plus `Tank::{shield_active_dr, heal_on_damaged, mana_on_kill}`, `Modifiers::{dmg_per_maxhp_rate, dmg_per_bounty_rate, shield_active_dmg}`, and `ArenaState::{bought_attack_mask, weapons_bought, economy_purchases}`. The SIMULATION TRAJECTORY is unchanged — only the digest domain grew, which shifts the hash. Parity with the snapshot layout is now enforced by `crates/sim/tests/checksum_parity.rs`. (Previous baseline 0x436067271f1956d8 was the balance pass.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
