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

/// One of four build IDENTITIES the default bot adopts per match. The archetype
/// is chosen DETERMINISTICALLY from the match seed (see [`Archetype::for_seed`])
/// and biases what the bot buys — so across a seed sweep you get a real spread of
/// builds (glass-cannon, tanky, eco-pivot, balanced) instead of one greedy line.
/// It is a PURCHASE BIAS only: it never bends the sim, reads no wall-clock, and
/// consumes no sim RNG stream (it hashes the public `master_seed` locally), so it
/// cannot threaten determinism.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Archetype {
    /// Weapons + damage; thin defense. Wins by out-DPSing the board.
    GlassCannon,
    /// HP / armor / regen / shield; a small weapon floor. Wins by out-lasting.
    Tanky,
    /// Economy first (snowball gold early), then pivot hard into weapons.
    EcoPivot,
    /// The original mixed survivor line: weapon floor → income → power.
    Balanced,
}

impl Archetype {
    /// Deterministic per-match pick from the public match seed. A self-contained
    /// SplitMix64 finalizer on `master_seed` (NOT a sim RNG stream, so the sim's
    /// checksum is untouched) → one of four archetypes, uniformly. Same seed always
    /// yields the same archetype, on every machine.
    pub fn for_seed(master_seed: u64) -> Archetype {
        // SplitMix64 finalizer — pure integer mixing, no floats, platform-stable.
        let mut z = master_seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        match z % 4 {
            0 => Archetype::GlassCannon,
            1 => Archetype::Tanky,
            2 => Archetype::EcoPivot,
            _ => Archetype::Balanced,
        }
    }

    /// The weapon-floor size this archetype holds before it diversifies into
    /// modifiers — glass-cannon stacks many weapons; tanky keeps a lean floor.
    fn weapon_floor(self, round: u32) -> usize {
        let base = match self {
            Archetype::GlassCannon => 6,
            Archetype::Tanky => 2,
            Archetype::EcoPivot => 1,
            Archetype::Balanced => 3,
        };
        (base + round as usize).min(WEAPON_FLOOR_CAP)
    }

    /// Total weapons this archetype wants to own (the cap that stops it preferring
    /// weapons over modifiers in the generic buy).
    fn target_weapons(self) -> usize {
        match self {
            Archetype::GlassCannon => 20,
            Archetype::Tanky => 8,
            Archetype::EcoPivot => 14,
            Archetype::Balanced => 14,
        }
    }

    /// Last round in which this archetype still pours spare gold into ECONOMY
    /// (income compounds only while rounds remain to pay it back). Eco-pivot
    /// invests longest; glass-cannon barely at all.
    fn econ_last_round(self) -> u32 {
        match self {
            Archetype::GlassCannon => 2,
            Archetype::Tanky => 4,
            Archetype::EcoPivot => 5,
            Archetype::Balanced => 4,
        }
    }

    /// Per-axis buy WEIGHTS (offense, defense), out of their sum. Every archetype
    /// keeps SOME defense (a tank with no HP/armor is one-shot by the scaling
    /// contact damage), but the ratio is its identity: glass-cannon leans offense,
    /// tanky leans defense, eco/balanced split evenly. The buy loop keeps the actual
    /// offense:defense purchase counts near this ratio, so the build genuinely
    /// reflects the archetype rather than collapsing to one axis.
    fn axis_weights(self) -> (u32, u32) {
        match self {
            Archetype::GlassCannon => (7, 3),
            Archetype::Tanky => (3, 7),
            Archetype::EcoPivot => (5, 5),
            Archetype::Balanced => (5, 5),
        }
    }
}

/// Classify a modifier offer into the axis an archetype cares about. Detected from
/// the effect CATEGORY (not item names), so new items of the same shape sort
/// correctly. A modifier counts as defensive if ANY effect raises survivability,
/// offensive if ANY raises damage, economic via the existing `is_economy`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModAxis {
    Offense,
    Defense,
    Economy,
    Other,
}

