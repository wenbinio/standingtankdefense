//! The authoritative arena data model + deterministic vector math.
//! Owned centrally (the cross-agent seam). Behavior modules READ/WRITE these
//! fields but must not change the struct definitions.

use crate::content;
use crate::ids::*;
use determinism::{Fixed, Rng};

/// 2D point/vector in Fixed units. The tank sits at the origin.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Vec2 {
    pub x: Fixed,
    pub y: Fixed,
}

// ---- Exact narrow-domain fast paths for the movement inner loop -------------
//
// `step_toward` is the single hottest thing in the sim (a callgrind profile of a
// bot-driven 30-min run puts `move_enemies` at ~48% of all instructions, and
// `Fixed::sqrt` alone at ~17%). Both `Fixed::sqrt` and `Fixed::div` are written
// for the FULL Q47.16 range and pay for it: `sqrt` runs Newton's method over
// `u128` (each iteration a software `u128` division), and `div` compiles to
// `__divti3`, a software 128-bit divide.
//
// Arena geometry never needs that range. The two helpers below take the same
// integers, detect the range in which 64-bit arithmetic is EXACT, and compute the
// IDENTICAL value there — same floor, same truncation-toward-zero, same
// saturation domain — falling back to the general routine outside it. This is
// not an approximation and not a different rounding rule: it is the same integer
// result reached with cheaper instructions, so no checksum can observe it.
// `fast_path_agrees_with_fixed_over_the_arena_domain` (below) pins that.

/// `floor(sqrt(v))` in Fixed units, identical to [`Fixed::sqrt`].
///
/// `Fixed::sqrt` computes `isqrt((raw as u128) << 16)`. When `raw < 2^47` that
/// shifted value fits in a `u64`, and `u64::isqrt` is the same exact floor-sqrt
/// on the same integer — so the two agree bit-for-bit. `raw < 2^47` means a
/// squared distance under 2^31 ≈ 2.1e9 arena units², i.e. any two points within
/// ~46,000 units of each other; the arena is ~2,000 units across.
#[inline]
fn fx_sqrt(v: Fixed) -> Fixed {
    let raw = v.raw();
    if (0..(1i64 << 47)).contains(&raw) {
        Fixed::from_raw(((raw as u64) << Fixed::FRAC_BITS).isqrt() as i64)
    } else {
        v.sqrt()
    }
}

/// `a * b` in Fixed units, identical to [`Fixed::mul`].
///
/// `Fixed::mul` computes `sat((a as i128 * b as i128) >> 16)`. When both operands
/// are under 2^31 in magnitude the product is under 2^62 and therefore fits an
/// `i64` — the saturation is the identity, and an arithmetic right shift floors
/// toward negative infinity in 64 bits exactly as it does in 128. Arena
/// coordinates run to ~2,000 units (raw ~1.3e8, i.e. 2^27), and speeds/radii are
/// far smaller, so the fast path carries every geometric multiply in the sim.
#[inline]
fn fx_mul(a: Fixed, b: Fixed) -> Fixed {
    const LIM: i64 = 1 << 31;
    let (ar, br) = (a.raw(), b.raw());
    if ar > -LIM && ar < LIM && br > -LIM && br < LIM {
        Fixed::from_raw((ar * br) >> Fixed::FRAC_BITS)
    } else {
        a.mul(b)
    }
}

/// `a / b` in Fixed units, identical to [`Fixed::div`].
///
/// `Fixed::div` computes `sat(((a as i128) << 16) / b)`. When `|a| < 2^47` the
/// shifted numerator fits in an `i64`, the quotient is no larger in magnitude
/// than the numerator (so it cannot overflow and the saturation is the identity),
/// and `i64` division truncates toward zero exactly like the `i128` one.
#[inline]
fn fx_div(a: Fixed, b: Fixed) -> Fixed {
    let (ar, br) = (a.raw(), b.raw());
    if br != 0 && ar > -(1i64 << 47) && ar < (1i64 << 47) {
        Fixed::from_raw((ar << Fixed::FRAC_BITS) / br)
    } else {
        a.div(b)
    }
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
    };
    #[inline]
    pub fn new(x: Fixed, y: Fixed) -> Vec2 {
        Vec2 { x, y }
    }
    /// Squared distance to `o` (no sqrt; use for range checks vs `range*range`).
    #[inline]
    pub fn dist_sq(self, o: Vec2) -> Fixed {
        let dx = o.x - self.x;
        let dy = o.y - self.y;
        fx_mul(dx, dx) + fx_mul(dy, dy)
    }
    /// Move from `self` toward `target` by at most `max_step`; clamps to target
    /// on arrival. Fully deterministic (integer sqrt). Returns the new point.
    pub fn step_toward(self, target: Vec2, max_step: Fixed) -> Vec2 {
        let dx = target.x - self.x;
        let dy = target.y - self.y;
        let d2 = fx_mul(dx, dx) + fx_mul(dy, dy);
        let step2 = fx_mul(max_step, max_step);
        if d2 <= step2 || d2 == Fixed::ZERO {
            return target;
        }
        let dist = fx_sqrt(d2);
        Vec2 {
            x: self.x + fx_div(fx_mul(dx, max_step), dist),
            y: self.y + fx_div(fx_mul(dy, max_step), dist),
        }
    }

    /// Move `self` directly AWAY from `from` by `step` units (Knockback). If
    /// `self == from` (degenerate, e.g. enemy exactly on the tank) the point is
    /// unchanged. Fully deterministic (integer sqrt).
    pub fn step_away(self, from: Vec2, step: Fixed) -> Vec2 {
        let dx = self.x - from.x;
        let dy = self.y - from.y;
        let d2 = fx_mul(dx, dx) + fx_mul(dy, dy);
        if d2 == Fixed::ZERO {
            return self;
        }
        let dist = fx_sqrt(d2);
        Vec2 {
            x: self.x + fx_div(fx_mul(dx, step), dist),
            y: self.y + fx_div(fx_mul(dy, step), dist),
        }
    }
}

