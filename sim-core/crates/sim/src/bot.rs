//! A deterministic auto-player so previews and the spectator/net demo can play
//! themselves. It reads only the public [`ArenaState`] (the same surface a real
//! client/UI sees) and returns one [`Input`] per tick — proving the sim is
//! drivable through its render-facing API. No RNG, no wall-clock: purely a
//! function of observed state, so it never threatens determinism.

use crate::content::ModEffect;
use crate::{content, ArenaState, Input, Offer, OfferKind};

/// Whether buying `off` would strictly hurt *this* tank with no upside the bot's
/// policy can spend — i.e. a modifier that debuffs the owner's own survivability
/// (cuts Max HP, or pushes HP regen toward a per-tick drain) in exchange for a
/// gold windfall the greedy bot never converts back into survival. Detected from
/// the effect *category*, not item names: any `Trade*ForGold` self-debuff counts,
/// so a future trap of the same shape is declined too. Humans still see and may
/// pick these as high-risk gambles — only the BOT auto-declines them, since
/// blindly buying them is the self-kill that wrecked the survival-curve proxy.
///
/// These trades are net-negative to the bot specifically because its buy logic
/// has no path that uses the gained gold to repair the lost HP/regen in time, so
/// the trade only ever hurts the one axis the bot is graded on: staying alive.
fn is_self_harm_trade(off: &Offer) -> bool {
    if !matches!(off.kind, OfferKind::Modifier) {
        return false;
    }
    // Scan the def's effects: a trade is self-harm if ANY effect drains the
    // axis the bot is graded on. (Each catalog entry carries one effect today,
    // so `any` reproduces the prior single-effect decision exactly.)
    content::MODIFIERS[off.def as usize].effects.iter().any(|e| match e {
        // Reduces Max HP for gold — a smaller HP pool the bot never offsets.
        ModEffect::TradeMaxHpForGold(hp_cost, _) => *hp_cost > 0,
        // Reduces HP regen for gold (may go negative → a drain) — self-damage.
        ModEffect::TradeRegenForGold(regen_cost, _) => *regen_cost > 0,
        _ => false,
    })
}

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
    /// MEASUREMENT ONLY (not a shipped achievement): a PURE-ECONOMY player — buys
    /// ONLY economy/gold offers, NEVER weapons or any other modifier. Used to
    /// verify the early-game punish (an unarmed tank relying solely on the starting
    /// Bow + Clear should usually die inside the first 5 min). Exposed via an
    /// explicit code (`9`) so the DEFAULT bot never adopts it.
    EcoOnly,
}

impl Challenge {
    /// Map the GDScript-facing integer code to a challenge. 1..=6 → Purist of
    /// attack-class 0..5, 7 → NoEconomy, 8 → JackOfAll, 9 → EcoOnly (measurement),
    /// anything else → None.
    pub fn from_code(code: i64) -> Challenge {
        match code {
            1..=6 => Challenge::Purist((code - 1) as u8),
            7 => Challenge::NoEconomy,
            8 => Challenge::JackOfAll,
            9 => Challenge::EcoOnly,
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
                !content::MODIFIERS[off.def as usize].is_economy()
            }
            // Pure-economy: forbid ALL weapons and every non-economy modifier; the
            // only permitted purchase is an economy/gold modifier.
            (Challenge::EcoOnly, OfferKind::Weapon) => false,
            (Challenge::EcoOnly, OfferKind::Modifier) => {
                content::MODIFIERS[off.def as usize].is_economy()
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

    /// Whether the active challenge permits buying this offer *and* the offer is
    /// not a strictly self-harmful trade the bot would die to. This is the single
    /// chokepoint every buy path funnels through, so no selection route (weapon
    /// floor, economy, or the generic best-affordable scan) can auto-pick a trap.
    fn allowed(&self, off: &Offer) -> bool {
        self.challenge.permits(off) && !is_self_harm_trade(off)
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

    /// Cheapest affordable slot whose offer matches `pred`, if any.
    fn cheapest_where(&self, s: &ArenaState, pred: impl Fn(&Offer) -> bool) -> Option<usize> {
        let mut best: Option<(usize, i64)> = None;
        for (i, off) in s.shop.offers.iter().enumerate() {
            if off.cost > s.economy.gold || !pred(off) {
                continue;
            }
            if best.map_or(true, |(_, c)| off.cost < c) {
                best = Some((i, off.cost));
            }
        }
        best.map(|(i, _)| i)
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

        // Default survivor minmaxes income: hold a defensive weapon floor that
        // grows with the round, then snowball income while early rounds still
        // pay it back (compounding) — before pivoting to raw power below. The
        // floor keeps it from going all-economy and dying.
        if self.challenge == Challenge::None {
            let round = if s.round == u32::MAX { 0 } else { s.round };
            let floor = (SURVIVAL_FLOOR + round as usize).min(WEAPON_FLOOR_CAP);
            if s.weapons.len() < floor {
                if let Some(slot) = self.cheapest_where(s, |o| matches!(o.kind, OfferKind::Weapon)) {
                    self.cooldown = 6;
                    return Input::BuyOffer { slot: slot as u8 };
                }
            } else if round <= ECON_LAST_ROUND {
                if let Some(slot) = self.cheapest_where(s, |o| {
                    matches!(o.kind, OfferKind::Modifier)
                        && content::MODIFIERS[o.def as usize].is_economy()
                }) {
                    self.cooldown = 6;
                    return Input::BuyOffer { slot: slot as u8 };
                }
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
/// Default bot's base defensive weapon count before it invests in income.
const SURVIVAL_FLOOR: usize = 3;
/// The weapon floor grows with the round but never exceeds this.
const WEAPON_FLOOR_CAP: usize = 10;
/// Income only compounds with rounds left to run — stop investing after this.
const ECON_LAST_ROUND: u32 = 6;