fn mod_axis(def: u16) -> ModAxis {
    use ModEffect::*;
    let m = &content::MODIFIERS[def as usize];
    if m.is_economy() {
        return ModAxis::Economy;
    }
    let mut offense = false;
    let mut defense = false;
    for e in m.effects.iter() {
        match e {
            // STATIC offense — plain additive/multiplicative damage the bot can
            // stack safely. The DYNAMIC stat-scaling damage scalers
            // (`DamagePerMaxHp` / `DamagePerBountyPct` / `ShieldActiveDamagePct` /
            // `DamagePerWeapon`) are deliberately NOT treated as preferred offense:
            // they multiply live tank/economy stats, so stacking them on a
            // snowballing build inflates damage past the Fixed range. Leaving them
            // as "Other" keeps the bot from compounding them into an overflow while
            // it still buys plain damage for its offense identity.
            DamageGlobalPct(..) | DamageTypePct(..) | DamageMulPct(..) | AttackSpeedPct(..)
            | DamageScopePct(..) | DamageVsStunnedPct(..) | DamageVsPoisonedPct(..)
            | PoisonDamagePct(..) | StunDurationPct(..) | SpikesFlat(..) | SpikesPct(..)
            | GrantVulnPulse(..) => offense = true,
            MaxHp(..) | MaxHpPct(..) | Armor(..) | ManaShield(..) | HpRegen(..)
            | HpRegenPct(..) | ManaRegenPct(..) | Dodge(..) | HealOnKill(..)
            | HealOnPoison(..) | HealOnDamaged(..) | HealingPct(..) | MissingHpHealPct(..)
            | GrantRevive(..) | ShieldActiveDrPct(..) | IncomeShieldPct(..) | ManaOnKill(..) => {
                defense = true
            }
            _ => {}
        }
    }
    match (offense, defense) {
        (true, _) => ModAxis::Offense,
        (false, true) => ModAxis::Defense,
        _ => ModAxis::Other,
    }
}

/// Whether a modifier raises the tank's MAX HP (flat or %). The only defensive stat
/// that scales the HP pool to absorb the scaling contact damage — the bot prioritizes
/// it within the defense axis so it isn't one-shot late.
fn raises_max_hp(def: u16) -> bool {
    use ModEffect::*;
    content::MODIFIERS[def as usize]
        .effects
        .iter()
        .any(|e| matches!(e, MaxHp(..) | MaxHpPct(..)))
}

