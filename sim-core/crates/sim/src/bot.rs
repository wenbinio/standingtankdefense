//! A deterministic auto-player so previews and the spectator/net demo can play
//! themselves. It reads only the public [`ArenaState`] (the same surface a real
//! client/UI sees) and returns one [`Input`] per tick — proving the sim is
//! drivable through its render-facing API. No RNG, no wall-clock: purely a
//! function of observed state, so it never threatens determinism.

use crate::{content, ArenaState, Input, Offer, OfferKind};

/// A self-imposed playstyle constraint the bot will honor while buying, so the
/// preview can earn the cosmetic challenge achievements on demand. Purely a
/// purchase filter over the public shop — it never bends the sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Challenge {
    #[default]
    None,
    /// Buy only weapons of this attack-class (scope id 0..6).
    Purist(u8),
    /// Never buy income / gold modifiers.
    NoEconomy,
    /// Prefer collecting a weapon of every attack-class.
    JackOfAll,
}

impl Challenge {
    /// Map the GDScript-facing integer code to a challenge. 1..=6 → Purist of
    /// attack-class 0..5, 7 → NoEconomy, 8 → JackOfAll, anything else → None.
    pub fn from_code(code: i64) -> Challenge {
        match code {
            1..=6 => Challenge::Purist((code - 1) as u8),
            7 => Challenge::NoEconomy,
            8 => Challenge::JackOfAll,
            _ => Challenge::None,
        }
    }

    /// Whether this challenge permits buying `off`.
    pub fn permits(&self, off: &Offer) -> bool {
        match (self, off.kind) {
            (Challenge::Purist(class), OfferKind::Weapon) => {
                content::attack_scope_id(content::WEAPONS[off.def as usize].attack) == *class
            }
            (Challenge::NoEconomy, OfferKind::Modifier) => {
                !content::MODIFIERS[off.def as usize].effect.is_economy()
            }
            _ => true,
        }
    }

    /// Authoritative apply-time guard: drop a `BuyOffer` that this challenge
    /// forbids (resolved against the shop the buy actually lands on), so a
    /// slot-based purchase can never violate the rule no matter how the shop
    /// shifted between the bot's decision and the lead-delayed apply tick.
    /// A pure function of `(input, state, challenge)` — applied identically on
    /// the director and the client, it keeps their shadows in lockstep.
    pub fn filter(&self, inp: Input, s: &ArenaState) -> Input {
        if let Input::BuyOffer { slot } = inp {
            match s.shop.offers.get(slot as usize) {
                Some(off) if !self.permits(off) => return Input::Noop,
                _ => {}
            }
        }
        inp
    }
}

/// Greedy survivor policy: clear when swarmed, otherwise keep buying.
pub struct Bot {
    /// Ticks to wait before acting again (keeps it from buying every frame).
    cooldown: u32,
    /// Stop *preferring* weapons over upgrades once we own this many.
    target_weapons: usize,
    /// Optional self-imposed purchase constraint (challenge runs).
    challenge: Challenge,
}

impl Default for Bot {
    fn default() -> Self {
        Bot { cooldown: 0, target_weapons: 12, challenge: Challenge::None }
    }
}

impl Bot {
    /// A bot that honors `c` while buying (otherwise identical to `default`).
    pub fn with_challenge(c: Challenge) -> Self {
        Bot { challenge: c, ..Bot::default() }
    }

    fn weapon_class(def: u16) -> u8 {
        content::attack_scope_id(content::WEAPONS[def as usize].attack)
    }

    /// Whether the active challenge permits buying this offer.
    fn allowed(&self, off: &Offer) -> bool {
        self.challenge.permits(off)
    }

    /// Cheapest affordable slot holding a weapon this challenge wants, if any.
    /// For Purist that's the target class; for JackOfAll an attack-class we
    /// don't own yet; otherwise None (generic buying handles the default bot).
    fn wanted_weapon(&self, s: &ArenaState) -> Option<usize> {
        let mut pick: Option<(usize, i64)> = None;
        for (i, off) in s.shop.offers.iter().enumerate() {
            if off.cost > s.economy.gold || !matches!(off.kind, OfferKind::Weapon) {
                continue;
            }
            let wanted = match self.challenge {
                Challenge::Purist(class) => Self::weapon_class(off.def) == class,
                Challenge::JackOfAll => {
                    s.bought_attack_mask & (1u16 << Self::weapon_class(off.def)) == 0
                }
                _ => false,
            };
            if wanted && pick.map_or(true, |(_, c)| off.cost < c) {
                pick = Some((i, off.cost));
            }
        }
        pick.map(|(slot, _)| slot)
    }

    /// Can we reroll to fish for a wanted weapon (free, or affordable paid)?
    fn can_reroll(&self, s: &ArenaState) -> bool {
        s.economy.rerolls_remaining > 0 || s.economy.gold >= s.economy.reroll_cost
    }

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
        // A constrained run is weaker, so it leans on Clear sooner to survive
        // long enough for its achievement to land.
        let clear_at = if self.challenge == Challenge::None { 14 } else { 8 };
        if s.enemies.len() >= clear_at && s.tick >= s.tank.clear_cooldown_end {
            self.cooldown = 30;
            return Input::Clear;
        }

        // Purist / Jack-of-All: grab a wanted weapon; if none is offered, reroll
        // to fish for one rather than spending the slot on something off-target.
        // (Correctness no longer rides on this — the director/client buy-filter
        // drops any disallowed purchase authoritatively — but fishing lets the
        // bot acquire its class quickly so the achievement actually lands.)
        if matches!(self.challenge, Challenge::Purist(_) | Challenge::JackOfAll) {
            if let Some(slot) = self.wanted_weapon(s) {
                self.cooldown = 4;
                return Input::BuyOffer { slot: slot as u8 };
            }
            let jack_complete =
                self.challenge == Challenge::JackOfAll && s.bought_attack_mask == ALL_CLASSES;
            if !jack_complete && self.can_reroll(s) {
                self.cooldown = 8;
                return Input::Reroll;
            }
        }

        // Otherwise buy the best affordable, challenge-allowed offer. Prefer a
        // weapon while under the target arsenal size; among same-kind, cheapest.
        let want_weapon = s.weapons.len() < self.target_weapons;
        let mut best: Option<(usize, i64, bool)> = None; // (slot, cost, is_weapon)
        for (i, off) in s.shop.offers.iter().enumerate() {
            if off.cost > s.economy.gold || !self.allowed(off) {
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

        // Nothing affordable/allowed — spend a free reroll to fish for options.
        if s.economy.rerolls_remaining > 0 {
            self.cooldown = 20;
            return Input::Reroll;
        }
        Input::Noop
    }
}

/// Mask of all six attack-classes (JackOfAll completion target).
const ALL_CLASSES: u16 = 0b111111;
