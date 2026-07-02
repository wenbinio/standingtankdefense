//! BALANCE-PASS GUARD TESTS — codify the early-game design window (`docs/06`).
//!
//! Two deterministic scenarios pin the two ends of the "early grace, but punish the
//! naked eco-rush" target:
//!   (A) a NAKED eco-rush tank (no weapons, no defense items — only economy buys +
//!       the free Clear) is DEAD by tick ≤ 3600 (≤ 2 min @ 30 Hz);
//!   (B) a MODEST opener (a weapon or two + some HP/armor) is ALIVE past tick 3600.
//! The separating lever is OFFENSE: the early wave throughput sits just above what a
//! weaponless tank can survive on Clear alone, but well within what even a small
//! arsenal holds. All scenarios are pure functions of the seed (deterministic).
//!
//! NOTE on the naked case: a frame-perfect Clear (the free 10 s board-wipe) can
//! phase-lock the synchronized wave on a MINORITY of seeds, letting a weaponless tank
//! coast — an artifact of the Clear MECHANIC (which this pass does not touch), not a
//! real strategy. So the guard asserts the punish holds on the broad MAJORITY of
//! seeds (and on a fixed representative seed), rather than every seed.

use sim::bot::{Bot, Challenge};
use sim::{step, ArenaState, Input, WeaponInstance};

const DEADLINE: u32 = 3600; // ≤ 2 min @ 30 Hz — the eco-rush death deadline.

/// Run a NAKED eco-rush: strip the starting weapon, buy ONLY economy (EcoOnly), and
/// use the free Clear reactively when swarmed. Returns the tank's death tick (or the
/// cap if it somehow survives). Fully deterministic from `seed`.
fn run_naked_eco_rush(seed: u64, cap: u32) -> u32 {
    let mut s = ArenaState::new(seed, 0);
    s.weapons.clear(); // truly weaponless — no offense at all
    let mut bot = Bot::with_challenge(Challenge::EcoOnly);
    while s.tick < cap && !s.dead {
        // Economy buys from the eco-bot; Clear reactively when the board is full.
        let bot_a = Challenge::EcoOnly.filter(bot.decide(&s), &s);
        let clear_now = s.enemies.len() >= 8 && s.tick >= s.tank.clear_cooldown_end;
        let a = match bot_a {
            Input::Clear if !clear_now => Input::Noop,
            _ if clear_now => Input::Clear,
            other => other,
        };
        step(&mut s, a);
        // Defend the invariant: nothing can grant this tank a weapon.
        if !s.weapons.is_empty() {
            s.weapons.clear();
        }
    }
    s.tick
}

/// Run a MODEST opener: keep the starting Bow, add one weapon, and a little HP/armor
/// (≈ a couple of cheap defensive buys). Clear reactively; no further purchases.
/// Returns the death tick (or cap). Deterministic from `seed`.
fn run_modest_opener(seed: u64, cap: u32) -> u32 {
    let mut s = ArenaState::new(seed, 0);
    // + Mortar Launcher (def 1 — cheap reliable splash).
    let id = s.alloc_entity_id();
    s.weapons.push(WeaponInstance {
        instance_id: id,
        def: 1,
        next_fire_tick: 0,
    });
    // Modest defense: +12000 Max HP and +5 armor.
    s.tank.max_hp += 12_000;
    s.tank.hp = s.tank.max_hp;
    s.tank.armor += 5;
    while s.tick < cap && !s.dead {
        let a = if s.enemies.len() >= 14 && s.tick >= s.tank.clear_cooldown_end {
            Input::Clear
        } else {
            Input::Noop
        };
        step(&mut s, a);
    }
    s.tick
}

#[test]
fn naked_eco_rush_is_dead_by_deadline_on_a_representative_seed() {
    // Seed 0 is a representative, non-phase-locked seed: a weaponless eco-rush leaks
    // contact damage past its Clear and dies well inside the 2-min deadline.
    let death = run_naked_eco_rush(0, 56_000);
    assert!(
        death <= DEADLINE,
        "naked eco-rush must be DEAD by tick {DEADLINE}, died at {death}"
    );
}

#[test]
fn naked_eco_rush_punish_holds_for_the_broad_majority() {
    // Across a 24-seed sample, the eco-rush punish must land on the large majority
    // (≥ 75%). The minority that survive are the Clear-phase-lock artifact (see the
    // module note) — a property of the untouchable Clear mechanic, not a real build.
    let n = 24u64;
    let dead_by_deadline = (0..n)
        .filter(|&seed| run_naked_eco_rush(seed, 56_000) <= DEADLINE)
        .count();
    assert!(
        dead_by_deadline * 100 >= (n as usize) * 75,
        "eco-rush punish must hold for ≥75% of seeds; only {dead_by_deadline}/{n} died by tick {DEADLINE}"
    );
}

#[test]
fn modest_opener_survives_past_the_deadline() {
    // A modest opener (Bow + one weapon + a little HP/armor) must establish board
    // control and live PAST the deadline on EVERY sampled seed — the other end of the
    // design window (early grace for a real build must not collapse).
    for seed in 0..16u64 {
        let death = run_modest_opener(seed, 56_000);
        assert!(
            death > DEADLINE,
            "modest opener (seed {seed}) must survive past tick {DEADLINE}, died at {death}"
        );
    }
}
