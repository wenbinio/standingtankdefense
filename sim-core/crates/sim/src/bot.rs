//! A deterministic auto-player so previews and the spectator/net demo can play
//! themselves. It reads only the public [`ArenaState`] (the same surface a real
//! client/UI sees) and returns one [`Input`] per tick — proving the sim is
//! drivable through its render-facing API. No RNG, no wall-clock: purely a
//! function of observed state, so it never threatens determinism.

use crate::content::{Attack, ModEffect, WeaponAbility, WeaponDef};
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

// ───────────────────────────── weapon valuation ─────────────────────────────
//
// The bot used to choose weapons with `cheapest_where(kind == Weapon)`. Because
// a weapon's `cost` is a pure function of its RARITY (500 / 1500 / 3000 / 5000),
// "cheapest" meant "lowest-rarity offer, ties to the lowest slot index" — the
// bot never looked at damage, cooldown, attack class or what it already owned.
// That made it blind on the single most important decision in the game, so every
// build-shape number measured through it described the PRICE TABLE rather than
// the game. Everything below replaces that with a value estimate.
//
// The model is deliberately a competent player's heuristic, not an optimizer:
//   value(offer) = own marginal power (with the synergy it would then enjoy)
//                + the synergy uplift it grants the arsenal ALREADY owned
// ranked against price with a gold-dependent weight. All integer, no RNG, no
// wall-clock, stable slot iteration order — it cannot perturb determinism.

/// Estimated number of enemies one volley connects with, in TENTHS (10 == one
/// enemy). This is the single biggest thing price hides: a `Barrage(8)` and a
/// `SingleTarget` of equal damage-per-second are not equally good, and an
/// `Area`/`Wave` weapon that sweeps the pack camped on the tank is worth several
/// single-target weapons during the swarm phases the run actually dies to.
///
/// The numbers are a player's rule of thumb ("this one hits the whole pack"),
/// not a simulation: multi-target classes are credited generously but capped, so
/// a 12-projectile barrage cannot claim twelve simultaneous targets that rarely
/// all exist.
fn est_targets10(w: &WeaponDef) -> i64 {
    match w.attack {
        Attack::SingleTarget => 10,
        // Splash: the primary target plus whatever the radius catches.
        Attack::Splash(r) => 10 + (r / 50).clamp(0, 20),
        // Barrage/Bounce hit N DISTINCT enemies — the strongest honest multiplier
        // in the catalog, capped because N enemies must actually be in range.
        Attack::Barrage(n) => (10 * n as i64).min(MAX_TARGETS10),
        Attack::Bounce(n) => (9 * n as i64).min(MAX_TARGETS10),
        // Instant pulses centred on the tank: they hit exactly the enemies that
        // are about to deal contact damage, which is what kills the tank.
        Attack::Area(r) => (15 + r / 20).min(MAX_TARGETS10),
        Attack::Wave(extra) => (15 + (w.range + extra) / 25).min(MAX_TARGETS10),
    }
}

/// Percentage uplift for everything a weapon does BESIDES its listed damage:
/// damage-over-time, crowd control, and its signature ability. Expressed as a
/// percent of the weapon's base power so it scales with the weapon rather than
/// swamping it. Bounded per-source so no single flag can dominate the ranking.
fn utility_pct(w: &WeaponDef) -> i64 {
    let mut pct = 0i64;
    let oh = &w.on_hit;
    // Poison / fire keep dealing damage between shots.
    if oh.poison_dps > 0 {
        pct += (oh.poison_dps * 2).min(40);
    }
    if oh.fire_stacks > 0 {
        pct += (oh.fire_stacks as i64 / 4).min(30);
    }
    // Frost slows the pack; stun removes attackers outright. Both convert into
    // contact damage the tank never takes.
    if oh.frost_stacks > 0 {
        pct += (oh.frost_stacks as i64 * 2).min(15);
    }
    if oh.stun_ticks > 0 {
        pct += (oh.stun_ticks as i64 / 3).min(30);
    }
    pct += match w.ability {
        WeaponAbility::None => 0,
        // Sustain: healing per enemy hit is survival, which is what the run is
        // graded on.
        WeaponAbility::LifeDrain { .. } => 25,
        WeaponAbility::ManaDrain { .. } => 10,
        WeaponAbility::Knockback { .. } => 12,
        WeaponAbility::Root { .. } => 25,
        // Vulnerability amplifies the WHOLE arsenal, not just this weapon.
        WeaponAbility::VulnOnHit { stacks } => (stacks as i64 * 2).min(20),
        WeaponAbility::Hazard { .. } => 20,
        WeaponAbility::Summon { .. } => 20,
    };
    pct
}