/// The player's stationary tank.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tank {
    pub hp: i64,
    pub max_hp: i64,
    pub pos: Vec2,
    pub clear_cooldown_end: Tick,
    /// Flat damage reduction applied before the shield/HP (min 1 gets through).
    pub armor: i64,
    /// Dodge chance = `dodge_num / dodge_den` (avoids a hit entirely).
    pub dodge_num: u32,
    pub dodge_den: u32,
    /// Mana Shield absorb pool; damage hits it before HP.
    pub mana_shield: i64,
    pub mana_shield_max: i64,
    pub mana_regen_per_tick: i64,
    /// Passive HP regeneration per tick.
    pub hp_regen_per_tick: i64,
    /// Spikes: damage dealt to nearby enemies when the tank is hit.
    pub spikes_damage: i64,
    /// Multiplier on spikes damage (starts at `ONE`).
    pub spikes_mult: Fixed,
    /// Damage reduction (fraction) applied to ALL incoming damage while the Mana
    /// Shield is active (`mana_shield > 0`) — the source's "+% Damage Reduction
    /// while Mana Shield active". Accumulates additively; clamped to `[0, ONE]`
    /// at hit time so the resulting multiplier can never go negative. Starts ZERO.
    pub shield_active_dr: Fixed,
    /// Flat HP healed each time an incoming hit LANDS (i.e. is not dodged) — the
    /// source's "+N Heal when damaged". Routes through `heal` (so `healing_mult`
    /// + max-HP cap apply). One heal per landed hit. Starts 0.
    pub heal_on_damaged: i64,
    /// Heal the tank this much when an enemy dies (on-kill trigger).
    pub heal_on_kill: i64,
    /// Restore this much Mana Shield each time an enemy dies (on-kill trigger) —
    /// the source's Maw of Death "+N Mana regenerated when an enemy dies". Routes
    /// through `restore_mana` (cap-respecting; one per kill, like `heal_on_kill`).
    /// Starts 0.
    pub mana_on_kill: i64,
    /// Heal the tank this much each tick an enemy takes poison damage.
    pub heal_on_poison: i64,
    /// Multiplier on all healing the tank receives (the source's "+% Healing";
    /// starts at `ONE`).
    pub healing_mult: Fixed,
    /// Fraction of missing HP healed once per second (the source's "% Missing HP
    /// Heal every second"; starts `ZERO`).
    pub missing_hp_heal_pct: Fixed,
    /// Remaining one-shot revives (Ankh): a fatal hit is survived instead of dying.
    pub revives: u32,
    /// Max-HP granted (and HP repaired to) when a revive is consumed.
    pub revive_bonus_hp: i64,

    // ---- EXPANSION E2 (exotic mechanics) -------------------------------------
    /// Shield-break stun (source: Energy Pulse). When the Mana Shield transitions
    /// `>0 → 0` because of a hit, stun every enemy within `shieldbreak_stun_range`
    /// for `shieldbreak_stun_ticks`. `range == 0` ⇒ disabled. Both feed the
    /// checksum (snapshot-serialized).
    pub shieldbreak_stun_range: i64,
    pub shieldbreak_stun_ticks: u32,
    /// Spikes-applied DoT (source: Poison Armor — spikes ALSO poison the reflected
    /// attacker). When Spikes retaliation lands on an enemy, apply this Poison DoT
    /// to it (reusing the existing poison status). `dps == 0` ⇒ disabled.
    pub spikes_poison_dps: i64,
    pub spikes_poison_ticks: u32,
    /// Stacking spikes (source: Bloody Spikes — spikes damage accumulates per hit).
    /// Each landed hit grows `spikes_stacks` by 1 up to `spikes_stacks_max`; the
    /// bonus spikes damage is `spikes_stack_per × spikes_stacks`. Reset to 0 at the
    /// round boundary (matching the source's "resets when a new shop is made").
    /// `spikes_stack_per == 0` ⇒ no stacking. `spikes_stacks` feeds the checksum.
    pub spikes_stack_per: i64,
    pub spikes_stacks: u32,
    pub spikes_stacks_max: u32,
    /// Periodic damage/poison aura (source: Blight Aura). Every `aura_cadence`
    /// ticks (integer; NOT wall-clock), deal `aura_damage` and apply a Poison DoT
    /// to all enemies within `aura_range`. `aura_tick` is the per-tank cadence
    /// counter (snapshot-serialized, feeds the checksum). `aura_cadence == 0` ⇒
    /// disabled.
    pub aura_range: i64,
    pub aura_cadence: u32,
    pub aura_damage: i64,
    pub aura_poison_dps: i64,
    pub aura_poison_ticks: u32,
    pub aura_tick: u32,
}

