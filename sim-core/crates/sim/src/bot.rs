//! A deterministic auto-player so previews and the spectator/net demo can play
//! themselves. It reads only the public [`ArenaState`] (the same surface a real
//! client/UI sees) and returns one [`Input`] per tick — proving the sim is
//! drivable through its render-facing API. No RNG, no wall-clock: purely a
//! function of observed state, so it never threatens determinism.

use crate::{ArenaState, Input, OfferKind};

/// Greedy survivor policy: clear when swarmed, otherwise keep buying.
pub struct Bot {
    /// Ticks to wait before acting again (keeps it from buying every frame).
    cooldown: u32,
    /// Stop *preferring* weapons over upgrades once we own this many.
    target_weapons: usize,
}

impl Default for Bot {
    fn default() -> Self {
        Bot { cooldown: 0, target_weapons: 12 }
    }
}

impl Bot {
    /// Choose this tick's action from the current state.
    pub fn decide(&mut self, s: &ArenaState) -> Input {
        if s.dead {
            return Input::Noop;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return Input::Noop;
        }

        // Emergency: swarmed and the Clear is off cooldown → wipe the board.
        if s.enemies.len() >= 14 && s.tick >= s.tank.clear_cooldown_end {
            self.cooldown = 30;
            return Input::Clear;
        }

        // Otherwise buy the best affordable offer. Prefer a weapon while under
        // the target arsenal size; among same-kind, take the cheapest.
        let want_weapon = s.weapons.len() < self.target_weapons;
        let mut best: Option<(usize, i64, bool)> = None; // (slot, cost, is_weapon)
        for (i, off) in s.shop.offers.iter().enumerate() {
            if off.cost > s.economy.gold {
                continue;
            }
            let is_weapon = matches!(off.kind, OfferKind::Weapon);
            let better = match best {
                None => true,
                Some((_, bcost, bweapon)) => {
                    if is_weapon != bweapon {
                        // Kind preference wins outright.
                        is_weapon == want_weapon
                    } else {
                        off.cost < bcost
                    }
                }
            };
            if better {
                best = Some((i, off.cost, is_weapon));
            }
        }
        if let Some((slot, _, _)) = best {
            self.cooldown = 6;
            return Input::BuyOffer { slot: slot as u8 };
        }

        // Nothing affordable — spend a free reroll to fish for cheaper options.
        if s.economy.rerolls_remaining > 0 {
            self.cooldown = 20;
            return Input::Reroll;
        }
        Input::Noop
    }
}