/// Percentage adjustment for reach. A long-range projectile weapon starts hurting
/// the wave earlier and keeps firing while the tank is not yet swarmed; a
/// 300-range single-target weapon only fires once things are already on top of
/// you. Does not apply to `Area`/`Wave`, whose short listed range is inherent to
/// the class and already priced into [`est_targets10`].
fn range_pct(w: &WeaponDef) -> i64 {
    match w.attack {
        Attack::Area(_) | Attack::Wave(_) => 0,
        _ => ((w.range - 600) / 30).clamp(-15, 20),
    }
}

/// Standalone power estimate for one instance of `def`, ignoring synergy — the
/// weapon's damage-per-second times how many enemies it lands on, adjusted for
/// utility and reach. `utility_weight_pct` lets an archetype care more or less
/// about the non-damage half (see [`WeaponBias`]).
fn weapon_power(def: u16, utility_weight_pct: i64) -> i64 {
    let w = &content::WEAPONS[def as usize];
    // Tenths of damage-per-second: damage * TICK_HZ / cooldown, ×10.
    let dps10 = w.damage * 300 / (w.cooldown_ticks.max(1) as i64);
    let base = dps10 * est_targets10(w) / 10;
    let pct = (100 + utility_pct(w) * utility_weight_pct / 100 + range_pct(w)).max(20);
    (base * pct / 100).max(1)
}

/// The breadth incentive, mirrored from `Modifiers::arsenal_synergy_add`: +10%
/// per damage type beyond the first (cap +40%), +8% per attack class beyond the
/// first (cap +40%), +5% per distinct weapon def beyond the first (cap +40%),
/// total capped at +120%. Copies are neither rewarded nor taxed.
///
/// This is a MODEL of a rule the sim owns, kept here as constants so the bot
/// compiles against any revision of `modifiers.rs` (which this agent does not
/// own). If the shipped magnitudes change, these four constants must follow —
/// the bot would otherwise be optimizing a game it is no longer playing.
fn arsenal_synergy_pct(damage_types: u32, attack_classes: u32, distinct_defs: u32) -> i64 {
    let axis = |n: u32, per: i64| (n.saturating_sub(1) as i64 * per).min(SYN_AXIS_CAP_PCT);
    (axis(damage_types, SYN_PER_DAMAGE_TYPE_PCT)
        + axis(attack_classes, SYN_PER_ATTACK_CLASS_PCT)
        + axis(distinct_defs, SYN_PER_DISTINCT_DEF_PCT))
    .min(SYN_TOTAL_CAP_PCT)
}

/// What the bot currently owns, summarized once per decision so each offer can be
/// priced against it in O(1). Built by scanning `s.weapons` in index order.
struct Arsenal {
    /// Bit per damage type present.
    type_mask: u32,
    /// Bit per attack class (`content::attack_scope_id`) present.
    class_mask: u32,
    damage_types: u32,
    attack_classes: u32,
    distinct_defs: u32,
    /// Sum of the SATURATED power of every owned instance — the base that a synergy
    /// uplift would multiply.
    total_power: i64,
    /// `(def, copies owned)` in first-seen order. Small (≤ the weapon cap), so a
    /// linear scan beats any bitset over a catalog whose size is content-owned and
    /// free to grow.
    defs: Vec<(u16, i64)>,
}

impl Arsenal {
    fn scan(s: &ArenaState, bias: WeaponBias) -> Arsenal {
        let mut a = Arsenal {
            type_mask: 0,
            class_mask: 0,
            damage_types: 0,
            attack_classes: 0,
            distinct_defs: 0,
            total_power: 0,
            defs: Vec::with_capacity(s.weapons.len()),
        };
        for inst in s.weapons.iter() {
            let w = &content::WEAPONS[inst.def as usize];
            a.type_mask |= 1u32 << w.damage_type;
            a.class_mask |= 1u32 << content::attack_scope_id(w.attack);
            match a.defs.iter_mut().find(|(d, _)| *d == inst.def) {
                Some((_, n)) => *n += 1,
                None => a.defs.push((inst.def, 1)),
            }
        }
        for &(def, n) in a.defs.iter() {
            let unit = weapon_power(def, bias.utility_weight_pct);
            for i in 0..n {
                a.total_power += saturated_copy_power(unit, i, bias.copy_saturation);
            }
        }
        a.damage_types = a.type_mask.count_ones();
        a.attack_classes = a.class_mask.count_ones();
        a.distinct_defs = a.defs.len() as u32;
        a
    }

    fn copies_of(&self, def: u16) -> i64 {
        self.defs.iter().find(|(d, _)| *d == def).map_or(0, |(_, n)| *n)
    }
}

