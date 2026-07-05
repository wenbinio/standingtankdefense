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
        dx.mul(dx) + dy.mul(dy)
    }
    /// Move from `self` toward `target` by at most `max_step`; clamps to target
    /// on arrival. Fully deterministic (integer sqrt). Returns the new point.
    pub fn step_toward(self, target: Vec2, max_step: Fixed) -> Vec2 {
        let dx = target.x - self.x;
        let dy = target.y - self.y;
        let d2 = dx.mul(dx) + dy.mul(dy);
        let step2 = max_step.mul(max_step);
        if d2 <= step2 || d2 == Fixed::ZERO {
            return target;
        }
        let dist = d2.sqrt();
        Vec2 {
            x: self.x + dx.mul(max_step).div(dist),
            y: self.y + dy.mul(max_step).div(dist),
        }
    }

    /// Binary-angle (BAM) heading of this vector: 0..=65535 units per full
    /// turn, 0 along +x, counterclockwise-positive (+y = 16384). Deterministic
    /// pure-integer octant approximation (piecewise-linear `atan`, monotonic
    /// within each octant — exact at the octant boundaries, ≤ ~4° off between
    /// them, which is plenty for sweep-sector membership). The zero vector maps
    /// to 0. Intermediate math is widened to i128 so extreme fixed-point
    /// magnitudes cannot overflow.
    pub fn bam_angle(self) -> u16 {
        let x = self.x.raw();
        let y = self.y.raw();
        if x == 0 && y == 0 {
            return 0;
        }
        let ax = (x as i128).abs();
        let ay = (y as i128).abs();
        // Quarter-turn angle q in [0, 16384]: linear blend of the two octants.
        let q: i128 = if ax >= ay {
            (ay << 13) / ax // 0..=8192 (|y| <= |x|)
        } else {
            16384 - ((ax << 13) / ay) // 8192..=16384
        };
        let ang: i128 = match (x >= 0, y >= 0) {
            (true, true) => q,
            (false, true) => 32768 - q,
            (false, false) => 32768 + q,
            (true, false) => 65536 - q,
        };
        (ang & 0xFFFF) as u16
    }

    /// Move `self` directly AWAY from `from` by `step` units (Knockback). If
    /// `self == from` (degenerate, e.g. enemy exactly on the tank) the point is
    /// unchanged. Fully deterministic (integer sqrt).
    pub fn step_away(self, from: Vec2, step: Fixed) -> Vec2 {
        let dx = self.x - from.x;
        let dy = self.y - from.y;
        let d2 = dx.mul(dx) + dy.mul(dy);
        if d2 == Fixed::ZERO {
            return self;
        }
        let dist = d2.sqrt();
        Vec2 {
            x: self.x + dx.mul(step).div(dist),
            y: self.y + dy.mul(step).div(dist),
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

    // ---- EXPANSION E3 (fidelity-mechanics pass) -------------------------------
    /// Permanent HP-regen accumulated per Healthstone-class ATTACK (the source's
    /// "+0.2 permanent HP Regen" — sub-integer, so fixed-point). Added on top of
    /// `hp_regen_per_tick` each tick, paid out through `regen_carry`. Starts ZERO.
    pub regen_bonus_per_tick: Fixed,
    /// Fractional carry for `regen_bonus_per_tick` (the sub-1-HP remainder kept
    /// between ticks so no regen is lost to integer flooring). Authoritative —
    /// persists across ticks, feeds the checksum/snapshot. Starts ZERO.
    pub regen_carry: Fixed,
    /// Deep Freeze opt-in (source upgrade): only while `true` does reaching
    /// `FROST_MAX_STACKS` freeze an enemy; otherwise stacks cap at max slow.
    /// Granted by `ModEffect::GrantDeepFreeze`. Starts `false`.
    pub deep_freeze: bool,
    /// Frost Armor: frost stacks applied to enemies in Spikes range when the
    /// tank is hit (`defense::spikes`; fires even with 0 Spikes damage). 0 = off.
    pub retaliate_frost: u8,
    /// Flaming Armor: fire stacks applied on the same retaliation. 0 = off.
    pub retaliate_fire: u16,
    /// Flat bonus Spikes damage added to the retaliation for an enemy's FIRST
    /// landed hit on the tank (per enemy; see `EnemyStatus::hit_tank`). 0 = off.
    pub spikes_first_hit: i64,
    /// Deflection: fraction of current Spikes damage granted as flat damage
    /// reduction per incoming hit, capped at 50% of the hit. Starts ZERO.
    pub spikes_dr_rate: Fixed,
    /// Damage-taken→Spikes conversion: fraction of the damage the tank took
    /// this tick added as flat Spikes damage to the retaliation. Starts ZERO.
    pub dmg_taken_to_spikes: Fixed,
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

    /// THE "at 95% health or above" gate shared by every heal-conditional
    /// damage bonus (`DamageWhileHealthyPct` / `HealingWeaponDamagePct`). Pure
    /// integer compare (hp × 20 ≥ max_hp × 19 ⇔ hp/max ≥ 0.95); both stats are
    /// clamped ≤ `STAT_CEIL` (1e12) so the ×20 cannot overflow i64.
    pub fn is_healthy(&self) -> bool {
        self.max_hp > 0 && self.hp.saturating_mul(20) >= self.max_hp.saturating_mul(19)
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
    /// Obscured (Ale Launcher): while `obscure_ticks > 0`, this enemy's attacks
    /// on the tank miss with `obscure_pct`% probability (rolled on `rng_proc`).
    /// Strongest pct wins; duration takes the longer remaining; pct clears on
    /// expiry.
    pub obscure_pct: u16,
    pub obscure_ticks: u32,
    /// TYPED vulnerability stacks (each +1% damage taken, applying only to hits
    /// of the matching damage type 0..=4) — from `VulnTypeOnHit` weapons (Thorn
    /// / Liquid Fire drench). Composes additively with the generic
    /// `vuln_stacks`.
    pub vuln_by_type: [u16; 5],
    /// Whether this enemy has ever LANDED a hit on the tank (drives the
    /// first-hit bonus Spikes, `Tank::spikes_first_hit`).
    pub hit_tank: bool,
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
        Enemy {
            id,
            def,
            hp,
            pos,
            status: EnemyStatus::default(),
        }
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

/// An active ROTATING-WAVE sweep (from `Attack::WaveRotating`): fired once,
/// then each tick it advances one angular sector (`step_bam` binary-angle
/// units, 65536 = full turn) around the tank and hits every enemy inside
/// `radius` whose angle falls in the sector — so the wave visibly "rotates"
/// across the board over `ticks_left` ticks, hitting each enemy as it passes.
/// Fully deterministic: integer binary-angle math (`Vec2::bam_angle`), stable
/// id order, no RNG. Damage/on-hit are baked at fire time (like projectiles).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WaveSweep {
    pub id: EntityId,
    /// Catalog index of the firing weapon (render sprite selection).
    pub weapon_kind: u16,
    /// Baked damage (weapon multiplier resolved at fire time).
    pub damage: i64,
    pub damage_type: u8,
    /// Sweep reach from the tank (`range + extra`).
    pub radius: Fixed,
    /// Current sector start, in binary-angle units (0..=65535 = full turn).
    pub angle_bam: u16,
    /// Sector swept per tick (binary-angle units; divides 65536 so the sectors
    /// tile the circle exactly — each enemy is hit once per revolution).
    pub step_bam: u16,
    /// Remaining sweep ticks; the sweep expires at 0.
    pub ticks_left: u32,
    /// Rotation direction (`true` = clockwise / decreasing angle).
    pub clockwise: bool,
    /// Baked on-hit status (poison/frost/fire/stun, pre-scaled at fire time).
    pub on_hit: content::StatusOnHit,
    /// Signature ability executed per enemy the sweep hits.
    pub ability: content::WeaponAbility,
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
    /// Catalog index of the weapon that fired it (`content::WEAPONS`). Pure
    /// bookkeeping for rendering (sprite selection) — combat never reads it —
    /// but it rides the snapshot (so reconnect redraws correctly), and every
    /// snapshot-carried field must feed `checksum()` (parity rule).
    pub weapon_kind: u16,
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
    pub bounty_mult: Fixed, // applies to kill bounty only
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
    /// Magic Treasure holding pool: gold accrued by a held treasure (+2/s in
    /// `economy::tick_income`), auto-banked into the wallet when the next shop
    /// rolls (`economy::on_round_start`). `0` ⇒ no treasure held. Authoritative
    /// (feeds `checksum()`, rides the snapshot).
    pub treasure_pool: i64,
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

/// What KINDS of purchase a [`PendingPerk`] may consume — the source scopes
/// its meta items more tightly than rarity alone (see `buy_modifier`):
/// Multiplication Gems target "the next 500 Gold (Common) **Upgrade**" while
/// Duplicator / Black Market target "**Weapon or Spikes Damage Upgrade**".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PerkScope {
    /// Any offer kind qualifies (rarity check only).
    Any,
    /// Only modifier ("Upgrade") purchases qualify — never weapons.
    UpgradeOnly,
    /// Weapons, or modifiers from the Spikes upgrade family, qualify.
    WeaponOrSpikes,
}

impl PerkScope {
    /// Stable wire/checksum tag.
    pub fn as_u8(self) -> u8 {
        match self {
            PerkScope::Any => 0,
            PerkScope::UpgradeOnly => 1,
            PerkScope::WeaponOrSpikes => 2,
        }
    }
    /// Inverse of [`as_u8`](Self::as_u8).
    pub fn from_u8(v: u8) -> Option<PerkScope> {
        Some(match v {
            0 => PerkScope::Any,
            1 => PerkScope::UpgradeOnly,
            2 => PerkScope::WeaponOrSpikes,
            _ => return None,
        })
    }
}

/// A one-shot meta perk armed by a meta item (Multiplication Gems / Duplicator
/// / Black Market), consumed by the next matching non-meta purchase
/// (`docs/06` #5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingPerk {
    /// Only purchases of this rarity qualify (`255` = any rarity).
    pub rarity: u8,
    /// What offer kinds qualify (the source's per-item scoping).
    pub scope: PerkScope,
    /// Extra free copies granted to the matching purchase (duplicator).
    pub extra_copies: u32,
    /// Whether the matching purchase is free (Black Market voucher).
    pub free: bool,
}

/// Whether a modifier belongs to the Spikes upgrade family (the source's
/// "Spikes Damage Upgrade" wording) — any effect that grows/extends Spikes
/// retaliation (flat/% Spikes damage, Bloody Spikes stacking, Poison Armor's
/// spikes-applied poison). Judgment call: the source groups all its Spikes
/// upgrades under that label, so the whole family qualifies.
fn modifier_is_spikes_upgrade(def: u16) -> bool {
    content::MODIFIERS[def as usize].effects.iter().any(|e| {
        matches!(
            e,
            content::ModEffect::SpikesFlat(..)
                | content::ModEffect::SpikesPct(..)
                | content::ModEffect::SpikesPoison(..)
                | content::ModEffect::StackingSpikes(..)
        )
    })
}

impl PendingPerk {
    /// Whether a (non-meta) purchase of `offer` qualifies for this perk:
    /// rarity must match AND the offer must fall inside the perk's scope.
    pub fn matches(&self, offer: Offer) -> bool {
        let rarity_ok = self.rarity == 255 || self.rarity == ArenaState::offer_rarity(offer);
        let scope_ok = match self.scope {
            PerkScope::Any => true,
            PerkScope::UpgradeOnly => matches!(offer.kind, OfferKind::Modifier),
            PerkScope::WeaponOrSpikes => match offer.kind {
                OfferKind::Weapon => true,
                OfferKind::Modifier => modifier_is_spikes_upgrade(offer.def),
            },
        };
        rarity_ok && scope_ok
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
    // ---- EXPANSION E3 (fidelity-mechanics pass) -------------------------------
    /// Multiplier on Frost slow strength (the source's "+% Frost damage and slow
    /// strength"). Scales the per-stack slow in `status::move_speed_mult`.
    /// Starts `ONE`; accumulates additively.
    pub frost_strength_mult: Fixed,
    /// Multiplier on Fire strength (the source's "+% Fire damage and damage
    /// vulnerability"): scales the per-stack Fire vulnerability AND the Fire
    /// death-explosion damage. Starts `ONE`; accumulates additively.
    pub fire_dmg_mult: Fixed,
    /// Multiplier on the Fire death-explosion damage ALONE (the source's
    /// Combustion). Multiplies with `fire_dmg_mult` on the explosion (distinct
    /// multiplicative sources). Starts `ONE`; accumulates additively.
    pub fire_explosion_mult: Fixed,
    /// "+% Enemies hit by Bounce and Barrage": per FIRE, those attacks gain
    /// `floor(base_targets × pct)` extra targets (integer, per fire). Starts
    /// `ZERO`; accumulates additively.
    pub bounce_barrage_pct: Fixed,
    /// GLOBAL additive damage bonus active only while `tank.hp ≥ 95% max_hp`
    /// (the source's "+35% Damage when at 95% health or above"), resolved LIVE
    /// at fire time via `dynamic_global_add`. Starts `ZERO`.
    pub healthy_dmg: Fixed,
    /// HEALING-WEAPON-scoped additive damage bonus active only while
    /// `Tank::is_healthy()` (Battle Fervor's "+35% Damage for Healing Weapons
    /// … when at 95% health or above"): applies ONLY to weapons classified by
    /// `content::WeaponDef::is_healing`, resolved LIVE at fire time via
    /// `healing_weapon_add`. Starts `ZERO`; accumulates additively.
    pub healing_weapon_healthy_dmg: Fixed,
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

/// One sim→render event (`docs/09 §9.3` v1). Emitted during `step()` phases so
/// the renderer gets exact edges (a spawn+kill within one tick, multi-hits,
/// muzzle flashes) instead of lossily diffing snapshots. **Render-only, one-way,
/// deterministic by construction**: events are a pure function of deterministic
/// state; emitting them never mutates anything the checksum covers, and the
/// netcode never ships them. All payloads are integers (positions are
/// `floor_to_int` world units, like the view).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SimEvent {
    /// An enemy died to a PLAYER source (weapon/hazard/aura/minion/poison/
    /// spikes/Clear — every `pending_kills` push has a matching `EnemyKilled`).
    /// `fire_explosion_radius` is the Fire death-explosion radius when this
    /// death detonates one (Fire-stacked non-boss via the shared death path),
    /// else 0. `bounty` is the CATALOG base bounty (the actual gold paid —
    /// multipliers/procs applied — arrives as `GoldBounty`).
    EnemyKilled {
        x: i64,
        y: i64,
        kind: u16,
        boss: bool,
        bounty: i64,
        fire_explosion_radius: i64,
    },
    /// An enemy left the board WITHOUT dying to the player (contact
    /// self-destruct on the tank — no bounty, no death FX).
    EnemyDespawned { id: u32 },
    /// A projectile detonated: point, carried damage, damage type, and splash
    /// radius in world units (0 ⇒ single-target).
    Impact {
        x: i64,
        y: i64,
        damage: i64,
        damage_type: u8,
        splash_radius: i64,
    },
    /// A weapon emitted a projectile (muzzle flash / launch FX).
    ProjectileSpawned {
        weapon_kind: u16,
        x: i64,
        y: i64,
        target_x: i64,
        target_y: i64,
    },
    /// A hit landed on the tank (post-dodge; `damage` is the post-armor,
    /// post-shield-DR total applied to shield + HP).
    TankHit { damage: i64 },
    /// A new round began (shop refresh boundary).
    RoundStart { round: u32 },
    /// The boss entered the arena.
    BossSpawned { id: u32 },
    /// A weapon ability dropped a persistent hazard.
    HazardPlaced {
        x: i64,
        y: i64,
        radius: i64,
        ticks: u32,
        damage_type: u8,
    },
    /// A hazard's lifetime ran out.
    HazardExpired { id: u32 },
    /// An enemy hit `FROST_MAX_STACKS` and froze (the Deep Freeze payoff).
    FreezeProc { id: u32 },
    /// The Mana Shield transitioned `>0 → 0` from a hit.
    ShieldBroke,
    /// Total kill bounty paid this tick (multipliers + procs applied).
    GoldBounty { amount: i64 },
}

/// The transient per-tick [`SimEvent`] buffer. **Not authoritative state**: it
/// is cleared at the top of every `step()`, excluded from `checksum()` and the
/// wire snapshot (same class as `tank_hit_this_tick`), and — because undrained
/// events legitimately sit here *between* ticks (the renderer drains after
/// `step`) — it compares equal to any other buffer so `ArenaState` equality
/// (snapshot round-trip, shadow-sim identity) stays a statement about
/// authoritative state only. Clear-on-step bounds it to one tick's events.
#[derive(Clone, Debug, Default)]
pub struct Events(pub Vec<SimEvent>);

impl PartialEq for Events {
    /// Always equal: events are render-side ephemera, not state identity.
    fn eq(&self, _other: &Events) -> bool {
        true
    }
}
impl Eq for Events {}

impl Events {
    /// Drain the buffered events (read-and-clear).
    pub fn take(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.0)
    }
    /// Borrow the buffered events.
    pub fn as_slice(&self) -> &[SimEvent] {
        &self.0
    }
    /// Drop all buffered events.
    pub fn clear(&mut self) {
        self.0.clear();
    }
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
    /// Active rotating-wave sweeps (`Attack::WaveRotating`). Stored in id
    /// order; ticked in `combat::tick_sweeps`.
    pub sweeps: Vec<WaveSweep>,
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
    /// Total post-mitigation damage the tank took this tick (drives the
    /// damage-taken→Spikes conversion). Transient like `tank_hit_this_tick`:
    /// accumulated in `defense::hit_tank`, consumed and zeroed in
    /// `defense::spikes` the same tick, so it is always 0 at a tick boundary
    /// and excluded from the checksum/snapshot.
    pub damage_taken_this_tick: i64,
    /// Flat bonus Spikes damage earned this tick from enemies landing their
    /// FIRST hit on the tank (`Tank::spikes_first_hit`). Transient — same
    /// life-cycle as `damage_taken_this_tick`.
    pub spikes_first_bonus_this_tick: i64,
    /// Transient sim→render event stream for THIS tick (`docs/09 §9.3`).
    /// Cleared at the top of every `step()`, appended during step phases,
    /// excluded from `checksum()`/snapshot (and from `ArenaState` equality —
    /// see [`Events`]). The renderer drains it after `step`; undrained events
    /// are dropped by the next tick's clear (no unbounded growth).
    pub events: Events,

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
    /// purchases). Mostly render/profile-facing, but `bought_attack_mask` is
    /// read by `bot::Challenge::filter` (which shapes the inputs fed to `step`
    /// on both client and director), so all three ride the snapshot AND feed
    /// the checksum (snapshot↔checksum parity). The free starting Bow is
    /// granted, not bought, so it does not set a bit here. Bit `n` ==
    /// `attack_scope_id` `n` (0..6).
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
                regen_bonus_per_tick: Fixed::ZERO,
                regen_carry: Fixed::ZERO,
                deep_freeze: false,
                retaliate_frost: 0,
                retaliate_fire: 0,
                spikes_first_hit: 0,
                spikes_dr_rate: Fixed::ZERO,
                dmg_taken_to_spikes: Fixed::ZERO,
            },
            weapons: Vec::new(),
            enemies: Vec::new(),
            projectiles: Vec::new(),
            hazards: Vec::new(),
            minions: Vec::new(),
            sweeps: Vec::new(),
            economy: Economy {
                gold: 500,
                // 600 gold/s baseline (tuning); the UN-multiplied base — see
                // `economy::BASE_INCOME_PER_TICK` (source income-scoping rule).
                income_per_tick: crate::economy::BASE_INCOME_PER_TICK,
                income_mult: Fixed::ONE,
                income_regen_pct: Fixed::ZERO,
                bounty_mult: Fixed::ONE,
                bounty_proc_chance_pct: 0,
                bounty_proc_bonus: Fixed::ZERO,
                gold_per_damage: Fixed::ZERO,
                income_shield_pct: Fixed::ZERO,
                rerolls_remaining: 5,
                reroll_cost: crate::input::REROLL_COST_BASE,
                treasure_pool: 0,
            },
            shop: ShopState::default(),
            modifiers: Modifiers::new(),
            ramps: Vec::new(),
            vuln_pulses: Vec::new(),
            pending_perk: None,
            tank_hit_this_tick: false,
            shield_broke_this_tick: false,
            damage_taken_this_tick: 0,
            spikes_first_bonus_this_tick: 0,
            events: Events::default(),
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

    /// Buffer a render event for this tick (`docs/09 §9.3`). Never perturbs
    /// authoritative state — the buffer is off-checksum/off-snapshot.
    #[inline]
    pub(crate) fn emit(&mut self, ev: SimEvent) {
        self.events.0.push(ev);
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
                // META items (`docs/06` #5) arm the purchase flow / hold gold.
                // Perk SCOPE is keyed off the source items (the catalog's only
                // duplicators/voucher): the Common duplicator is Multiplication
                // Gems — "the next 500 Gold (Common) Upgrade" ⇒ UPGRADES only;
                // the Rare duplicator is Duplicator — "the next Rare Weapon or
                // Spikes Damage Upgrade" ⇒ weapon-or-spikes.
                content::ModEffect::GrantDuplicator(rarity, copies) => {
                    self.pending_perk = Some(PendingPerk {
                        rarity: rarity as u8,
                        scope: if rarity == 0 {
                            PerkScope::UpgradeOnly
                        } else {
                            PerkScope::WeaponOrSpikes
                        },
                        extra_copies: copies as u32,
                        free: false,
                    });
                }
                // Black Market: "Buy 1 Uncommon Weapon or Spikes Damage
                // Upgrade of your choosing" ⇒ weapon-or-spikes.
                content::ModEffect::GrantVoucher(rarity) => {
                    self.pending_perk = Some(PendingPerk {
                        rarity: rarity as u8,
                        scope: PerkScope::WeaponOrSpikes,
                        extra_copies: 0,
                        free: true,
                    });
                }
                // Magic Treasure (the catalog's only `GrantGold` user): the
                // source item is a HELD consumable whose value grows +2/s and
                // is banked on use — or immediately when a second Treasure is
                // bought ("Purchasing a second Magic Treasure uses the first").
                // Modeled input-free: arm the pool at `g`; it grows in
                // `economy::tick_income` and auto-banks at the next shop roll
                // (`economy::on_round_start`). Buying another banks the first.
                content::ModEffect::GrantGold(g) => {
                    let prior = std::mem::take(&mut self.economy.treasure_pool);
                    self.award_gold(prior);
                    self.economy.treasure_pool = g;
                }
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
                other => self
                    .modifiers
                    .apply_effect(other, &mut self.economy, &mut self.tank),
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
