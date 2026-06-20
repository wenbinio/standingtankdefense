//! Input application — AGENT C. Applies one player action this tick.
use crate::ids::Input;
use crate::shop;
use crate::state::*;

/// Phase 2: apply `inp`.
/// - `Noop`: nothing.
/// - `BuyOffer{slot}`: if slot valid and `gold >= offer.cost`, deduct gold and
///   push a new `WeaponInstance` (def = offer.weapon_def, fresh id,
///   `next_fire_tick = s.tick`). Otherwise ignore (illegal/insufficient).
/// - `Reroll`: if `rerolls_remaining > 0`, decrement and `shop::generate_offers`;
///   else if `gold >= reroll_cost`, deduct and regenerate; then raise reroll_cost
///   (e.g. +100). Otherwise ignore.
/// - `Clear`: if `s.tick >= tank.clear_cooldown_end`, deal a burst to all enemies
///   in some radius (M0: damage all enemies; kills award bounty via
///   `economy::award_bounty`) and set cooldown (e.g. +300 ticks). Else ignore.
pub(crate) fn apply(s: &mut ArenaState, inp: Input) {
    let _ = (inp, shop::generate_offers as fn(&mut ArenaState), s);
    todo!("AGENT C: input::apply")
}