impl Tank {
    /// Apply healing, scaled by `healing_mult` and capped at `max_hp`. The single
    /// chokepoint for every heal so "+% Healing" and the cap live in one place.
    pub fn heal(&mut self, amount: i64) {
        if amount <= 0 {
            return;
        }
        let scaled = self.healing_mult.scale_i64(amount);
        self.hp = (self.hp + scaled).min(self.max_hp);
    }

    /// Restore `amount` to the Mana-Shield pool, capped at its max (the source's
    /// mana-drain weapons "restore N Mana Shield per enemy hit"). A no-op if the
    /// tank has no shield pool. Not scaled by `healing_mult` (it is shield, not HP).
    pub fn restore_mana(&mut self, amount: i64) {
        if amount <= 0 || self.mana_shield_max <= 0 {
            return;
        }
        self.mana_shield = (self.mana_shield + amount).min(self.mana_shield_max);
    }
}

/// An owned weapon instance (multiple copies of one def stack as separate
/// instances — the source's "everything stacks").
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WeaponInstance {
    pub instance_id: EntityId,
    pub def: u16, // index into content::WEAPONS
    pub next_fire_tick: Tick,
}

/// Per-enemy status effects (Poison / Frost / Fire / Stun). Pure integer
/// counters (no floats) for determinism. Default = no status.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EnemyStatus {
    /// Poison damage per tick while `poison_ticks > 0` (a DoT).
    pub poison_dps: i64,
    pub poison_ticks: u32,
    /// Frost stacks (each slows move/attack ~2%, capped at `FROST_MAX_STACKS`).
    pub frost_stacks: u8,
    /// Remaining frost duration; on expiry the stacks clear.
    pub frost_ticks: u32,
    /// Fire stacks (each adds +0.5% damage taken; enemy explodes on death).
    pub fire_stacks: u16,
    /// Generic vulnerability stacks (each adds +1% damage taken) — from
    /// Vulnerability-Pulse auras.
    pub vuln_stacks: u16,
    /// Immobile while `> 0` (from on-hit stuns).
    pub stun_ticks: u32,
    /// Freeze duration (from reaching `FROST_MAX_STACKS`, the Deep Freeze
    /// payoff). While `> 0` the enemy is immobile AND takes +50% damage; decays
    /// once per tick and clears with no residual effect.
    pub freeze_ticks: u32,
}

/// A periodic aura the tank emits: every `interval_ticks` it adds `magnitude`
/// vulnerability stacks to enemies within `range` (the source's Vulnerability
/// Pulse). Registered when its modifier is bought.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VulnPulse {
    pub magnitude: u16,
    pub range: i64,
    pub interval_ticks: u32,
    pub next_tick: Tick,
}

/// An active enemy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Enemy {
    pub id: EntityId,
    pub def: u16, // index into content::ENEMIES
    pub hp: i64,
    pub pos: Vec2,
    pub status: EnemyStatus,
}

impl Enemy {
    /// Construct an enemy with no status (the common case).
    pub fn new(id: EntityId, def: u16, hp: i64, pos: Vec2) -> Enemy {
        Enemy { id, def, hp, pos, status: EnemyStatus::default() }
    }
}

/// A persistent damaging area dropped by a weapon ability (Boom Bloom's mine
/// field / scorched ground). Each tick it pulses `dmg` to every non-boss enemy
/// within `radius`, for `ticks_left` ticks, then expires. Fully deterministic:
/// fixed integer fields, stable id order, no RNG. Damage routes through the
/// shared death path so kills award bounty and Fire deaths still explode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hazard {
    pub id: EntityId,
    pub pos: Vec2,
    /// Damage dealt to each enemy in range per tick.
    pub dmg: i64,
    /// Damage type (matches the placing weapon — drives the armor matrix).
    pub damage_type: u8,
    pub radius: i64,
    /// Remaining ticks before the hazard expires.
    pub ticks_left: u32,
}

/// A temporary ALLY summoned by a weapon (a raised Larva /
/// Spore). Each tick it walks toward the nearest non-boss enemy and
/// strikes it when in reach; it expires at `expire_tick`. Damage routes through
/// the shared death path so its kills award bounty and trigger Fire explosions.
/// Fully deterministic: fixed/integer fields, stable id order, no RNG.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Minion {
    pub id: EntityId,
    pub pos: Vec2,
    /// Render kind: 0 = larva, 1 = spore.
    pub kind: u8,
    /// Reserved for future enemy retaliation; minions are lifetime-bounded today.
    pub hp: i64,
    /// Per-strike base damage (scaled by the match-time curve when it lands).
    pub damage: i64,
    pub damage_type: u8,
    /// Earliest tick it may strike again (attack cooldown).
    pub next_attack_tick: Tick,
    /// Tick at which it vanishes.
    pub expire_tick: Tick,
}

