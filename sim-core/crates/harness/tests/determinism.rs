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
        0x3b7f3e9de60f57f0, // re-baselined (BATTLE FERVOR SCOPING): `Modifiers::healing_weapon_healthy_dmg` entered the checksum stream — Battle Fervor's +35% is now scoped to healing weapons (HealingWeaponDamagePct / WeaponDef::is_healing) instead of the documented global DamageWhileHealthyPct approximation. The M0 TRAJECTORY is unchanged: the scripted bot never buys Battle Fervor and owns no healing weapon (probed: both healthy-damage aggregates zero, healing_mult untouched at the final tick), so the shift is purely the new checksummed field in the digest. Captured fresh from `cargo run -p harness`. (Previous baseline 0x672269466cd58df0 was the source-arc restoration.)
        "M0 golden checksum drift — determinism broke OR content/scenario changed intentionally"
    );
}
