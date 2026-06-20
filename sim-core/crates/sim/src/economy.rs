//! Economy — AGENT C. Integer gold; bounty scales by `s.economy.bounty_mult`,
//! passive income does NOT (source rule, `docs/02 §2.4`).
use crate::state::*;

/// Called once when a new round begins (before input). M0: no-op or a small
/// per-round bonus. Keep deterministic.
pub(crate) fn on_round_start(s: &mut ArenaState) {
    let _ = s;
    todo!("AGENT C: economy::on_round_start")
}

/// Phase 7: add `s.economy.income_per_tick` to gold (no multiplier).
pub(crate) fn tick_income(s: &mut ArenaState) {
    todo!("AGENT C: economy::tick_income")
}

/// Award kill bounty for an enemy of `enemy_def`: `gold += floor(bounty * bounty_mult)`.
/// Called by combat on each kill.
pub(crate) fn award_bounty(s: &mut ArenaState, enemy_def: u16) {
    let _ = (enemy_def, s);
    todo!("AGENT C: economy::award_bounty")
}

/// Phase 8: if `s.tank.hp <= 0` and not already dead, set `dead = true` and
/// `death_tick = Some(s.tick)`.
pub(crate) fn resolve_deaths(s: &mut ArenaState) {
    todo!("AGENT C: economy::resolve_deaths")
}