/// An in-flight projectile (homes on `target`; applies splash at arrival).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Projectile {
    pub id: EntityId,
    pub pos: Vec2,
    pub target: EntityId,
    pub last_target_pos: Vec2,
    pub damage: i64,
    pub damage_type: u8,
    pub splash_radius: Fixed, // ZERO ⇒ single target
    pub speed: Fixed,
    /// Status this projectile applies to whatever it hits.
    pub on_hit: content::StatusOnHit,
    /// Signature ability executed at impact (life/mana drain, knockback, root,
    /// vulnerability stacks, hazard placement). `None` for most projectiles.
    pub ability: content::WeaponAbility,
}

/// Player economy state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Economy {
    pub gold: i64,
    pub income_per_tick: i64, // base passive income (no multiplier — source rule)
    /// Multiplier on passive income (starts at `ONE`). Distinct from `bounty_mult`,
    /// which by the source rule never touches passive income.
    pub income_mult: Fixed,
    /// Fraction of each income award also granted to the tank as instant HP
    /// (the source's "% of Gold Income as instant HP Regen"; starts `ZERO`).
    pub income_regen_pct: Fixed,
    pub bounty_mult: Fixed,   // applies to kill bounty only
    /// Chance (in percent, 0–100) that a kill pays a bonus bounty; `0` ⇒ no roll.
    pub bounty_proc_chance_pct: i64,
    /// Bonus fraction of the base bounty paid when a proc fires (e.g. `2.0` ⇒ +200%).
    pub bounty_proc_bonus: Fixed,
    /// Damage-scaled bounty rate (the source's "Bloodmoney"): each point of
    /// player damage dealt awards `floor(damage × gold_per_damage)` gold. Additive
    /// across owned copies (starts `ZERO`).
    pub gold_per_damage: Fixed,
    /// Fraction of each income award also added to the Mana-Shield pool (capped at
    /// its max), mirroring `income_regen_pct` for HP (starts `ZERO`).
    pub income_shield_pct: Fixed,
    pub rerolls_remaining: u32,
    pub reroll_cost: i64,
}

/// What a shop slot sells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OfferKind {
    /// `def` indexes `content::WEAPONS`.
    Weapon,
    /// `def` indexes `content::MODIFIERS`.
    Modifier,
}

/// One purchasable shop slot (a weapon or a stacking modifier).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Offer {
    pub kind: OfferKind,
    pub def: u16,
    pub cost: i64,
}

/// A one-shot meta perk armed by a meta item (Magic Coin / Duplicator / Black
/// Market), consumed by the next matching non-meta purchase (`docs/06` #5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingPerk {
    /// Only purchases of this rarity qualify (`255` = any rarity).
    pub rarity: u8,
    /// Extra free copies granted to the matching purchase (duplicator).
    pub extra_copies: u32,
    /// Whether the matching purchase is free (Black Market voucher).
    pub free: bool,
}

impl PendingPerk {
    /// Whether a purchase at `rarity` qualifies for this perk.
    pub fn matches(&self, rarity: u8) -> bool {
        self.rarity == 255 || self.rarity == rarity
    }
}

/// An active time-scaling growth: re-applies `effect` every `interval_ticks`
/// (the source's "+X every 30 seconds"). Registered when a ramping modifier is
/// purchased; lives for the rest of the match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActiveRamp {
    pub effect: content::ModEffect,
    pub interval_ticks: u32,
    pub next_apply: Tick,
}

/// Aggregated damage & attack-speed modifiers (`docs/05 §5.3`). The genre's
/// "everything stacks" engine: **additive within a source kind, multiplicative
/// across distinct multiplicative sources**. Economy/defensive modifiers apply
/// immediately on purchase (to `Economy`/`Tank`); this aggregate holds only what
/// damage resolution consults live each tick. Impl lives in `modifiers.rs`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Modifiers {
    /// Additive % applied to all weapon damage.
    pub add_global: Fixed,
    /// Additive % per damage type (Normal, Piercing, Magic, Siege, Chaos).
    pub add_by_type: [Fixed; 5],
    /// Additive % per weapon scope (attack class / range bucket / rarity);
    /// indexed by `content::*_scope_id`. Length `content::NUM_SCOPES`.
    pub add_by_scope: [Fixed; 12],
    /// Product of multiplicative damage factors (starts at `ONE`).
    pub mul_global: Fixed,
    /// Additive % attack speed (reduces effective weapon cooldown).
    pub attack_speed: Fixed,
    /// Additive % bonus damage dealt to **stunned** enemies (target-conditional,
    /// resolved at impact; starts `ZERO`).
    pub vs_stunned: Fixed,
    /// Additive % bonus damage dealt to **poisoned** enemies (starts `ZERO`).
    pub vs_poisoned: Fixed,
    /// Multiplier on applied Poison DoT magnitude (starts at `ONE`).
    pub poison_dmg_mult: Fixed,
    /// Multiplier on applied Stun duration (starts at `ONE`).
    pub stun_dur_mult: Fixed,
    /// Self-scaling damage: `+per` additive % to weapons of `dmg_type` for each
    /// owned weapon of `weapon_def` (the source's "+1% Piercing Damage per Bow").
    /// Append-only; resolved live at fire time. Length grows with purchases.
    pub weapon_count_scaling: Vec<WeaponCountScale>,
    /// DYNAMIC global-damage scaler keyed to the LIVE `tank.max_hp` (the source's
    /// Mastercrafted Masonry "+X% Damage per 2000 Max HP"). Stored as the summed
    /// per-unit rate `Σ (n/d)`; at fire time the live bonus is
    /// `rate × (max_hp / 2000)`. Accumulates additively across purchases. Like
    /// `weapon_count_scaling`, it is resolved live (never baked at purchase), so it
    /// tracks Max-HP bought afterwards. Starts `ZERO`.
    pub dmg_per_maxhp_rate: Fixed,
    /// DYNAMIC global-damage scaler keyed to the LIVE `economy.bounty_mult` (the
    /// source's Golden Ring "+X% Damage per 50% Bounty"). Summed per-unit rate;
    /// live bonus is `rate × (bounty_bonus_pct / 50)` where `bounty_bonus_pct` is
    /// the bounty multiplier ABOVE the 1.0 base. Starts `ZERO`.
    pub dmg_per_bounty_rate: Fixed,
    /// DYNAMIC global-damage scaler active only while the Mana Shield is up (the
    /// source's Arcane Mark "+X% Damage while Mana Shield active"). Summed bonus;
    /// at fire time it is added to the global additive iff `tank.mana_shield > 0`
    /// (the offensive mirror of `tank.shield_active_dr`). Starts `ZERO`.
    pub shield_active_dmg: Fixed,
}