/// What the `n`-th EXTRA copy of a weapon is actually worth, given `unit` is what
/// the first one is worth: `unit × SAT / (SAT + n)`.
///
/// This is target saturation, and it is the difference between a bot that can
/// weigh "another copy" against "something new" and one that cannot. Identical
/// weapons share a damage type, a range, a cooldown and a target pool, so the
/// tenth copy spends much of its output overkilling what the first nine already
/// killed, while a different weapon reaches targets, ranges and armor classes the
/// stack does not. A player estimates this instinctively ("I have enough of
/// those"); the bot needs it stated.
///
/// It is deliberately an ESTIMATE and not a claim about the rules: the sim does
/// not tax copies, and neither does the breadth synergy. Without it the stack/spread
/// choice is a cliff — copies score identically forever — and whichever side of the
/// cliff the bot lands on is an artifact of the shop it happened to be rich in
/// rather than of what the weapons are worth.
fn saturated_copy_power(unit: i64, already_owned: i64, saturation: i64) -> i64 {
    let sat = saturation.max(1);
    (unit * sat / (sat + already_owned.max(0))).max(1)
}

/// How one archetype reads a weapon. These knobs are what keep the four build
/// identities apart now that all of them can see value: without them a value-aware
/// bot converges on one "best" line and the strategy-diversity measurement dies.
#[derive(Clone, Copy)]
struct WeaponBias {
    /// How much of the breadth synergy this archetype believes in. A glass-cannon
    /// discounts it (it wants the single biggest hitter stacked); an eco-pivot,
    /// which buys late with a full wallet, leans into it.
    synergy_weight_pct: i64,
    /// How much the non-damage half (crowd control, sustain, DoT) counts. Tanky
    /// builds pay up for stuns, slows and life-drain; glass-cannons want numbers.
    utility_weight_pct: i64,
    /// Price sensitivity: the gold-on-hand reference is `gold / price_divisor`, so
    /// a LARGER divisor means a smaller reference, which means price keeps
    /// mattering for longer (see [`Bot::price_ref`]).
    price_divisor: i64,
    /// Save-up discipline: decline to buy when the best AFFORDABLE weapon is worth
    /// less than this percentage of the best weapon on the board (see
    /// [`Bot::best_weapon`]). Higher = more willing to sit on gold and wait for
    /// something good.
    patience_pct: i64,
    /// Copy tolerance: the saturation constant in [`saturated_copy_power`]. This is
    /// the axis that actually separates a STACKER from a COVERAGE build, so it is
    /// per-archetype rather than global. Larger = flatter = happier to buy the
    /// eleventh copy of its best weapon.
    copy_saturation: i64,
}