/// The opposite combat axis (offense ↔ defense); identity for non-combat axes.
fn other_axis(a: ModAxis) -> ModAxis {
    match a {
        ModAxis::Offense => ModAxis::Defense,
        ModAxis::Defense => ModAxis::Offense,
        x => x,
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
    /// The build identity for this match, chosen deterministically from the seed
    /// on the first `decide` (cached). `None` until then.
    archetype: Option<Archetype>,
    /// MODIFIER purchases this bot has issued (weapons excluded). Bounded by
    /// `MODIFIER_BUY_CAP` so the bot can't stack hundreds of stat upgrades into a
    /// fixed-point overflow — a real build is finite.
    mods_bought: u32,
    /// OFFENSE-axis modifier purchases (subset of `mods_bought`), tracked so the buy
    /// loop can hold the archetype's offense:defense ratio.
    offense_bought: u32,
    /// DEFENSE-axis modifier purchases (subset of `mods_bought`).
    defense_bought: u32,
}

impl Default for Bot {
    fn default() -> Self {
        Bot {
            cooldown: 0,
            target_weapons: 12,
            challenge: Challenge::None,
            archetype: None,
            mods_bought: 0,
            offense_bought: 0,
            defense_bought: 0,
        }
    }
}

impl Bot {
    /// A bot that honors `c` while buying (otherwise identical to `default`).
    pub fn with_challenge(c: Challenge) -> Self {
        Bot { challenge: c, ..Bot::default() }
    }

    /// This match's archetype, resolved from the seed on first use and cached.
    fn archetype(&mut self, s: &ArenaState) -> Archetype {
        *self.archetype.get_or_insert_with(|| Archetype::for_seed(s.master_seed))
    }

    /// Whether the bot may still buy MODIFIERS (it stops once it has stacked
    /// `MODIFIER_BUY_CAP` of them — a finite, realistic build that can't compound
    /// into a fixed-point overflow). Weapons are unaffected.
    fn can_buy_mod(&self) -> bool {
        self.mods_bought < MODIFIER_BUY_CAP
    }

    /// Commit a MODIFIER purchase at `slot` (of known `axis`): record it against the
    /// cap and the per-axis counts, then set the post-buy think delay. The single
    /// funnel for every modifier buy path.
    fn buy_mod(&mut self, slot: usize, axis: ModAxis) -> Input {
        self.mods_bought += 1;
        match axis {
            ModAxis::Offense => self.offense_bought += 1,
            ModAxis::Defense => self.defense_bought += 1,
            _ => {}
        }
        self.cooldown = 6;
        Input::BuyOffer { slot: slot as u8 }
    }

    /// Which combat axis to buy next so the offense:defense purchase ratio tracks the
    /// archetype's `axis_weights`. Picks whichever axis is most "behind" its target
    /// share (defense first on ties, so a fresh build always grabs survival early).
    fn next_axis(&self, arch: Archetype) -> ModAxis {
        let (wo, wd) = arch.axis_weights();
        let off = self.offense_bought as u64;
        let def = self.defense_bought as u64;
        // Compare cross-multiplied shares: offense is behind iff off/wo < def/wd.
        if off * wd as u64 <= def * wo as u64 {
            ModAxis::Offense
        } else {
            ModAxis::Defense
        }
    }

    /// Whether the bot may still buy WEAPONS. A real arsenal is finite; without this
    /// the bot dumps every spare coin into weapons for the whole match (thousands of
    /// instances). Capping keeps builds realistic and the per-tick fire loop bounded.
    fn can_buy_weapon(&self, s: &ArenaState) -> bool {
        s.weapons.len() < WEAPON_BUY_CAP
    }

    /// Most EXPENSIVE affordable offer matching `pred` (the bot snowballs huge gold,
    /// so it should buy the most impactful item it can, not the cheapest filler).
    fn priciest_where(&self, s: &ArenaState, pred: impl Fn(&Offer) -> bool) -> Option<usize> {
        let mut best: Option<(usize, i64)> = None;
        for (i, off) in s.shop.offers.iter().enumerate() {
            if off.cost > s.economy.gold || !self.allowed(off) || !pred(off) {
                continue;
            }
            if best.map_or(true, |(_, c)| off.cost > c) {
                best = Some((i, off.cost));
            }
        }
        best.map(|(i, _)| i)
    }

    fn weapon_class(def: u16) -> u8 {
        content::attack_scope_id(content::WEAPONS[def as usize].attack)
    }

    /// Whether the active challenge permits buying this offer *and* the offer is
    /// not a strictly self-harmful trade the bot would die to *and* it is not an
    /// overflow-prone dynamic damage scaler. This is the single chokepoint every
    /// buy path funnels through, so no selection route (weapon floor, economy,
    /// preferred axis, or the generic best-affordable scan) can auto-pick a trap.
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

        // Default survivor: spend along this match's ARCHETYPE (chosen deterministically
        // from the seed), so the 80-seed sweep shows a real spread of builds. Order:
        //   (1) hold the archetype's weapon floor,
        //   (2) snowball ECONOMY while its window is open,
        //   (3) keep the arsenal topped up toward the archetype's weapon target,
        //   (4) buy the next COMBAT-AXIS modifier (offense/defense) that keeps the
        //       build near the archetype's offense:defense ratio — taking the most
        //       impactful (priciest) affordable item, since the bot snowballs gold.
        // Falls through to the generic best-affordable scan only if nothing above
        // fits (e.g. a shop with no matching axis offer).
        let mut arch_target_weapons = self.target_weapons;
        if self.challenge == Challenge::None {
            let arch = self.archetype(s);
            arch_target_weapons = arch.target_weapons();
            let round = if s.round == u32::MAX { 0 } else { s.round };
            let floor = arch.weapon_floor(round);
            // (1) Weapon floor.
            if s.weapons.len() < floor {
                if let Some(slot) = self.cheapest_where(s, |o| matches!(o.kind, OfferKind::Weapon)) {
                    self.cooldown = 6;
                    return Input::BuyOffer { slot: slot as u8 };
                }
            }
            // (2) Economy window: snowball income while rounds remain to pay it back.
            if self.can_buy_mod() && round <= arch.econ_last_round() {
                if let Some(slot) = self.priciest_where(s, |o| {
                    matches!(o.kind, OfferKind::Modifier)
                        && content::MODIFIERS[o.def as usize].is_economy()
                }) {
                    return self.buy_mod(slot, ModAxis::Economy);
                }
            }
            // (3) Keep the arsenal topped up toward the archetype's weapon target.
            if self.can_buy_weapon(s) && s.weapons.len() < arch_target_weapons {
                if let Some(slot) = self.cheapest_where(s, |o| matches!(o.kind, OfferKind::Weapon)) {
                    self.cooldown = 6;
                    return Input::BuyOffer { slot: slot as u8 };
                }
            }
            // (4) Combat-axis modifier, holding the archetype's offense:defense ratio.
            if self.can_buy_mod() {
                let want = self.next_axis(arch);
                // Survival floor: the scaling contact damage one-shots a flat-HP tank,
                // so when buying DEFENSE the bot prefers a MAX-HP item (the only stat
                // that scales the pool to absorb a leak) over flat armor/regen.
                if want == ModAxis::Defense {
                    if let Some(slot) = self.priciest_where(s, |o| {
                        matches!(o.kind, OfferKind::Modifier) && raises_max_hp(o.def)
                    }) {
                        return self.buy_mod(slot, ModAxis::Defense);
                    }
                }
                // Try the wanted axis first (priciest = most impactful), then the other
                // axis as a fallback so a shop missing the wanted axis still progresses.
                for axis in [want, other_axis(want)] {
                    if let Some(slot) =
                        self.priciest_where(s, |o| matches!(o.kind, OfferKind::Modifier) && mod_axis(o.def) == axis)
                    {
                        return self.buy_mod(slot, axis);
                    }
                }
            }
        }

        // Generic fallback: buy the best affordable, challenge-allowed offer. Prefer a
        // weapon while under the target arsenal size; among same-kind, cheapest. For
        // the default bot, weapons stop at the ARCHETYPE target (not the hard cap), so
        // a tanky build keeps a lean arsenal and a glass-cannon a fat one — i.e. the
        // archetype's weapon count is preserved instead of every build draining its
        // leftover gold into the same 40-weapon ceiling. (`WEAPON_BUY_CAP` remains the
        // absolute safety ceiling for challenge runs / edge cases.)
        let want_weapon = s.weapons.len() < arch_target_weapons;
        let mods_ok = self.can_buy_mod() || self.challenge != Challenge::None;
        let weapons_ok = if self.challenge == Challenge::None {
            s.weapons.len() < arch_target_weapons
        } else {
            self.can_buy_weapon(s)
        };
        let mut best: Option<(usize, i64, bool)> = None; // (slot, cost, is_weapon)
        for (i, off) in s.shop.offers.iter().enumerate() {
            let is_weapon = matches!(off.kind, OfferKind::Weapon);
            let kind_ok = if is_weapon { weapons_ok } else { mods_ok };
            if off.cost > s.economy.gold || !self.allowed(off) || !kind_ok {
                continue;
            }
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
        if let Some((slot, _, is_weapon)) = best {
            if is_weapon {
                self.cooldown = 6;
                return Input::BuyOffer { slot: slot as u8 };
            }
            return self.buy_mod(slot, mod_axis(s.shop.offers[slot].def));
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
/// The archetype weapon floor grows with the round but never exceeds this.
const WEAPON_FLOOR_CAP: usize = 10;
/// Hard cap on MODIFIER purchases per match for the default bot. A real build is
/// finite; without a cap the bot would buy a modifier every few ticks for the whole
/// 30-min match (~thousands), compounding multiplicative damage / economy into a
/// fixed-point overflow. 120 is far more than any human buys yet bounded enough that
/// no stat runs away (with the saturating Fixed ops as a final backstop). Challenge
/// runs are exempt (they self-limit by playstyle).
const MODIFIER_BUY_CAP: u32 = 250;
/// Hard cap on WEAPON instances for the default bot — a realistic arsenal. Without
/// it, once the modifier cap is reached the bot dumps all remaining gold into
/// weapons for the rest of the match (thousands of instances). Challenge runs are
/// exempt.
const WEAPON_BUY_CAP: usize = 40;