/// One self-scaling damage rule (see [`Modifiers::weapon_count_scaling`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeaponCountScale {
    /// Weapon def whose owned count drives the bonus.
    pub weapon_def: u16,
    /// Damage type the bonus applies to (matches the firing weapon's type).
    pub dmg_type: u8,
    /// Additive % per owned copy of `weapon_def`.
    pub per: Fixed,
}

/// The per-round shop. M0 offers weapons only.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ShopState {
    pub offers: Vec<Offer>,
    pub shop_seq: u32,
}

/// The complete authoritative arena state. `step()` is a pure function of this
/// plus the tick's `Input` (`docs/03 §3.3`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArenaState {
    pub tick: Tick,
    pub round: u32, // u32::MAX sentinel before the first round starts
    pub master_seed: u64,
    pub player_id: u32,

    pub tank: Tank,
    pub weapons: Vec<WeaponInstance>,
    pub enemies: Vec<Enemy>,
    pub projectiles: Vec<Projectile>,
    /// Persistent damaging areas (land mines / burning oil) from weapon
    /// abilities. Stored in id order; ticked in `combat::tick_hazards`.
    pub hazards: Vec<Hazard>,
    /// Summoned allies (Larvae / Spores). Stored in id order; ticked in
    /// `combat::tick_minions`.
    pub minions: Vec<Minion>,
    pub economy: Economy,
    pub shop: ShopState,
    /// Aggregated damage/attack-speed modifiers consulted during combat.
    pub modifiers: Modifiers,
    /// Active time-scaling growths (re-applied at their intervals).
    pub ramps: Vec<ActiveRamp>,
    /// Active Vulnerability-Pulse auras.
    pub vuln_pulses: Vec<VulnPulse>,
    /// A meta perk (duplicator/voucher) armed for the next matching purchase.
    pub pending_perk: Option<PendingPerk>,
    /// Set when the tank takes damage this tick (drives Spikes retaliation).
    /// Transient: always `false` at a tick boundary, so it is excluded from the
    /// checksum/snapshot.
    pub tank_hit_this_tick: bool,
    /// Set when the Mana Shield transitioned `>0 → 0` from a hit this tick (drives
    /// the shield-break stun pulse — source: Energy Pulse). Transient: consumed and
    /// cleared within the same tick (in `defense::shield_break_stun`), so it is
    /// always `false` at a tick boundary and excluded from the checksum/snapshot.
    pub shield_broke_this_tick: bool,

    pub next_entity_id: u32,
    pub dead: bool,
    pub death_tick: Option<Tick>,

    /// Enemy defs killed this tick (pushed by combat/Clear, drained by
    /// economy::collect_bounties in the same tick). Decouples Agent B from
    /// Agent C — no cross-module calls. Empty at end of every tick.
    pub pending_kills: Vec<u16>,

    /// Running scoreboard: total damage dealt by PLAYER sources over the match
    /// and total gold earned (bounty + income + grants + trades). Authoritative
    /// (feed the checksum); never reset.
    pub total_damage_dealt: i64,
    pub total_gold_earned: i64,

    /// Playstyle telemetry for cosmetic achievements (which weapon *attack
    /// classes* the player chose to BUY, how many weapons, how many income
    /// purchases). Render/profile-only — NOT fed to the checksum, so it can
    /// never affect determinism. The free starting Bow is granted, not bought,
    /// so it does not set a bit here. Bit `n` == `attack_scope_id` `n` (0..6).
    pub bought_attack_mask: u16,
    pub weapons_bought: u32,
    pub economy_purchases: u32,

    // Per-purpose RNG streams (cursors ride in snapshots).
    pub rng_spawn: Rng,
    pub rng_targeting: Rng,
    pub rng_shop: Rng,
    pub rng_reroll: Rng,
    pub rng_proc: Rng,
}