impl Archetype {
    fn weapon_bias(self) -> WeaponBias {
        match self {
            // Stack the biggest hitter; breadth is a distraction, utility is fluff,
            // and it will hold out (and overpay) for a top-end weapon.
            Archetype::GlassCannon => WeaponBias {
                synergy_weight_pct: 60,
                utility_weight_pct: 50,
                price_divisor: 2,
                patience_pct: 55,
                copy_saturation: 30,
            },
            // Buys crowd control and sustain: damage it never takes is damage it
            // never has to out-heal. Wants boards filled sooner, so less patient.
            Archetype::Tanky => WeaponBias {
                synergy_weight_pct: 100,
                utility_weight_pct: 175,
                price_divisor: 4,
                patience_pct: 30,
                copy_saturation: 5,
            },
            // The value shopper: most price-sensitive, keenest on breadth (its whole
            // plan is to convert a big wallet into a broad board), and content to
            // sit on gold while the wallet does the work.
            Archetype::EcoPivot => WeaponBias {
                synergy_weight_pct: 130,
                utility_weight_pct: 100,
                price_divisor: 8,
                patience_pct: 45,
                copy_saturation: 9,
            },
            Archetype::Balanced => WeaponBias {
                synergy_weight_pct: 100,
                utility_weight_pct: 100,
                price_divisor: 4,
                patience_pct: 40,
                copy_saturation: 8,
            },
        }
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

    /// Best affordable slot holding a weapon this challenge wants, if any. For
    /// Purist that's the target class; for JackOfAll an attack-class we don't own
    /// yet; otherwise None (generic buying handles the default bot). Within the
    /// wanted set the pick is by VALUE-for-money, same as everywhere else — a
    /// challenge constrains WHICH weapons are legal, it is not a reason to then
    /// take the worst of them.
    fn wanted_weapon(&self, s: &ArenaState) -> Option<usize> {
        let challenge = self.challenge;
        // Patience 0: a challenge run's job is to ACQUIRE its class (that is what the
        // achievement checks), so it takes what it can get rather than saving.
        self.best_weapon(s, self.weapon_bias(), 0, |off| match challenge {
            Challenge::Purist(class) => Self::weapon_class(off.def) == class,
            Challenge::JackOfAll => {
                s.bought_attack_mask & (1u16 << Self::weapon_class(off.def)) == 0
            }
            _ => false,
        })
    }

    /// Can we reroll to fish for a wanted weapon (free, or affordable paid)?
    fn can_reroll(&self, s: &ArenaState) -> bool {
        s.economy.rerolls_remaining > 0 || s.economy.gold >= s.economy.reroll_cost
    }

    /// The gold-on-hand reference used to trade value off against price. Ranking
    /// is by `value / (cost + ref)`:
    ///   * `ref ≈ 0` (broke) → the ranking is value-PER-GOLD, so a 500g weapon
    ///     that is half as good as a 5000g one wins, which is correct when the
    ///     next purchase is the only one you can afford;
    ///   * `ref` large (rich) → `cost` is noise beside it and the ranking becomes
    ///     value outright, which is also correct: when gold has stopped being the
    ///     constraint, buy the best thing on the board.
    ///
    /// Capped so the comparison stays in a small integer range once the bot's
    /// economy snowballs (gold reaches the billions late).
    fn price_ref(&self, s: &ArenaState, bias: WeaponBias) -> i64 {
        (s.economy.gold / bias.price_divisor.max(1)).clamp(0, PRICE_REF_CAP)
    }

    /// The valuation, for one offer, against an already-summarized arsenal:
    ///
    ///   value = own_power × (1 + synergy_after)          ← what it adds itself
    ///         + owned_power × Δsynergy                   ← what it adds to the rest
    ///
    /// The second term is the whole point of the breadth incentive and the thing
    /// the price-first policy could not see. Worked example with the shipped
    /// magnitudes: holding 14 copies of one def, a new DISTINCT def moves the
    /// synergy by +5%, worth `0.05 × 14p = 0.7p` on the existing stack alone — so a
    /// new weapon only needs ~30% of a stacked weapon's raw power to be the better
    /// buy, and if it also brings a new damage type or attack class (+10% / +8%)
    /// almost nothing beats it. Once every axis is capped (+120%) Δ falls to zero
    /// and the bot correctly goes back to stacking its strongest weapon.
    fn weapon_value(&self, ars: &Arsenal, bias: WeaponBias, def: u16) -> i64 {
        let w = &content::WEAPONS[def as usize];
        let copies = ars.copies_of(def);
        // What THIS copy adds, not what the first one was worth.
        let own =
            saturated_copy_power(weapon_power(def, bias.utility_weight_pct), copies, bias.copy_saturation);

        let adds_type = ars.type_mask & (1u32 << w.damage_type) == 0;
        let adds_class = ars.class_mask & (1u32 << content::attack_scope_id(w.attack)) == 0;
        let adds_def = copies == 0;

        let before = arsenal_synergy_pct(ars.damage_types, ars.attack_classes, ars.distinct_defs);
        let after = arsenal_synergy_pct(
            ars.damage_types + adds_type as u32,
            ars.attack_classes + adds_class as u32,
            ars.distinct_defs + adds_def as u32,
        );
        // The archetype's belief in breadth scales only the MARGIN, never the
        // synergy the arsenal already has — that part is a fact, not an opinion.
        let delta = (after - before) * bias.synergy_weight_pct / 100;
        own * (100 + before + delta) / 100 + ars.total_power * delta / 100
    }

    /// Best affordable, allowed WEAPON offer by value-for-money, or `None`.
    /// `extra` narrows the candidate set (challenge runs use it to require a
    /// class); pass `|_| true` for the default bot.
    ///
    /// `patience_pct` is the SAVE-UP rule. The bot declines to buy at all when the
    /// best thing it can currently afford is worth less than `patience_pct`% of the
    /// best weapon visible on the board — i.e. "there is something much better
    /// here, keep the gold". Pass `0` to disable it (the weapon floor does, because
    /// board control is not negotiable).
    ///
    /// This is the other half of the fix, and it matters as much as the valuation.
    /// Early on the bot earns ~500 gold a round and the shop shows one 500g Common
    /// beside three 1500–3000g weapons worth five to nine times as much. Buying the
    /// affordable one on repeat is how the arsenal used to become eleven copies of a
    /// Common before the first shop ever refreshed — and no valuation can prevent
    /// that, because when exactly one offer is affordable "cheapest" and "best" are
    /// the same offer. Patience converts income into FEWER, BETTER weapons bought
    /// LATER, which is both what a competent player does and what lets shop variety
    /// reach the build at all.
    ///
    /// Ties resolve to the LOWEST slot index (strict `>` improvement over an
    /// in-order scan), so the choice is a pure, reproducible function of state.
    fn best_weapon(
        &self,
        s: &ArenaState,
        bias: WeaponBias,
        patience_pct: i64,
        extra: impl Fn(&Offer) -> bool,
    ) -> Option<usize> {
        let ars = Arsenal::scan(s, bias);
        let r = self.price_ref(s, bias);
        let mut best: Option<(usize, i64, i64)> = None; // (slot, value, cost)
        let mut best_affordable_value = 0i64;
        let mut best_visible_value = 0i64;
        for (i, off) in s.shop.offers.iter().enumerate() {
            if !matches!(off.kind, OfferKind::Weapon) || !self.allowed(off) || !extra(off) {
                continue;
            }
            let v = self.weapon_value(&ars, bias, off.def);
            // The patience yardstick counts every weapon on the board, including
            // the ones gold cannot reach yet — those are exactly what it saves for.
            if v > best_visible_value {
                best_visible_value = v;
            }
            if off.cost > s.economy.gold {
                continue;
            }
            let better = match best {
                None => true,
                // value_i / (cost_i + r) > value_b / (cost_b + r), cross-multiplied.
                // i128 keeps the product exact and cannot overflow under the
                // workspace's release-mode `overflow-checks`.
                Some((_, bv, bc)) => {
                    (v as i128) * ((bc + r) as i128) > (bv as i128) * ((off.cost + r) as i128)
                }
            };
            if better {
                best = Some((i, v, off.cost));
                best_affordable_value = v;
            }
        }
        if patience_pct > 0 && best_affordable_value * 100 < best_visible_value * patience_pct {
            return None; // hold the gold; something much better is on the board.
        }
        best.map(|(i, _, _)| i)
    }

    /// The weapon bias for this match: the archetype's for a default run, the
    /// balanced profile for a challenge run (which has no archetype identity — its
    /// identity is the challenge).
    fn weapon_bias(&self) -> WeaponBias {
        match (self.challenge, self.archetype) {
            (Challenge::None, Some(a)) => a.weapon_bias(),
            _ => Archetype::Balanced.weapon_bias(),
        }
    }

    /// Cheapest affordable slot whose offer matches `pred`, if any. Still the rule
    /// for MODIFIERS in the generic fallback; weapons go through
    /// [`Bot::best_weapon`].
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
            let round = if s.round == u32::MAX { 0 } else { s.round };
            // The generic fallback below honors the same PACE, so no path can dump
            // the whole arsenal budget into a single early shop.
            arch_target_weapons = arch.target_weapons();
            let floor = arch.weapon_floor(round);
            let bias = arch.weapon_bias();
            // (1) Weapon floor — the BEST weapon on the board for the money, not
            // the cheapest one (see the valuation block above).
            // Patience 0 here: below the floor the tank has no board control, and
            // a weapon now beats a better weapon later.
            if s.weapons.len() < floor {
                if let Some(slot) = self.best_weapon(s, bias, 0, |_| true) {
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
            // How fast that happens is now governed by PATIENCE, not by a quota: the
            // bot buys when the board has something worth its gold (see
            // `Bot::best_weapon`), which spreads acquisition across shops and a
            // growing wallet on its own.
            if self.can_buy_weapon(s) && s.weapons.len() < arch_target_weapons {
                if let Some(slot) = self.best_weapon(s, bias, bias.patience_pct, |_| true) {
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
        // weapon while under the target arsenal size. For the default bot, weapons stop
        // at the ARCHETYPE target (not the hard cap), so a tanky build keeps a lean
        // arsenal and a glass-cannon a fat one — i.e. the archetype's weapon count is
        // preserved instead of every build draining its leftover gold into the same
        // 40-weapon ceiling. (`WEAPON_BUY_CAP` remains the absolute safety ceiling for
        // challenge runs / edge cases.)
        //
        // Weapons are ranked by VALUE-for-money here too, so no path — floor, top-up,
        // or this fallback — can quietly reintroduce the price-first pick. Modifiers
        // keep the cheapest-first rule (their axis preference is handled above, and
        // modifier valuation is out of scope for this change).
        let want_weapon = s.weapons.len() < arch_target_weapons;
        let mods_ok = self.can_buy_mod() || self.challenge != Challenge::None;
        let weapons_ok = if self.challenge == Challenge::None {
            s.weapons.len() < arch_target_weapons
        } else {
            self.can_buy_weapon(s)
        };
        let bias = self.weapon_bias();
        let weapon_pick = if weapons_ok {
            self.best_weapon(s, bias, bias.patience_pct, |_| true)
        } else {
            None
        };
        let mod_pick = if mods_ok {
            self.cheapest_where(s, |o| matches!(o.kind, OfferKind::Modifier) && self.allowed(o))
        } else {
            None
        };
        // Kind preference wins outright; the other kind is the fallback.
        let ordered =
            if want_weapon { [weapon_pick, mod_pick] } else { [mod_pick, weapon_pick] };
        for pick in ordered {
            let Some(slot) = pick else { continue };
            if matches!(s.shop.offers[slot].kind, OfferKind::Weapon) {
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

// ── weapon-valuation constants ──────────────────────────────────────────────
/// Ceiling on the estimated simultaneous-target count (in tenths) any attack
/// class may claim. A `Barrage(12)` needs twelve enemies in range to be worth
/// twelve targets; crediting it in full would let the catalog's widest weapon
/// outrank everything on a board that rarely holds that many.
const MAX_TARGETS10: i64 = 50;
/// Mirror of `Modifiers::arsenal_synergy_add`: +10% per damage type beyond the
/// first, +8% per attack class beyond the first, +5% per distinct weapon def
/// beyond the first, each axis capped at +40% and the total at +120%. Copies are
/// neither rewarded nor taxed. Kept here as constants (rather than read from
/// `modifiers.rs`) so the bot's MODEL of the incentive is explicit and reviewable;
/// they must be updated in lockstep if the shipped magnitudes move.
const SYN_PER_DAMAGE_TYPE_PCT: i64 = 10;
const SYN_PER_ATTACK_CLASS_PCT: i64 = 8;
const SYN_PER_DISTINCT_DEF_PCT: i64 = 5;
const SYN_AXIS_CAP_PCT: i64 = 40;
const SYN_TOTAL_CAP_PCT: i64 = 120;
/// Upper bound on the gold-on-hand reference in the value/price trade-off. Four
/// times the priciest weapon (5000g), so once the bot holds ~80k gold price has
/// effectively stopped mattering — which is the intended behavior — while the
/// comparison stays in a small, overflow-proof integer range as gold snowballs
/// into the billions.
const PRICE_REF_CAP: i64 = 20_000;
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

#[cfg(test)]
mod weapon_valuation_tests {
    use super::*;
    use crate::{step, WeaponInstance};

    fn def_by_name(name: &str) -> u16 {
        content::WEAPONS
            .iter()
            .position(|w| w.name == name)
            .unwrap_or_else(|| panic!("no weapon named {name}")) as u16
    }

    /// An arena with an empty arsenal, `gold` on hand, and exactly the named
    /// weapons on offer (slot order == argument order).
    fn shop_of(gold: i64, names: &[&str]) -> ArenaState {
        let mut s = ArenaState::new(1234, 0);
        s.weapons.clear();
        s.economy.gold = gold;
        s.shop.offers = names
            .iter()
            .map(|n| {
                let def = def_by_name(n);
                Offer { kind: OfferKind::Weapon, def, cost: content::WEAPONS[def as usize].cost }
            })
            .collect();
        s
    }

    fn give(s: &mut ArenaState, name: &str, n: usize) {
        let def = def_by_name(name);
        for _ in 0..n {
            let id = s.alloc_entity_id();
            s.weapons.push(WeaponInstance { instance_id: id, def, next_fire_tick: 0 });
        }
    }

    fn balanced() -> WeaponBias {
        Archetype::Balanced.weapon_bias()
    }

    /// THE regression test for the bug this module exists to fix. The old policy
    /// was `cheapest_where(kind == Weapon)`, which took the 500g Poison Bow (60
    /// damage, single target) over the 5000g Slap (2600 damage, ~44× the
    /// damage-per-second) purely because it cost less. With gold to spare, value
    /// must win.
    #[test]
    fn picks_the_better_weapon_not_the_cheaper_one() {
        let s = shop_of(1_000_000, &["Poison Bow", "Slap"]);
        let bot = Bot::default();
        assert_eq!(bot.best_weapon(&s, balanced(), 0, |_| true), Some(1));
    }

    /// ...but cheap is still sometimes right. The same comparison flips on a thin
    /// wallet, because value-PER-GOLD — not value alone — is the ranking.
    #[test]
    fn price_still_matters_when_gold_is_tight() {
        let bot = Bot::default();
        // Knives (500g, Barrage(3)) vs Ballista (1500g, Barrage(4), stronger).
        let rich = shop_of(1_000_000, &["Knives", "Ballista"]);
        assert_eq!(bot.best_weapon(&rich, balanced(), 0, |_| true), Some(1), "rich: buy the best");
        let poor = shop_of(1_600, &["Knives", "Ballista"]);
        assert_eq!(
            bot.best_weapon(&poor, balanced(), 0, |_| true),
            Some(0),
            "poor: buy the efficient one"
        );
    }

    /// Multi-target reach is part of a weapon's worth and is invisible to price:
    /// both of these are 500g Commons, but one hits three enemies per volley.
    #[test]
    fn multi_target_beats_single_target_at_the_same_price() {
        let s = shop_of(1_000_000, &["Magic Missile", "Knives"]);
        assert_eq!(s.shop.offers[0].cost, s.shop.offers[1].cost, "same price by construction");
        let bot = Bot::default();
        assert_eq!(bot.best_weapon(&s, balanced(), 0, |_| true), Some(1));
    }

    /// The decision depends on WHAT IS ALREADY OWNED — the thing a price-first bot
    /// structurally cannot see. Magic Bolt outguns Chaos Orb head to head, but on
    /// top of a stack of Magic Bolts the Chaos Orb wins on breadth (new damage
    /// type, new distinct def).
    #[test]
    fn breadth_synergy_flips_the_choice_once_a_stack_exists() {
        let bot = Bot::default();
        let empty = shop_of(1_000_000, &["Magic Bolt", "Chaos Orb"]);
        assert_eq!(
            bot.best_weapon(&empty, balanced(), 0, |_| true),
            Some(0),
            "head to head the stronger weapon wins"
        );

        let mut stacked = shop_of(1_000_000, &["Magic Bolt", "Chaos Orb"]);
        give(&mut stacked, "Magic Bolt", 14);
        assert_eq!(
            bot.best_weapon(&stacked, balanced(), 0, |_| true),
            Some(1),
            "on top of 14 copies, a new damage type is worth more than a 15th"
        );
    }

    /// The synergy model mirrors `Modifiers::arsenal_synergy_add`: +10 / +8 / +5 per
    /// extra damage type / attack class / distinct def, each axis capped at +40, the
    /// total at +120. Copies move none of it.
    #[test]
    fn synergy_model_matches_the_documented_magnitudes() {
        assert_eq!(arsenal_synergy_pct(1, 1, 1), 0, "one weapon earns nothing");
        assert_eq!(arsenal_synergy_pct(2, 1, 1), 10, "+10 per extra damage type");
        assert_eq!(arsenal_synergy_pct(1, 2, 1), 8, "+8 per extra attack class");
        assert_eq!(arsenal_synergy_pct(1, 1, 2), 5, "+5 per extra distinct def");
        assert_eq!(arsenal_synergy_pct(5, 1, 1), 40, "damage-type axis caps at +40");
        assert_eq!(arsenal_synergy_pct(9, 1, 1), 40, "and stays capped");
        assert_eq!(arsenal_synergy_pct(1, 6, 1), 40, "attack-class axis caps at +40");
        assert_eq!(arsenal_synergy_pct(1, 1, 9), 40, "distinct-def axis caps at +40");
        assert_eq!(arsenal_synergy_pct(5, 6, 9), 120, "all three axes capped == +120");
        assert_eq!(arsenal_synergy_pct(9, 9, 40), 120, "total caps at +120");
    }

    /// The worked example from the brief: with no synergy yet banked, a NEW distinct
    /// def is already the better buy at about 29% of a stacked weapon's raw power.
    #[test]
    fn a_new_def_pays_for_itself_at_roughly_thirty_percent_of_a_copy() {
        // 14 copies of power p: a copy adds p; a new def adds q + 0.05·14p.
        // Break-even q = 0.3p → ~29% (see `Bot::weapon_value`).
        let p = 10_000i64;
        let copies = 14i64;
        let new_def_gain = 3_000i64 + (copies * p) * SYN_PER_DISTINCT_DEF_PCT / 100;
        assert!(
            new_def_gain >= p,
            "a def at 30% of a stacked weapon's power should already win: {new_def_gain} vs {p}"
        );
    }

    /// Copies are worth progressively less (target saturation), and how fast is an
    /// archetype trait — that is what separates a stacker from a coverage build.
    #[test]
    fn extra_copies_are_worth_progressively_less() {
        let unit = 10_000;
        let sat = 8;
        assert_eq!(saturated_copy_power(unit, 0, sat), unit, "the first copy is worth full");
        assert!(saturated_copy_power(unit, 1, sat) < saturated_copy_power(unit, 0, sat));
        assert!(saturated_copy_power(unit, 5, sat) < saturated_copy_power(unit, 1, sat));
        assert!(saturated_copy_power(unit, 20, sat) >= 1, "never zero or negative");
        // A glass-cannon tolerates stacking far better than a tanky build.
        let glass = Archetype::GlassCannon.weapon_bias().copy_saturation;
        let tanky = Archetype::Tanky.weapon_bias().copy_saturation;
        assert!(
            saturated_copy_power(unit, 6, glass) > saturated_copy_power(unit, 6, tanky),
            "glass-cannon must value the 7th copy more than a tanky build does"
        );
    }

    /// Patience: with only a 500g Common affordable beside a far better 5000g Epic,
    /// the bot declines and keeps the gold. This is what stops it converting its
    /// whole early income into copies of the one thing it can afford.
    #[test]
    fn patience_declines_a_weak_buy_when_something_far_better_is_on_the_board() {
        let s = shop_of(900, &["Poison Bow", "Slap"]);
        let bot = Bot::default();
        assert_eq!(
            bot.best_weapon(&s, balanced(), 0, |_| true),
            Some(0),
            "impatient (at the weapon floor) it takes what it can get"
        );
        assert_eq!(
            bot.best_weapon(&s, balanced(), 40, |_| true),
            None,
            "patient, it saves for the Epic instead"
        );
    }

    /// Patience must not stall a uniformly cheap shop — there is nothing better to
    /// wait for, so the bot buys.
    #[test]
    fn patience_does_not_stall_when_the_board_is_uniform() {
        let s = shop_of(900, &["Magic Bolt", "Magic Missile"]);
        let bot = Bot::default();
        assert!(bot.best_weapon(&s, balanced(), 40, |_| true).is_some());
    }

    /// It never proposes a slot it cannot pay for.
    #[test]
    fn never_selects_an_unaffordable_offer() {
        let s = shop_of(600, &["Slap", "Magic Bolt", "Meteor Barrage"]);
        let bot = Bot::default();
        let slot = bot.best_weapon(&s, balanced(), 0, |_| true).expect("one is affordable");
        assert!(s.shop.offers[slot].cost <= s.economy.gold);
        assert_eq!(slot, 1);
    }

    /// Ties resolve to the lowest slot index, so the choice is reproducible rather
    /// than dependent on scan order.
    #[test]
    fn ties_go_to_the_lowest_slot() {
        let s = shop_of(1_000_000, &["Magic Bolt", "Magic Bolt"]);
        let bot = Bot::default();
        assert_eq!(bot.best_weapon(&s, balanced(), 0, |_| true), Some(0));
    }

    /// The four archetypes must not collapse into one another: they are how build
    /// diversity is measured.
    #[test]
    fn archetypes_have_distinct_weapon_biases() {
        let all =
            [Archetype::GlassCannon, Archetype::Tanky, Archetype::EcoPivot, Archetype::Balanced];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                let (x, y) = (a.weapon_bias(), b.weapon_bias());
                assert!(
                    x.synergy_weight_pct != y.synergy_weight_pct
                        || x.utility_weight_pct != y.utility_weight_pct
                        || x.price_divisor != y.price_divisor
                        || x.patience_pct != y.patience_pct
                        || x.copy_saturation != y.copy_saturation,
                    "{a:?} and {b:?} have identical weapon biases"
                );
            }
        }
    }

    /// Determinism is sacred: the same seed must produce the same input stream,
    /// every time. (Integer-only valuation, no RNG, no wall-clock, stable slot
    /// iteration order.)
    #[test]
    fn decisions_are_identical_for_the_same_seed() {
        fn play(seed: u64, ticks: u32) -> Vec<Input> {
            let mut s = ArenaState::new(seed, 0);
            let mut bot = Bot::default();
            let mut out = Vec::new();
            while s.tick < ticks && !s.dead {
                let a = bot.decide(&s);
                out.push(a);
                step(&mut s, a);
            }
            out
        }
        for seed in [0u64, 7, 42] {
            assert_eq!(play(seed, 4_000), play(seed, 4_000), "seed {seed} diverged");
        }
    }

    /// The valuation must not overflow at the extremes the sim actually reaches (a
    /// full 40-weapon arsenal of the catalog's strongest weapon, billions of gold).
    /// `overflow-checks` is on in release, so a panic here would be a live crash.
    #[test]
    fn valuation_is_overflow_safe_at_the_extremes() {
        let mut s = shop_of(i64::MAX / 4, &["Slap", "Meteor Barrage", "Shroom Doom"]);
        give(&mut s, "Slap", WEAPON_BUY_CAP);
        let bot = Bot::default();
        for a in
            [Archetype::GlassCannon, Archetype::Tanky, Archetype::EcoPivot, Archetype::Balanced]
        {
            let bias = a.weapon_bias();
            let ars = Arsenal::scan(&s, bias);
            for off in s.shop.offers.iter() {
                let v = bot.weapon_value(&ars, bias, off.def);
                assert!(v > 0, "{a:?} valued {} at {v}", content::WEAPONS[off.def as usize].name);
            }
            assert!(bot.best_weapon(&s, bias, bias.patience_pct, |_| true).is_some());
        }
    }

    /// Every weapon in the catalog must price without panicking or landing on a
    /// nonsense value — the bot has to cope with whatever content ships.
    #[test]
    fn every_catalogued_weapon_prices_sanely() {
        for (i, w) in content::WEAPONS.iter().enumerate() {
            let p = weapon_power(i as u16, 100);
            assert!(p > 0, "{} priced at {p}", w.name);
            assert!(est_targets10(w) >= 10, "{} claims fewer than one target", w.name);
            assert!(est_targets10(w) <= MAX_TARGETS10, "{} exceeds the target cap", w.name);
        }
    }
}
