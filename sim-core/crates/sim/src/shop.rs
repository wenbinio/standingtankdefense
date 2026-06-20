//! Shop / offers — AGENT C. Deterministic; randomness only from `s.rng_shop`.
use crate::content;
use crate::state::*;

/// Generate a fresh set of offers (recommend 3 slots) by drawing weapon defs
/// from `content::WEAPONS` via `s.rng_shop`. Set each `Offer.cost` from the
/// weapon's `cost`. Replace `s.shop.offers`, increment `s.shop.shop_seq`.
/// Called at each round boundary and on reroll.
pub(crate) fn generate_offers(s: &mut ArenaState) {
    let _ = &content::WEAPONS;
    todo!("AGENT C: shop::generate_offers")
}