impl ArenaState {
    /// Fresh arena for `master_seed`/`player_id`. Starts with one Bow so combat
    /// is exercised from tick 0; the first `step()` generates the round-0 shop.
    pub fn new(master_seed: u64, player_id: u32) -> ArenaState {
        let d = |p: Purpose| Rng::derive(master_seed, player_id, p as u32, 0);
        let mut s = ArenaState {
            tick: 0,
            round: u32::MAX,
            master_seed,
            player_id,
            tank: Tank {
                hp: 24_000,
                max_hp: 24_000,
                pos: Vec2::ZERO,
                clear_cooldown_end: 0,
                armor: 0,
                dodge_num: 0,
                dodge_den: 100,
                mana_shield: 0,
                mana_shield_max: 0,
                mana_regen_per_tick: 0,
                hp_regen_per_tick: 0,
                spikes_damage: 0,
                spikes_mult: Fixed::ONE,
                shield_active_dr: Fixed::ZERO,
                heal_on_damaged: 0,
                heal_on_kill: 0,
                mana_on_kill: 0,
                heal_on_poison: 0,
                healing_mult: Fixed::ONE,
                missing_hp_heal_pct: Fixed::ZERO,
                revives: 0,
                revive_bonus_hp: 0,
                shieldbreak_stun_range: 0,
                shieldbreak_stun_ticks: 0,
                spikes_poison_dps: 0,
                spikes_poison_ticks: 0,
                spikes_stack_per: 0,
                spikes_stacks: 0,
                spikes_stacks_max: 0,
                aura_range: 0,
                aura_cadence: 0,
                aura_damage: 0,
                aura_poison_dps: 0,
                aura_poison_ticks: 0,
                aura_tick: 0,
            },
            weapons: Vec::new(),
            enemies: Vec::new(),
            projectiles: Vec::new(),
            hazards: Vec::new(),
            minions: Vec::new(),
            economy: Economy {
                gold: 500,
                income_per_tick: 20, // 600 gold/s baseline (tuning)
                income_mult: Fixed::ONE,
                income_regen_pct: Fixed::ZERO,
                bounty_mult: Fixed::ONE,
                bounty_proc_chance_pct: 0,
                bounty_proc_bonus: Fixed::ZERO,
                gold_per_damage: Fixed::ZERO,
                income_shield_pct: Fixed::ZERO,
                rerolls_remaining: 5,
                reroll_cost: 100,
            },
            shop: ShopState::default(),
            modifiers: Modifiers::new(),
            ramps: Vec::new(),
            vuln_pulses: Vec::new(),
            pending_perk: None,
            tank_hit_this_tick: false,
            shield_broke_this_tick: false,
            next_entity_id: 1,
            dead: false,
            death_tick: None,
            pending_kills: Vec::new(),
            total_damage_dealt: 0,
            total_gold_earned: 0,
            bought_attack_mask: 0,
            weapons_bought: 0,
            economy_purchases: 0,
            rng_spawn: d(Purpose::Spawn),
            rng_targeting: d(Purpose::Targeting),
            rng_shop: d(Purpose::Shop),
            rng_reroll: d(Purpose::Reroll),
            rng_proc: d(Purpose::Proc),
        };
        let id = s.alloc_entity_id();
        s.weapons.push(WeaponInstance {
            instance_id: id,
            def: content::STARTING_WEAPON,
            next_fire_tick: 0,
        });
        s
    }

    /// Allocate a fresh, never-reused entity id.
    #[inline]
    pub fn alloc_entity_id(&mut self) -> EntityId {
        let id = EntityId(self.next_entity_id);
        self.next_entity_id += 1;
        id
    }

    /// Index of a live enemy by id, if present.
    pub fn enemy_index(&self, id: EntityId) -> Option<usize> {
        self.enemies.iter().position(|e| e.id == id)
    }

    /// The single chokepoint for every gold gain: credits the wallet AND the
    /// `total_gold_earned` scoreboard. ALL gold income (bounty, passive income,
    /// grants, trades) routes through here so the scoreboard stays authoritative.
    #[inline]
    pub fn award_gold(&mut self, amount: i64) {
        if amount == 0 {
            return;
        }
        // SATURATING: an extreme snowball (Bloodmoney + bounty procs on a huge board)
        // can push lifetime gold past i64. Clamping is deterministic (pure integer,
        // platform-stable) and only ever engages on out-of-range totals, so no normal
        // run / checksum changes — it turns a would-be overflow panic into a ceiling.
        self.economy.gold = self.economy.gold.saturating_add(amount);
        self.total_gold_earned = self.total_gold_earned.saturating_add(amount);
    }

    /// Record `amount` of damage dealt by a PLAYER source: accumulates the
    /// scoreboard total and pays the damage-scaled bounty (Bloodmoney) through
    /// `award_gold`. Called once per damage site after the enemy borrow ends.
    #[inline]
    pub fn record_player_damage(&mut self, amount: i64) {
        if amount <= 0 {
            return;
        }
        // SATURATING scoreboard accumulation (see `award_gold`): a saturated single-
        // hit damage value can otherwise overflow the lifetime total.
        self.total_damage_dealt = self.total_damage_dealt.saturating_add(amount);
        if self.economy.gold_per_damage > Fixed::ZERO {
            self.award_gold(self.economy.gold_per_damage.scale_i64(amount));
        }
    }

    /// Apply a purchased modifier (folds into the damage/attack-speed aggregate,
    /// and applies any immediate economy/defensive effect). Disjoint field
    /// borrows keep this single-call.
    pub fn buy_modifier(&mut self, def_idx: u16) {
        let def = &content::MODIFIERS[def_idx as usize];
        let ramp = def.ramp;
        // Scan the modifier's effects IN ORDER. A few effects carry per-arena
        // trigger state and are intercepted here; every other effect folds into
        // the Modifiers/Tank/Economy aggregates via the normal path. (Each catalog
        // entry carries a single effect today, so this reproduces the prior
        // single-effect behavior exactly.)
        for &effect in def.effects {
            match effect {
                content::ModEffect::GrantVulnPulse(mag, range, interval) => {
                    self.vuln_pulses.push(VulnPulse {
                        magnitude: mag as u16,
                        range,
                        interval_ticks: interval as u32,
                        next_tick: self.tick + interval as u32,
                    });
                }
                // META items (`docs/06` #5) arm the purchase flow / grant gold.
                content::ModEffect::GrantDuplicator(rarity, copies) => {
                    self.pending_perk = Some(PendingPerk {
                        rarity: rarity as u8,
                        extra_copies: copies as u32,
                        free: false,
                    });
                }
                content::ModEffect::GrantVoucher(rarity) => {
                    self.pending_perk = Some(PendingPerk {
                        rarity: rarity as u8,
                        extra_copies: 0,
                        free: true,
                    });
                }
                content::ModEffect::GrantGold(g) => self.award_gold(g),
                // HP/defense ↔ Gold trades: pay tank stats for gold, intercepted
                // here so the gold routes through the scoreboard.
                content::ModEffect::TradeMaxHpForGold(hp_cost, gold_gain) => {
                    self.tank.max_hp -= hp_cost;
                    self.tank.hp = self.tank.hp.min(self.tank.max_hp);
                    self.award_gold(gold_gain);
                }
                content::ModEffect::TradeRegenForGold(regen_cost, gold_gain) => {
                    // MAY go negative — a negative regen drains HP each tick.
                    self.tank.hp_regen_per_tick -= regen_cost;
                    self.award_gold(gold_gain);
                }
                other => self.modifiers.apply_effect(other, &mut self.economy, &mut self.tank),
            }
        }
        if let Some(r) = ramp {
            self.ramps.push(ActiveRamp {
                effect: r.effect,
                interval_ticks: r.interval_ticks,
                next_apply: self.tick + r.interval_ticks,
            });
        }
    }

    /// Grant one unit of an offer (a weapon instance or a modifier application),
    /// independent of gold/perk handling. Used by `input` for the base purchase
    /// and for each extra duplicator copy.
    pub fn grant_offer(&mut self, offer: Offer) {
        match offer.kind {
            OfferKind::Weapon => {
                let id = self.alloc_entity_id();
                let def = &content::WEAPONS[offer.def as usize];
                // Telemetry (cosmetic, off-checksum): record the attack class bought.
                self.bought_attack_mask |= 1u16 << content::attack_scope_id(def.attack);
                self.weapons_bought += 1;
                self.weapons.push(WeaponInstance {
                    instance_id: id,
                    def: offer.def,
                    next_fire_tick: self.tick,
                });
            }
            OfferKind::Modifier => {
                if content::MODIFIERS[offer.def as usize].is_economy() {
                    self.economy_purchases += 1;
                }
                self.buy_modifier(offer.def);
            }
        }
    }

    /// A purchase's rarity (for perk matching).
    pub fn offer_rarity(offer: Offer) -> u8 {
        match offer.kind {
            OfferKind::Weapon => content::WEAPONS[offer.def as usize].rarity,
            OfferKind::Modifier => content::MODIFIERS[offer.def as usize].rarity,
        }
    }

    /// Whether an offer is a META item (must not trigger or be duplicated).
    pub fn offer_is_meta(offer: Offer) -> bool {
        matches!(offer.kind, OfferKind::Modifier)
            && content::MODIFIERS[offer.def as usize].is_meta()
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::{fx_div, fx_mul, fx_sqrt, Vec2};
    use determinism::Fixed;

    /// The optimization's whole warrant: over (and well past) the domain the arena
    /// actually reaches, the cheap 64-bit routines return the SAME integers as
    /// `Fixed::sqrt` / `Fixed::div`. Any divergence here would be a checksum
    /// divergence, so this is checked exhaustively over a dense sweep plus the
    /// boundaries of the fast-path guards.
    #[test]
    fn fast_path_agrees_with_fixed_over_the_arena_domain() {
        let mut probes: Vec<i64> = Vec::new();
        // Dense low range (sub-unit through a few hundred units squared).
        for i in 0..40_000i64 {
            probes.push(i);
        }
        // Geometric sweep across the whole i64 range, including both sides of the
        // 2^47 guard and the saturating extremes.
        let mut v = 1i64;
        while v < i64::MAX / 3 {
            probes.push(v - 1);
            probes.push(v);
            probes.push(v + 1);
            v = v.saturating_mul(3);
        }
        for b in 40..52 {
            let e = 1i64 << b;
            probes.extend([e - 2, e - 1, e, e + 1, e + 2]);
        }
        probes.extend([i64::MAX, i64::MAX - 1]);

        for &p in &probes {
            let f = Fixed::from_raw(p);
            let nf = Fixed::from_raw(p.saturating_neg());
            assert_eq!(fx_sqrt(f).raw(), f.sqrt().raw(), "sqrt diverged at raw {p}");
            for &q in &[1i64, 2, 3, 65_536, 65_537, -1, -65_536, 7_919, -7_919, i64::MAX, i64::MIN] {
                let d = Fixed::from_raw(q);
                assert_eq!(fx_div(f, d).raw(), f.div(d).raw(), "div diverged at {p} / {q}");
                assert_eq!(fx_div(nf, d).raw(), nf.div(d).raw(), "div diverged at -{p} / {q}");
                assert_eq!(fx_mul(f, d).raw(), f.mul(d).raw(), "mul diverged at {p} * {q}");
                assert_eq!(fx_mul(nf, d).raw(), nf.mul(d).raw(), "mul diverged at -{p} * {q}");
                assert_eq!(fx_mul(d, f).raw(), d.mul(f).raw(), "mul diverged at {q} * {p}");
            }
            // `mul` squares its own operand all over the geometry code.
            assert_eq!(fx_mul(f, f).raw(), f.mul(f).raw(), "square diverged at {p}");
            assert_eq!(fx_mul(nf, nf).raw(), nf.mul(nf).raw(), "square diverged at -{p}");
            assert_eq!(fx_mul(f, nf).raw(), f.mul(nf).raw(), "mixed-sign mul diverged at {p}");
        }
        // Both sides of the 2^31 `fx_mul` guard, in every sign combination.
        for a in [(1i64 << 31) - 2, (1 << 31) - 1, 1 << 31, (1 << 31) + 1, 1 << 40] {
            for b in [1i64, 65_536, (1 << 31) - 1, 1 << 31, (1 << 31) + 1] {
                for (sa, sb) in [(1i64, 1i64), (1, -1), (-1, 1), (-1, -1)] {
                    let (x, y) = (Fixed::from_raw(a * sa), Fixed::from_raw(b * sb));
                    assert_eq!(fx_mul(x, y).raw(), x.mul(y).raw(), "mul diverged at {a}*{sa} × {b}*{sb}");
                }
            }
        }
    }

    /// `dist_sq` is the most-called routine in the sim (every range check in
    /// combat, hazards, auras and minions), so its rewrite gets its own check
    /// against the unoptimized formula over the arena and well past it.
    #[test]
    fn dist_sq_matches_the_unoptimized_reference() {
        for ax in (-3_000_000..=3_000_000).step_by(97_003) {
            for ay in (-3_000_000..=3_000_000).step_by(311_027) {
                let a = Vec2::new(Fixed::from_raw(ax), Fixed::from_raw(ay));
                for bx in [-2_100_000i64, -65_536, 0, 1, 65_537, 2_100_000] {
                    let b = Vec2::new(Fixed::from_raw(bx), Fixed::from_raw(-bx));
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    assert_eq!(a.dist_sq(b), dx.mul(dx) + dy.mul(dy), "dist_sq diverged");
                }
            }
        }
    }

    /// End-to-end: the two callers of the fast paths reproduce a reference
    /// `step_toward` / `step_away` built from the unoptimized `Fixed` routines,
    /// over a grid of offsets and speeds spanning the arena.
    #[test]
    fn step_toward_matches_the_unoptimized_reference() {
        fn ref_step_toward(a: Vec2, target: Vec2, max_step: Fixed) -> Vec2 {
            let dx = target.x - a.x;
            let dy = target.y - a.y;
            let d2 = dx.mul(dx) + dy.mul(dy);
            let step2 = max_step.mul(max_step);
            if d2 <= step2 || d2 == Fixed::ZERO {
                return target;
            }
            let dist = d2.sqrt();
            Vec2 { x: a.x + dx.mul(max_step).div(dist), y: a.y + dy.mul(max_step).div(dist) }
        }
        fn ref_step_away(a: Vec2, from: Vec2, step: Fixed) -> Vec2 {
            let dx = a.x - from.x;
            let dy = a.y - from.y;
            let d2 = dx.mul(dx) + dy.mul(dy);
            if d2 == Fixed::ZERO {
                return a;
            }
            let dist = d2.sqrt();
            Vec2 { x: a.x + dx.mul(step).div(dist), y: a.y + dy.mul(step).div(dist) }
        }

        let origin = Vec2::ZERO;
        for xr in (-2_100_000..=2_100_000).step_by(9_973) {
            for yr in (-2_100_000..=2_100_000).step_by(131_071) {
                let p = Vec2::new(Fixed::from_raw(xr), Fixed::from_raw(yr));
                for spd in [1i64, 3, 7, 11, 40] {
                    let s = Fixed::from_int(spd);
                    assert_eq!(p.step_toward(origin, s), ref_step_toward(p, origin, s));
                    assert_eq!(p.step_away(origin, s), ref_step_away(p, origin, s));
                }
            }
        }
    }
}
