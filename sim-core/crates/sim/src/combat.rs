//! Combat behavior — AGENT B. Implement the three phases below. Read content via
//! `crate::content`, use `s.rng_targeting` for any randomness, keep all math
//! integer/Fixed (no floats), and iterate in a stable order (by `instance_id` /
//! `id`); never iterate a HashMap. No cross-module calls: record kills by pushing
//! the dead enemy's `def` onto `s.pending_kills` (economy drains it later).
use crate::content::{self, Attack, WeaponAbility};
use crate::ids::EntityId;
use crate::state::*;
use determinism::Fixed;

/// Accumulates the TANK-side / world-side effects of a weapon's signature
/// ability across all the enemies one attack damages, so they can be applied
/// once after the (tank-read-only) enemy loop ends. Enemy-LOCAL ability effects
/// (vuln stacks, knockback displacement, root) are applied directly to the
/// enemy inside [`apply_weapon_hit`]; only effects that touch the tank or spawn
/// world entities are deferred here.
/// One ally to raise this attack: position of the slain enemy plus the minion's
/// seed stats (the per-strike `damage_type` is taken from the killing weapon at
/// flush time, so every minion of one cast shares it).
#[derive(Clone, Copy)]
struct SummonReq {
    pos: Vec2,
    kind: u8,
    hp: i64,
    damage: i64,
}

#[derive(Clone, Default)]
struct AbilityAccum {
    /// Total HP to heal the tank (life-drain), summed per damaged enemy.
    heal: i64,
    /// Total Mana-Shield to restore (mana-drain), summed per damaged enemy.
    mana: i64,
    /// Where to drop a hazard (the first enemy this attack damaged), if any.
    hazard_at: Option<Vec2>,
    /// Corpses to raise as allies (Summon weapons that killed an enemy this hit).
    summons: Vec<SummonReq>,
}

impl AbilityAccum {
    /// Apply the deferred tank/world effects of `ability` (placed at
    /// `damage_type` for hazards). Called once after a damage site, while `s` is
    /// fully borrowable again.
    fn flush(self, s: &mut ArenaState, ability: WeaponAbility, damage_type: u8) {
        if self.heal > 0 && !s.dead {
            s.tank.heal(self.heal);
        }
        if self.mana > 0 {
            s.tank.restore_mana(self.mana);
        }
        if let (WeaponAbility::Hazard { dmg, radius, ticks }, Some(pos)) = (ability, self.hazard_at)
        {
            let id = s.alloc_entity_id();
            s.hazards.push(Hazard {
                id,
                pos,
                dmg,
                damage_type,
                radius,
                ticks_left: ticks,
            });
        }
        // Raise an ally from each corpse this attack made, up to the global cap.
        for req in self.summons {
            if s.minions.len() >= MAX_MINIONS {
                break;
            }
            let id = s.alloc_entity_id();
            s.minions.push(Minion {
                id,
                pos: req.pos,
                kind: req.kind,
                hp: req.hp,
                damage: req.damage,
                damage_type,
                next_attack_tick: s.tick,
                expire_tick: s.tick + MINION_LIFETIME,
            });
        }
    }
}

/// Player's target-conditional damage bonuses, captured once per tick (they don't
/// change mid-tick) and applied at impact against each enemy's live status.
#[derive(Clone, Copy)]
struct CondDamage {
    vs_stunned: Fixed,
    vs_poisoned: Fixed,
}

impl CondDamage {
    fn of(s: &ArenaState) -> CondDamage {
        CondDamage {
            vs_stunned: s.modifiers.vs_stunned,
            vs_poisoned: s.modifiers.vs_poisoned,
        }
    }
    /// Multiplier for this enemy: `1 + Σ matching conditional bonuses`.
    fn mult(&self, e: &Enemy) -> Fixed {
        let mut m = Fixed::ONE;
        if e.status.stun_ticks > 0 {
            m += self.vs_stunned;
        }
        if e.status.poison_ticks > 0 {
            m += self.vs_poisoned;
        }
        m
    }
}

/// Apply one weapon hit to an enemy: `base × armor-matrix × modifier-stack ×
/// fire-vulnerability × target-conditional` damage, then the weapon's on-hit
/// status. Bosses are immune to weapon fire — only `Clear` damages them.
/// Returns the integer damage subtracted from the enemy (0 for an immune boss),
/// for the caller's damage/Bloodmoney accounting.
// The argument list mirrors the damage pipeline's inputs one-to-one; bundling
// them into a struct would churn a hot checksum-path call site for no behavior
// change.
#[allow(clippy::too_many_arguments)]
fn apply_weapon_hit(
    e: &mut Enemy,
    base: i64,
    damage_type: u8,
    mod_mult: Fixed,
    cond: CondDamage,
    on_hit: &content::StatusOnHit,
    ability: WeaponAbility,
    tank_pos: Vec2,
    accum: &mut AbilityAccum,
) -> i64 {
    let edef = &content::ENEMIES[e.def as usize];
    if edef.boss {
        // Bosses are immune to weapon fire AND to its abilities.
        return 0;
    }
    let armor = content::damage_multiplier(damage_type, edef.armor_class);
    let vuln = crate::status::vulnerability_mult(e);
    let dmg = armor
        .mul(mod_mult)
        .mul(vuln)
        .mul(cond.mult(e))
        .scale_i64(base);
    e.hp -= dmg;
    crate::status::apply_on_hit(e, on_hit);
    apply_ability_on_hit(e, ability, tank_pos, accum);
    dmg
}

/// Execute a weapon's signature ability against one enemy it just damaged.
/// Enemy-LOCAL effects (vulnerability stacks, knockback, root) mutate the enemy
/// here; tank/world effects accumulate into `accum` for a single post-loop
/// flush. Deterministic: integer/Fixed math, no RNG.
fn apply_ability_on_hit(
    e: &mut Enemy,
    ability: WeaponAbility,
    tank_pos: Vec2,
    accum: &mut AbilityAccum,
) {
    match ability {
        WeaponAbility::None => {}
        WeaponAbility::Summon { kind, hp, damage } => {
            // Raise from the corpse only if THIS hit was the killing blow.
            // (`apply_weapon_hit` has already subtracted the damage.)
            if e.hp <= 0 {
                accum.summons.push(SummonReq {
                    pos: e.pos,
                    kind,
                    hp,
                    damage,
                });
            }
        }
        WeaponAbility::LifeDrain { per_hit } => accum.heal += per_hit,
        WeaponAbility::ManaDrain { per_hit } => accum.mana += per_hit,
        WeaponAbility::VulnOnHit { stacks } => {
            e.status.vuln_stacks = e.status.vuln_stacks.saturating_add(stacks);
        }
        WeaponAbility::Root { ticks } => {
            // Root = immobilize + poisoned flag, applied directly (does NOT scale
            // with +% Stun Duration, per the source). We reuse `stun_ticks` for
            // the immobilize and ensure a poison marker so "Rooted enemies are
            // considered Stunned & Poisoned".
            e.status.stun_ticks = e.status.stun_ticks.max(ticks);
            if e.status.poison_ticks < ticks {
                e.status.poison_ticks = e.status.poison_ticks.max(ticks);
                // A token DoT so the poisoned condition holds without overwriting
                // a stronger existing poison.
                if e.status.poison_dps == 0 {
                    e.status.poison_dps = 1;
                }
            }
        }
        WeaponAbility::Knockback { dist } => {
            // Shove the enemy directly away from the tank by `dist` units.
            let away = e.pos.step_away(tank_pos, Fixed::from_int(dist));
            e.pos = away;
        }
        WeaponAbility::Hazard { .. } => {
            // Record the FIRST damaged enemy's position as the drop point.
            if accum.hazard_at.is_none() {
                accum.hazard_at = Some(e.pos);
            }
        }
    }
}

/// Phase 4: each ready weapon (`s.tick >= next_fire_tick`) picks a target —
/// RANDOM among enemies within `range` (via `s.rng_targeting.below(n)`, matching
/// the source's "attack at random") — and emits a `Projectile` toward it
/// (`damage`, `damage_type`, `splash_radius` = 0 for SingleTarget or
/// `Splash(r)` radius, `speed` = proj_speed). Set `next_fire_tick = s.tick +
/// cooldown_ticks`. No enemy in range ⇒ do not fire (do not advance cooldown).
pub(crate) fn fire_weapons(s: &mut ArenaState) {
    // Collect new projectiles first so we don't borrow `s` mutably while
    // iterating. Weapons fire in their existing (instance_id) order; candidate
    // enemies are gathered in stable id order before any random pick.
    let mut new_projectiles: Vec<Projectile> = Vec::new();
    let mut any_instant_damage = false;
    // Damage dealt by instant attacks this call (projectile damage is recorded
    // on impact in `advance_projectiles`).
    let mut instant_damage: i64 = 0;
    // Target-conditional bonuses are constant across this tick's fires.
    let cond = CondDamage::of(s);

    for wi in 0..s.weapons.len() {
        let def = s.weapons[wi].def;
        if s.tick < s.weapons[wi].next_fire_tick {
            continue;
        }
        let wdef = content::WEAPONS[def as usize];
        let range = Fixed::from_int(wdef.range);
        let range_sq = range.mul(range);

        // Candidates: enemies within range, in stable id order.
        let candidates: Vec<usize> = (0..s.enemies.len())
            .filter(|&ei| s.tank.pos.dist_sq(s.enemies[ei].pos) <= range_sq)
            .collect();
        if candidates.is_empty() {
            // No enemy in range: do not fire, do not advance cooldown.
            continue;
        }

        // Full per-weapon multiplier (global + type + scope) is resolved HERE,
        // at fire time. For projectiles we bake it into the carried damage; for
        // instant attacks we pass it to apply_weapon_hit.
        // Static multiplier (global + type + scope) plus live self-scaling
        // ("+% per owned weapon"). Folding identity:
        // (1 + add_static + add_self)×mul = weapon_damage_mult + add_self×mul.
        let static_mult = s.modifiers.weapon_damage_mult(&wdef);
        let add_self = s.modifiers.self_scaling_add(wdef.damage_type, &s.weapons);
        // DYNAMIC global-damage scalers resolved from the LIVE tank/economy (per
        // 2000 Max HP / per 50% Bounty / while Mana Shield active). GLOBAL additive,
        // so they thread in exactly like `add_self` — additive, then `×mul_global`.
        let add_dyn = s.modifiers.dynamic_global_add(&s.tank, &s.economy);
        let wmult = static_mult + (add_self + add_dyn).mul(s.modifiers.mul_global);
        let baked = wmult.scale_i64(wdef.damage);
        // Poison-damage / stun-duration scalers depend only on the player's
        // modifiers, so (like base damage) they bake into the hit at fire time.
        let on_hit = s.modifiers.scale_on_hit(wdef.on_hit);
        let ability = wdef.ability;
        let tank_pos = s.tank.pos;
        // Instant-attack abilities (Area/Wave/Bounce) accumulate here, flushed
        // once after this weapon's instant hits resolve.
        let mut accum = AbilityAccum::default();
        let mut new_proj = |s: &mut ArenaState, target_idx: usize, splash: Fixed| {
            let e = &s.enemies[target_idx];
            new_projectiles.push(Projectile {
                id: EntityId(0), // assigned after the loop
                pos: s.tank.pos,
                target: e.id,
                last_target_pos: e.pos,
                damage: baked,
                damage_type: wdef.damage_type,
                splash_radius: splash,
                speed: Fixed::from_int(wdef.proj_speed),
                on_hit,
                ability,
            });
        };

        match wdef.attack {
            Attack::SingleTarget => {
                let pick = candidates[s.rng_targeting.below(candidates.len() as u32) as usize];
                new_proj(s, pick, Fixed::ZERO);
            }
            Attack::Splash(r) => {
                let pick = candidates[s.rng_targeting.below(candidates.len() as u32) as usize];
                new_proj(s, pick, Fixed::from_int(r));
            }
            Attack::Barrage(n) => {
                // N distinct random in-range targets (partial Fisher–Yates).
                let mut pool = candidates.clone();
                let shots = (n as usize).min(pool.len());
                for k in 0..shots {
                    let j = k + s.rng_targeting.below((pool.len() - k) as u32) as usize;
                    pool.swap(k, j);
                    new_proj(s, pool[k], Fixed::ZERO);
                }
            }
            Attack::Area(r) => {
                // Instant pulse around the tank.
                let r2 = Fixed::from_int(r).mul(Fixed::from_int(r));
                for e in s.enemies.iter_mut() {
                    if tank_pos.dist_sq(e.pos) <= r2 {
                        instant_damage += apply_weapon_hit(
                            e,
                            wdef.damage,
                            wdef.damage_type,
                            wmult,
                            cond,
                            &on_hit,
                            ability,
                            tank_pos,
                            &mut accum,
                        );
                    }
                }
                any_instant_damage = true;
            }
            Attack::Wave(extra) => {
                // Instant sweep out to range + extra around the tank.
                let reach = range + Fixed::from_int(extra);
                let r2 = reach.mul(reach);
                for e in s.enemies.iter_mut() {
                    if tank_pos.dist_sq(e.pos) <= r2 {
                        instant_damage += apply_weapon_hit(
                            e,
                            wdef.damage,
                            wdef.damage_type,
                            wmult,
                            cond,
                            &on_hit,
                            ability,
                            tank_pos,
                            &mut accum,
                        );
                    }
                }
                any_instant_damage = true;
            }
            Attack::Bounce(n) => {
                // Random first target, then the N-1 nearest OTHER enemies to it.
                let first = candidates[s.rng_targeting.below(candidates.len() as u32) as usize];
                let origin = s.enemies[first].pos;
                let mut order: Vec<usize> = (0..s.enemies.len()).filter(|&i| i != first).collect();
                // Sort by (distance to origin, id) for a deterministic chain.
                order.sort_by_key(|&i| (s.enemies[i].pos.dist_sq(origin).raw(), s.enemies[i].id.0));
                let mut targets = vec![first];
                targets.extend(order.into_iter().take((n as usize).saturating_sub(1)));
                for ti in targets {
                    instant_damage += apply_weapon_hit(
                        &mut s.enemies[ti],
                        wdef.damage,
                        wdef.damage_type,
                        wmult,
                        cond,
                        &on_hit,
                        ability,
                        tank_pos,
                        &mut accum,
                    );
                }
                any_instant_damage = true;
            }
        }

        // Apply this weapon's deferred ability effects (life/mana drain heal,
        // hazard placement). A no-op for projectile attacks (their abilities
        // resolve at impact) and for ability-less weapons.
        accum.flush(s, ability, wdef.damage_type);

        // Effective cooldown is reduced by the attack-speed modifier.
        let asm = s.modifiers.attack_speed_mult();
        let cd = Fixed::from_int(wdef.cooldown_ticks as i64)
            .div(asm)
            .floor_to_int()
            .max(1) as u32;
        s.weapons[wi].next_fire_tick = s.tick + cd;
    }

    // Record instant-attack damage (scoreboard + Bloodmoney) now that no enemy
    // borrow is held.
    s.record_player_damage(instant_damage);

    for mut p in new_projectiles {
        p.id = s.alloc_entity_id();
        s.projectiles.push(p);
    }

    // Instant attacks (Area/Wave/Bounce) can kill: reap the dead now so bounty
    // is awarded this tick, preserving id order. Fire-stacked deaths explode.
    if any_instant_damage {
        crate::status::reap_dead(s);
    }
}

/// Phase 5: move each projectile toward its target by `speed`
/// (`Vec2::step_toward`). On arrival (reached target pos) apply damage: single
/// target hits the target enemy; `splash_radius > 0` hits all enemies within
/// that radius of the impact point. Multiply damage by
/// `content::damage_multiplier(damage_type, enemy_def.armor_class)`. Subtract
/// from enemy hp; if hp <= 0 remove the enemy and push its `def` to
/// `s.pending_kills`. Remove projectiles that have arrived; if a projectile's
/// target enemy no longer exists, detonate at `last_target_pos` (splash) or just
/// remove it (single target). Keep `s.enemies` ordered by id.
pub(crate) fn advance_projectiles(s: &mut ArenaState) {
    // Move each projectile toward its target, detect arrival, and on arrival
    // apply damage. We collect "impacts" while iterating (so the enemy borrow
    // is read-only) then apply hp changes / removals afterward in a stable pass.
    struct Impact {
        point: Vec2,
        target: EntityId,
        damage: i64,
        damage_type: u8,
        splash_radius: Fixed,
        on_hit: crate::content::StatusOnHit,
        ability: WeaponAbility,
    }

    let mut impacts: Vec<Impact> = Vec::new();
    let mut survivors: Vec<Projectile> = Vec::with_capacity(s.projectiles.len());

    // Take ownership of the projectile list to iterate without aliasing `s`.
    let projectiles = std::mem::take(&mut s.projectiles);
    for mut p in projectiles {
        // Resolve current target position (if the enemy still exists).
        let target_pos = s.enemies.iter().find(|e| e.id == p.target).map(|e| e.pos);

        match target_pos {
            Some(tpos) => {
                p.last_target_pos = tpos;
                let moved = p.pos.step_toward(tpos, p.speed);
                if moved == tpos {
                    // Arrived: detonate at the target position.
                    impacts.push(Impact {
                        point: tpos,
                        target: p.target,
                        damage: p.damage,
                        damage_type: p.damage_type,
                        splash_radius: p.splash_radius,
                        on_hit: p.on_hit,
                        ability: p.ability,
                    });
                } else {
                    p.pos = moved;
                    survivors.push(p);
                }
            }
            None => {
                // Target gone: splash projectiles detonate at last known pos;
                // single-target projectiles simply vanish.
                if p.splash_radius > Fixed::ZERO {
                    impacts.push(Impact {
                        point: p.last_target_pos,
                        target: p.target,
                        damage: p.damage,
                        damage_type: p.damage_type,
                        splash_radius: p.splash_radius,
                        on_hit: p.on_hit,
                        ability: p.ability,
                    });
                }
                // either way, projectile is removed (not pushed to survivors).
            }
        }
    }

    s.projectiles = survivors;

    // Apply impacts in order. Track kills to push their defs after. Final damage
    // = base × armor-matrix × modifier-stack × status-vulnerability
    // (`docs/05 §5.3`). Bosses are immune to weapon fire (only `Clear` hurts
    // them), and each hit also applies the weapon's on-hit status.
    // Conditional bonuses are evaluated live at impact against the target's status.
    let cond = CondDamage::of(s);
    let tank_pos = s.tank.pos;
    let mut impact_damage: i64 = 0;
    for imp in impacts {
        let mod_mult = Fixed::ONE; // weapon multiplier was baked into projectile damage at fire time
                                   // Each impact's signature ability accumulates over the enemies it hits,
                                   // then flushes (tank heal / mana / hazard) once.
        let mut accum = AbilityAccum::default();
        if imp.splash_radius > Fixed::ZERO {
            let radius_sq = imp.splash_radius.mul(imp.splash_radius);
            for e in s.enemies.iter_mut() {
                if imp.point.dist_sq(e.pos) <= radius_sq {
                    impact_damage += apply_weapon_hit(
                        e,
                        imp.damage,
                        imp.damage_type,
                        mod_mult,
                        cond,
                        &imp.on_hit,
                        imp.ability,
                        tank_pos,
                        &mut accum,
                    );
                }
            }
        } else if let Some(e) = s.enemies.iter_mut().find(|e| e.id == imp.target) {
            impact_damage += apply_weapon_hit(
                e,
                imp.damage,
                imp.damage_type,
                mod_mult,
                cond,
                &imp.on_hit,
                imp.ability,
                tank_pos,
                &mut accum,
            );
        }
        accum.flush(s, imp.ability, imp.damage_type);
    }
    s.record_player_damage(impact_damage);

    // Remove dead enemies, preserving id order; Fire-stacked deaths explode.
    crate::status::reap_dead(s);
}

/// Phase 6: move each enemy toward the tank (origin) by `EnemyDef::move_speed`
/// (`Vec2::step_toward`). On contact (reaches origin) deal `contact_damage` to
/// `s.tank.hp` and remove the enemy (no bounty for self-destruct). Keep
/// `s.enemies` ordered by id.
pub(crate) fn move_enemies(s: &mut ArenaState) {
    let tank_pos = s.tank.pos;
    let mut survivors: Vec<Enemy> = Vec::with_capacity(s.enemies.len());

    // Contact damage scales with match time on the SAME curve as enemy HP
    // (`content::enemy_hp_mult`): late game gets deadlier, not just tankier
    // (`docs/05`; mirrors the spawn-time HP scaling in `waves::spawn`).
    let dmg_mult = content::enemy_hp_mult(s.tick);

    for mut e in std::mem::take(&mut s.enemies) {
        // Stunned / frozen enemies can't move this tick.
        if crate::status::is_immobile(&e) {
            survivors.push(e);
            continue;
        }
        let edef = &content::ENEMIES[e.def as usize];
        // Movement is slowed by Frost stacks.
        let speed = Fixed::from_int(edef.move_speed).mul(crate::status::move_speed_mult(&e));
        let contact = dmg_mult.scale_i64(edef.contact_damage);
        let moved = e.pos.step_toward(tank_pos, speed);
        if moved == tank_pos {
            if edef.boss {
                // BOSS — PERSISTENT ATTRITION FIGHT (not a one-shot self-destruct).
                // The boss does NOT despawn on contact: it plants at the tank and
                // grinds it with a CADENCED contact hit (every `BOSS_CONTACT_CADENCE`
                // ticks) until the player kills it with `Clear` (the only thing that
                // hurts it) or the tank dies. This is what makes the 30-min climax a
                // real multi-Clear RACE — survival is no longer a single dodge coin
                // flip on one burst; the player must out-Clear the boss's sustained
                // DPS while the escort piles on. Determinism: the cadence is a pure
                // function of `s.tick` (no wall-clock, no new RNG); dodge/armor/shield
                // are still honored per hit inside `hit_tank`.
                e.pos = moved; // pin at the tank
                if s.tick.is_multiple_of(content::BOSS_CONTACT_CADENCE) {
                    crate::defense::hit_tank(s, contact);
                }
                survivors.push(e);
            } else {
                // Normal enemy: self-destruct on contact (no bounty), one hit, gone.
                crate::defense::hit_tank(s, contact);
            }
        } else {
            e.pos = moved;
            survivors.push(e);
        }
    }

    s.enemies = survivors;
}

/// Phase: tick persistent hazards (land mines / burning oil). Each hazard pulses
/// `dmg` (× armor matrix × status vulnerability) to every non-boss enemy within
/// its radius, decrements its lifetime, and expires at zero. Hazards are
/// processed in id order and enemies in id order, so the pass is deterministic.
/// Hazard damage is a player source (scoreboard / Bloodmoney) and kills route
/// through the shared death path (`reap_dead`), so bounty and Fire explosions
/// still fire.
pub(crate) fn tick_hazards(s: &mut ArenaState) {
    if s.hazards.is_empty() {
        return;
    }
    let cond = CondDamage::of(s);
    let tank_pos = s.tank.pos;
    let mut total: i64 = 0;
    let mut any = false;
    let mut hazards = std::mem::take(&mut s.hazards);
    for h in hazards.iter_mut() {
        if h.ticks_left == 0 {
            continue;
        }
        let radius = Fixed::from_int(h.radius);
        let r2 = radius.mul(radius);
        // A hazard carries no on-hit status and no chained ability.
        let mut accum = AbilityAccum::default();
        for e in s.enemies.iter_mut() {
            if h.pos.dist_sq(e.pos) <= r2 {
                total += apply_weapon_hit(
                    e,
                    h.dmg,
                    h.damage_type,
                    Fixed::ONE,
                    cond,
                    &content::StatusOnHit::NONE,
                    WeaponAbility::None,
                    tank_pos,
                    &mut accum,
                );
                any = true;
            }
        }
        h.ticks_left -= 1;
    }
    // Drop expired hazards, preserving id order.
    hazards.retain(|h| h.ticks_left > 0);
    s.hazards = hazards;
    s.record_player_damage(total);
    if any {
        crate::status::reap_dead(s);
    }
}

/// Phase: periodic damage/poison AURA centered on the tank (source: Blight Aura).
/// A per-tank integer counter (`tank.aura_tick`) advances every tick; each time it
/// reaches the cadence it RESETS to 0 and the aura fires: deal `aura_damage`
/// (× armor matrix × status vulnerability) AND apply the tank's `aura_poison_*`
/// Poison DoT to every non-boss enemy within `aura_range`, in STABLE id order (the
/// live `enemies` vec is id-ordered). The cadence is a pure integer count of ticks
/// (`aura_cadence`), NOT wall-clock. Aura damage is a player source (scoreboard /
/// Bloodmoney) and kills route through the shared death path (`reap_dead`), so
/// bounty and Fire explosions still fire. Integer/Fixed only; no RNG. A no-op when
/// the aura is unconfigured (`aura_cadence == 0`).
pub(crate) fn tick_aura(s: &mut ArenaState) {
    if s.tank.aura_cadence == 0 {
        return;
    }
    // Advance the integer cadence counter; fire (and reset) on the boundary.
    s.tank.aura_tick += 1;
    if s.tank.aura_tick < s.tank.aura_cadence {
        return;
    }
    s.tank.aura_tick = 0;

    let dmg = s.tank.aura_damage;
    let range = Fixed::from_int(s.tank.aura_range);
    let r2 = range.mul(range);
    let cond = CondDamage::of(s);
    let tank_pos = s.tank.pos;
    let poison = if s.tank.aura_poison_dps > 0 && s.tank.aura_poison_ticks > 0 {
        content::StatusOnHit {
            poison_dps: s.tank.aura_poison_dps,
            poison_ticks: s.tank.aura_poison_ticks,
            ..content::StatusOnHit::NONE
        }
    } else {
        content::StatusOnHit::NONE
    };
    let mut total: i64 = 0;
    let mut any = false;
    let mut accum = AbilityAccum::default();
    // `s.enemies` is id-ordered ⇒ this AoE pass is run-to-run stable.
    for e in s.enemies.iter_mut() {
        if tank_pos.dist_sq(e.pos) <= r2 {
            total += apply_weapon_hit(
                e,
                dmg,
                content::DMG_MAGIC,
                Fixed::ONE,
                cond,
                &poison,
                WeaponAbility::None,
                tank_pos,
                &mut accum,
            );
            any = true;
        }
    }
    s.record_player_damage(total);
    if any {
        crate::status::reap_dead(s);
    }
}

/// Tuning for summoned allies (Larvae / Spores).
const MAX_MINIONS: usize = 16; // global cap on living minions
const MINION_LIFETIME: u32 = 450; // ~15s before a minion vanishes
const MINION_ATTACK_CD: u32 = 30; // ticks between a minion's strikes (~1/s)
const MINION_REACH: i64 = 160; // melee reach (world units)
const MINION_SPEED: i64 = 18; // step toward the target per tick (out of reach)

/// Phase 6d: summoned allies act. Each minion (stable id order) targets the
/// nearest non-boss enemy: in reach it strikes on its attack cooldown, else it
/// steps toward the target. Strike damage scales with match time on the same
/// curve as enemy HP/damage and routes through the shared death path, so a
/// minion's kills award bounty and trigger Fire explosions. Expired minions are
/// removed. Deterministic: integer/Fixed math, stable id order, no RNG.
pub(crate) fn tick_minions(s: &mut ArenaState) {
    if s.minions.is_empty() {
        return;
    }
    let now = s.tick;
    s.minions.retain(|m| now < m.expire_tick);
    if s.minions.is_empty() {
        return;
    }

    let dmg_mult = content::enemy_hp_mult(now);
    let reach = Fixed::from_int(MINION_REACH);
    let reach2 = reach.mul(reach);
    let speed = Fixed::from_int(MINION_SPEED);

    // Move minions / decide strikes without holding an enemy borrow.
    let mut minions = std::mem::take(&mut s.minions);
    let mut strikes: Vec<(EntityId, i64, u8)> = Vec::new(); // (target, scaled dmg, dtype)
    for m in minions.iter_mut() {
        // Nearest non-boss enemy; ties resolve to the lower id (id-ordered list).
        let mut best: Option<(EntityId, Vec2, Fixed)> = None;
        for e in s.enemies.iter() {
            if content::ENEMIES[e.def as usize].boss {
                continue;
            }
            let d2 = m.pos.dist_sq(e.pos);
            if best.is_none_or(|(_, _, bd)| d2 < bd) {
                best = Some((e.id, e.pos, d2));
            }
        }
        if let Some((tid, tpos, d2)) = best {
            if d2 <= reach2 {
                if now >= m.next_attack_tick {
                    m.next_attack_tick = now + MINION_ATTACK_CD;
                    strikes.push((tid, dmg_mult.scale_i64(m.damage), m.damage_type));
                }
            } else {
                m.pos = m.pos.step_toward(tpos, speed);
            }
        }
    }
    s.minions = minions;

    if strikes.is_empty() {
        return;
    }
    // Resolve strikes (deterministic: minion id order). A minion strike carries
    // no on-hit status and no chained ability.
    let cond = CondDamage::of(s);
    let tank_pos = s.tank.pos;
    let mut accum = AbilityAccum::default();
    let mut total: i64 = 0;
    for (tid, dmg, dtype) in strikes {
        if let Some(e) = s.enemies.iter_mut().find(|e| e.id == tid) {
            total += apply_weapon_hit(
                e,
                dmg,
                dtype,
                Fixed::ONE,
                cond,
                &content::StatusOnHit::NONE,
                WeaponAbility::None,
                tank_pos,
                &mut accum,
            );
        }
    }
    s.record_player_damage(total);
    crate::status::reap_dead(s);
}

/// Phase 6c: ENEMY-side ranged attacks. Each enemy whose def carries
/// `EnemyAbility::RangedAttack` and is within its `range` of the tank fires on a
/// deterministic per-enemy phase — `(tick + enemy id) % cooldown == 0` — dealing
/// `damage` of `damage_type` to the tank through the defensive layer
/// (`defense::hit_tank`, which honors dodge / mana-shield / armor). The phase is
/// derived from existing deterministic quantities (tick + stable id), so it needs
/// NO per-enemy runtime state and never touches the snapshot/checksum. Enemies are
/// visited in stable id order. Damage scales with match time on the same curve as
/// contact damage / HP (`content::enemy_hp_mult`).
pub(crate) fn enemy_ranged_attacks(s: &mut ArenaState) {
    let tank_pos = s.tank.pos;
    let dmg_mult = content::enemy_hp_mult(s.tick);

    // Resolve which enemies fire (and for how much) without holding an enemy
    // borrow across the `defense::hit_tank` mutation. Stable id order.
    let mut hits: Vec<i64> = Vec::new();
    for e in s.enemies.iter() {
        // Stunned / frozen enemies can't attack this tick.
        if crate::status::is_immobile(e) {
            continue;
        }
        let edef = &content::ENEMIES[e.def as usize];
        if let content::EnemyAbility::RangedAttack {
            range,
            cooldown_ticks,
            damage,
            damage_type: _,
        } = edef.ability
        {
            if cooldown_ticks == 0 {
                continue;
            }
            let reach = Fixed::from_int(range);
            if tank_pos.dist_sq(e.pos) > reach.mul(reach) {
                continue; // not yet within standoff range
            }
            // Deterministic per-enemy firing phase.
            if (s.tick.wrapping_add(e.id.0)) % cooldown_ticks != 0 {
                continue;
            }
            // Time-scaled raw damage; the tank's flat armor / dodge / mana-shield
            // are applied inside `defense::hit_tank`. `damage_type` is reserved
            // for future tank armor-class matrixing and render telemetry.
            let raw = dmg_mult.scale_i64(damage);
            hits.push(raw);
        }
    }

    for raw in hits {
        crate::defense::hit_tank(s, raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ArenaState;

    fn blank_state() -> ArenaState {
        // ArenaState::new seeds one starting Bow weapon. We replace the weapon
        // list per-test as needed. We never call sim::step() (would hit other
        // agents' todo!() stubs); only the combat functions under test.
        ArenaState::new(0xABCDEF, 0)
    }

    fn mk_enemy(s: &mut ArenaState, def: u16, hp: i64, pos: Vec2) -> EntityId {
        let id = s.alloc_entity_id();
        s.enemies.push(Enemy::new(id, def, hp, pos));
        id
    }

    // ---- fire_weapons -------------------------------------------------------

    #[test]
    fn fire_only_when_enemy_in_range() {
        let mut s = blank_state();
        // Starting weapon is the Bow (def 0, range 900). Put an enemy far out.
        s.weapons.clear();
        let wid = s.alloc_entity_id();
        s.weapons.push(WeaponInstance {
            instance_id: wid,
            def: 0,
            next_fire_tick: 0,
        });

        // Enemy out of range (1500 > 900) → no fire, no cooldown advance.
        mk_enemy(
            &mut s,
            0,
            200,
            Vec2::new(Fixed::from_int(1500), Fixed::ZERO),
        );
        fire_weapons(&mut s);
        assert!(
            s.projectiles.is_empty(),
            "should not fire at out-of-range enemy"
        );
        assert_eq!(s.weapons[0].next_fire_tick, 0, "cooldown must not advance");

        // Enemy in range (300 < 900) → fires one projectile, advances cooldown.
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(300), Fixed::ZERO));
        fire_weapons(&mut s);
        assert_eq!(s.projectiles.len(), 1, "should fire at in-range enemy");
        let p = &s.projectiles[0];
        assert_eq!(p.damage, 75);
        assert_eq!(p.damage_type, content::DMG_PIERCING);
        assert_eq!(p.splash_radius, Fixed::ZERO, "Bow is single-target");
        // cooldown_ticks for Bow = 30, tick = 0.
        assert_eq!(s.weapons[0].next_fire_tick, 30);
    }

    #[test]
    fn fire_respects_cooldown() {
        let mut s = blank_state();
        s.weapons.clear();
        let wid = s.alloc_entity_id();
        s.weapons.push(WeaponInstance {
            instance_id: wid,
            def: 0,
            next_fire_tick: 100,
        });
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        s.tick = 50; // 50 < 100, not ready.
        fire_weapons(&mut s);
        assert!(s.projectiles.is_empty(), "weapon on cooldown must not fire");
    }

    #[test]
    fn modifiers_scale_dealt_damage() {
        // The full weapon multiplier is resolved at FIRE time and baked into the
        // projectile's carried damage. With +100% global damage the Bow's baked
        // damage is exactly 2× its base.
        let fire_baked = |add_global: Fixed| {
            let mut s = blank_state();
            s.weapons.clear();
            s.modifiers.add_global = add_global;
            let wid = s.alloc_entity_id();
            s.weapons.push(WeaponInstance {
                instance_id: wid,
                def: 0,
                next_fire_tick: 0,
            });
            mk_enemy(
                &mut s,
                0,
                1_000_000,
                Vec2::new(Fixed::from_int(100), Fixed::ZERO),
            );
            fire_weapons(&mut s);
            s.projectiles[0].damage
        };
        let base = fire_baked(Fixed::ZERO); // Bow base = 75
        let doubled = fire_baked(Fixed::from_ratio(1, 1)); // +100%
        assert_eq!(base, content::WEAPONS[0].damage);
        assert_eq!(doubled, base * 2, "modifier did not scale combat damage");
    }

    #[test]
    fn per_scope_damage_only_affects_matching_weapons() {
        use content::{attack_scope_id, range_scope_id, rarity_scope_id, Attack};
        // Bow (def 0): SingleTarget, range 900 (long), rarity 0 (common).
        let bow = &content::WEAPONS[0];
        assert_eq!(attack_scope_id(bow.attack), 0);
        assert_eq!(range_scope_id(bow.range), 7); // 900 → long
        assert_eq!(rarity_scope_id(bow.rarity), 8);

        let mut s = blank_state();
        // +100% to SINGLE-TARGET weapons → applies to the Bow.
        s.modifiers.add_by_scope[0] = Fixed::from_ratio(1, 1);
        assert_eq!(s.modifiers.weapon_damage_mult(bow).scale_i64(1000), 2000);

        // +100% to SPLASH weapons (scope 1) → does NOT apply to the Bow.
        let mut s2 = blank_state();
        s2.modifiers.add_by_scope[1] = Fixed::from_ratio(1, 1);
        assert_eq!(s2.modifiers.weapon_damage_mult(bow), Fixed::ONE);
        // …but DOES apply to the Mortar (def 1, Splash).
        let mortar = &content::WEAPONS[1];
        assert_eq!(attack_scope_id(mortar.attack), 1);
        assert!(matches!(mortar.attack, Attack::Splash(_)));
        assert_eq!(
            s2.modifiers.weapon_damage_mult(mortar).scale_i64(1000),
            2000
        );
    }

    #[test]
    fn attack_speed_modifier_shortens_cooldown() {
        let mut s = blank_state();
        s.weapons.clear();
        let wid = s.alloc_entity_id();
        s.weapons.push(WeaponInstance {
            instance_id: wid,
            def: 0,
            next_fire_tick: 0,
        });
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        s.modifiers.attack_speed = Fixed::from_ratio(1, 1); // +100% ⇒ ×2 ⇒ cd 30→15
        fire_weapons(&mut s);
        assert_eq!(s.weapons[0].next_fire_tick, 15);
    }

    // ---- advance_projectiles ------------------------------------------------

    #[test]
    fn projectile_kills_and_records_def() {
        let mut s = blank_state();
        s.weapons.clear();
        // Enemy with low hp so a single hit kills it.
        let pos = Vec2::new(Fixed::from_int(10), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 100, pos);
        // Single-target projectile already adjacent (speed huge so it arrives).
        let pid = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: 200, // piercing vs armor 0 → 2x = 400, lethal
            damage_type: content::DMG_PIERCING,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert!(s.enemies.is_empty(), "enemy should be dead");
        assert_eq!(s.pending_kills, vec![0u16], "dead enemy def pushed");
        assert!(s.projectiles.is_empty(), "arrived projectile removed");
    }

    #[test]
    fn projectile_in_flight_survives_and_moves() {
        let mut s = blank_state();
        s.weapons.clear();
        let pos = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 200, pos);
        let pid = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: 75,
            damage_type: content::DMG_PIERCING,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(10), // far from arriving
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert_eq!(s.projectiles.len(), 1, "still in flight");
        assert_eq!(s.enemies.len(), 1, "enemy unharmed yet");
        assert_eq!(s.enemies[0].hp, 200);
        // Moved 10 units along +x.
        assert_eq!(s.projectiles[0].pos.x, Fixed::from_int(10));
    }

    #[test]
    fn splash_hits_multiple_enemies() {
        let mut s = blank_state();
        s.weapons.clear();
        let impact = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        // Three enemies: two within 300 of impact, one far away.
        let near1 = mk_enemy(&mut s, 0, 100, impact); // distance 0
        let _near2 = mk_enemy(&mut s, 0, 100, Vec2::new(Fixed::from_int(250), Fixed::ZERO)); // 150 away
        let _far = mk_enemy(
            &mut s,
            0,
            100,
            Vec2::new(Fixed::from_int(1000), Fixed::ZERO),
        ); // 900 away
        let pid = s.alloc_entity_id();
        // Mortar: siege 300, splash 300. Siege vs armor 0 → 1x = 300 dmg, lethal.
        s.projectiles.push(Projectile {
            id: pid,
            pos: impact,
            target: near1,
            last_target_pos: impact,
            damage: 300,
            damage_type: content::DMG_SIEGE,
            splash_radius: Fixed::from_int(300),
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        // Two near enemies dead, far one survives.
        assert_eq!(s.enemies.len(), 1, "only the far enemy survives");
        assert_eq!(s.pending_kills.len(), 2, "two kills recorded");
    }

    #[test]
    fn splash_detonates_at_last_pos_when_target_gone() {
        let mut s = blank_state();
        s.weapons.clear();
        let impact = Vec2::new(Fixed::from_int(50), Fixed::ZERO);
        // A bystander near the last known position, but the actual target id
        // does not exist anymore.
        let _bystander = mk_enemy(&mut s, 0, 100, impact);
        let pid = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: EntityId(99999), // nonexistent
            last_target_pos: impact,
            damage: 300,
            damage_type: content::DMG_SIEGE,
            splash_radius: Fixed::from_int(300),
            speed: Fixed::from_int(10),
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert!(s.enemies.is_empty(), "bystander killed by detonation");
        assert_eq!(s.pending_kills, vec![0u16]);
        assert!(s.projectiles.is_empty());
    }

    #[test]
    fn single_target_vanishes_when_target_gone() {
        let mut s = blank_state();
        s.weapons.clear();
        let _bystander = mk_enemy(&mut s, 0, 100, Vec2::ZERO);
        let pid = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: EntityId(99999),
            last_target_pos: Vec2::ZERO,
            damage: 1000,
            damage_type: content::DMG_PIERCING,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(10),
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert_eq!(
            s.enemies.len(),
            1,
            "bystander untouched by single-target miss"
        );
        assert!(s.projectiles.is_empty(), "stale projectile removed");
        assert!(s.pending_kills.is_empty());
    }

    // ---- move_enemies -------------------------------------------------------

    #[test]
    fn enemy_reaching_tank_deals_contact_damage() {
        let mut s = blank_state();
        s.weapons.clear();
        let hp0 = s.tank.hp;
        // Enemy 0 right next to origin; move_speed 8 ≥ distance → arrives.
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(3), Fixed::ZERO));
        move_enemies(&mut s);
        assert!(s.enemies.is_empty(), "enemy removed on contact");
        assert_eq!(s.tank.hp, hp0 - 500, "contact_damage applied");
        assert!(s.pending_kills.is_empty(), "self-destruct grants no bounty");
    }

    #[test]
    fn contact_damage_scales_with_match_time() {
        // Same enemy, two ticks: at tick 0 contact damage is the raw value; deep
        // into the match it is multiplied by `enemy_hp_mult`.
        let edef = &content::ENEMIES[0];
        let base = edef.contact_damage;

        let mut early = blank_state();
        early.weapons.clear();
        early.tick = 0;
        let hp0 = early.tank.hp;
        mk_enemy(
            &mut early,
            0,
            200,
            Vec2::new(Fixed::from_int(3), Fixed::ZERO),
        );
        move_enemies(&mut early);
        assert_eq!(early.tank.hp, hp0 - base, "tick 0 deals raw contact damage");

        // At the 2nd scaling step (×2 baseline) contact damage doubles.
        let mut late = blank_state();
        late.weapons.clear();
        late.tick = content::SCALE_STEP_2_TICK;
        let mult = content::enemy_hp_mult(late.tick);
        let expected = mult.scale_i64(base);
        assert!(expected > base, "scaling must increase contact damage");
        let hp_late = late.tank.hp;
        mk_enemy(
            &mut late,
            0,
            200,
            Vec2::new(Fixed::from_int(3), Fixed::ZERO),
        );
        move_enemies(&mut late);
        assert_eq!(
            late.tank.hp,
            hp_late - expected,
            "late contact damage scaled"
        );
    }

    #[test]
    fn enemy_marches_toward_tank() {
        let mut s = blank_state();
        s.weapons.clear();
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        let hp0 = s.tank.hp;
        move_enemies(&mut s);
        assert_eq!(s.enemies.len(), 1, "still marching");
        // Moved 8 units toward origin (move_speed 8) along -x.
        assert_eq!(s.enemies[0].pos.x, Fixed::from_int(92));
        assert_eq!(s.tank.hp, hp0, "no contact yet");
    }

    #[test]
    fn boss_persists_on_contact_and_grinds_on_cadence() {
        // The boss does NOT self-destruct: on reaching the tank it stays planted
        // and lands a contact hit only on its cadence ticks. A non-cadence tick
        // pins it with NO damage; a cadence tick deals one (scaled) contact hit.
        // Use the boss-phase tick so the ×11 multiplier (and a real cadence) apply.
        let cadence = content::BOSS_CONTACT_CADENCE;
        let boss_def = content::BOSS;
        let raw = content::ENEMIES[boss_def as usize].contact_damage;

        // Off-cadence tick: boss arrives, plants, deals nothing.
        let mut off = blank_state();
        off.weapons.clear();
        off.tick = content::BOSS_SPAWN_TICK + 1; // not a multiple of cadence (16)
        assert_ne!(off.tick % cadence, 0);
        let hp_off = off.tank.hp;
        mk_enemy(
            &mut off,
            boss_def,
            33_000_000,
            Vec2::new(Fixed::from_int(2), Fixed::ZERO),
        );
        move_enemies(&mut off);
        assert_eq!(
            off.enemies.len(),
            1,
            "boss persists on contact (no self-destruct)"
        );
        assert_eq!(off.enemies[0].pos, Vec2::ZERO, "boss planted on the tank");
        assert_eq!(off.tank.hp, hp_off, "no damage on an off-cadence tick");

        // On-cadence tick: boss is still planted but now lands one scaled hit.
        let mut on = blank_state();
        on.weapons.clear();
        on.tick = content::BOSS_SPAWN_TICK; // a multiple of cadence
        assert_eq!(on.tick % cadence, 0);
        let expected = content::enemy_hp_mult(on.tick).scale_i64(raw);
        let hp_on = on.tank.hp;
        mk_enemy(
            &mut on,
            boss_def,
            33_000_000,
            Vec2::new(Fixed::from_int(2), Fixed::ZERO),
        );
        move_enemies(&mut on);
        assert_eq!(
            on.enemies.len(),
            1,
            "boss still present after a contact hit"
        );
        assert_eq!(
            on.tank.hp,
            hp_on - expected,
            "cadence tick deals one scaled boss hit"
        );
        assert!(on.pending_kills.is_empty(), "boss contact grants no bounty");
    }

    // ---- determinism --------------------------------------------------------

    #[test]
    fn fire_weapons_is_deterministic_same_seed() {
        // Multiple in-range candidates so the random target pick is exercised.
        let mut a = blank_state();
        a.weapons.clear();
        let wid = a.alloc_entity_id();
        a.weapons.push(WeaponInstance {
            instance_id: wid,
            def: 0,
            next_fire_tick: 0,
        });
        for i in 0..5 {
            mk_enemy(
                &mut a,
                0,
                200,
                Vec2::new(Fixed::from_int(100 + i), Fixed::from_int(i)),
            );
        }
        let mut b = a.clone();

        fire_weapons(&mut a);
        fire_weapons(&mut b);
        assert_eq!(
            a.projectiles, b.projectiles,
            "same seed → same target chosen"
        );
        assert_eq!(a.rng_targeting.state(), b.rng_targeting.state());
        assert_eq!(a.projectiles.len(), 1);
    }

    #[test]
    fn weapon_applies_on_hit_status() {
        // Poison Bow (def 3) applies poison on hit through advance_projectiles.
        let mut s = blank_state();
        s.weapons.clear();
        let pos = Vec2::new(Fixed::from_int(10), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 100_000, pos); // survives the impact
        let pid = s.alloc_entity_id();
        let pb = &content::WEAPONS[3];
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: pb.damage,
            damage_type: pb.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: pb.on_hit,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert_eq!(s.enemies[0].status.poison_dps, 20, "poison applied on hit");
        assert_eq!(s.enemies[0].status.poison_ticks, 90);
    }

    #[test]
    fn conditional_damage_only_applies_to_matching_status() {
        // +100% damage to stunned; a stunned and an un-stunned enemy take a hit.
        let mut s = blank_state();
        s.modifiers.vs_stunned = Fixed::from_ratio(1, 1);
        let cond = CondDamage::of(&s);
        let mut plain = Enemy::new(EntityId(2), 0, 1_000_000, Vec2::ZERO);
        let mut stunned = Enemy::new(EntityId(1), 0, 1_000_000, Vec2::ZERO);
        stunned.status.stun_ticks = 10;
        // Piercing vs armor 0 = 2× matrix; base 100 ⇒ plain takes 200.
        apply_weapon_hit(
            &mut plain,
            100,
            content::DMG_PIERCING,
            Fixed::ONE,
            cond,
            &content::StatusOnHit::NONE,
            WeaponAbility::None,
            Vec2::ZERO,
            &mut AbilityAccum::default(),
        );
        apply_weapon_hit(
            &mut stunned,
            100,
            content::DMG_PIERCING,
            Fixed::ONE,
            cond,
            &content::StatusOnHit::NONE,
            WeaponAbility::None,
            Vec2::ZERO,
            &mut AbilityAccum::default(),
        );
        assert_eq!(
            1_000_000 - plain.hp,
            200,
            "no conditional bonus on un-stunned"
        );
        assert_eq!(1_000_000 - stunned.hp, 400, "+100% vs stunned doubles it");
    }

    #[test]
    fn poison_damage_modifier_scales_applied_dot() {
        // +50% Poison damage bakes into the on-hit poison the Poison Bow applies.
        let mut s = blank_state();
        s.weapons.clear();
        s.modifiers.poison_dmg_mult = Fixed::ONE + Fixed::from_ratio(1, 2); // ×1.5
        let pos = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        mk_enemy(&mut s, 0, 200, pos);
        give_weapon(&mut s, 3); // Poison Bow: poison_dps 20
        fire_weapons(&mut s);
        assert_eq!(s.projectiles.len(), 1);
        assert_eq!(
            s.projectiles[0].on_hit.poison_dps, 30,
            "20 × 1.5 baked at fire time"
        );
    }

    #[test]
    fn stun_duration_modifier_scales_applied_stun() {
        // A weapon with stun on-hit gets its stun extended by the modifier.
        let mut s = blank_state();
        s.modifiers.stun_dur_mult = Fixed::from_int(2); // ×2
        let on_hit = content::StatusOnHit {
            stun_ticks: 30,
            ..content::StatusOnHit::NONE
        };
        let scaled = s.modifiers.scale_on_hit(on_hit);
        assert_eq!(scaled.stun_ticks, 60);
        // Identity multiplier leaves it untouched.
        let s2 = blank_state();
        assert_eq!(s2.modifiers.scale_on_hit(on_hit).stun_ticks, 30);
    }

    #[test]
    fn self_scaling_damage_grows_with_weapon_count() {
        // "+1% Piercing Damage per Bow" (def 0, piercing). Own 3 Bows.
        let mut s = blank_state();
        s.weapons.clear();
        s.modifiers
            .weapon_count_scaling
            .push(crate::state::WeaponCountScale {
                weapon_def: 0,
                dmg_type: content::DMG_PIERCING,
                per: Fixed::from_ratio(1, 100),
            });
        for _ in 0..3 {
            give_weapon(&mut s, 0);
        }
        mk_enemy(
            &mut s,
            0,
            1_000_000,
            Vec2::new(Fixed::from_int(100), Fixed::ZERO),
        );
        fire_weapons(&mut s);
        assert_eq!(s.projectiles.len(), 3, "all three Bows fire");
        // add_self = 3 × 1% = 3% ⇒ 75 × 1.03 = 77 (floored).
        assert_eq!(s.projectiles[0].damage, 77);

        // Wrong damage type is unaffected: a Siege rule does nothing for the Bow.
        let mut s2 = blank_state();
        s2.weapons.clear();
        s2.modifiers
            .weapon_count_scaling
            .push(crate::state::WeaponCountScale {
                weapon_def: 0,
                dmg_type: content::DMG_SIEGE,
                per: Fixed::from_ratio(1, 100),
            });
        give_weapon(&mut s2, 0);
        mk_enemy(
            &mut s2,
            0,
            1_000_000,
            Vec2::new(Fixed::from_int(100), Fixed::ZERO),
        );
        fire_weapons(&mut s2);
        assert_eq!(
            s2.projectiles[0].damage,
            content::WEAPONS[0].damage,
            "no piercing scaling"
        );
    }

    #[test]
    fn projectile_damage_accumulates_on_scoreboard() {
        let mut s = blank_state();
        s.weapons.clear();
        let pos = Vec2::new(Fixed::from_int(10), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 100_000, pos); // survives
        let pid = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: 200, // piercing vs armor 0 → 2× = 400 actually dealt
            damage_type: content::DMG_PIERCING,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert_eq!(
            s.total_damage_dealt, 400,
            "actual armor-scaled damage scored"
        );
    }

    #[test]
    fn instant_attack_damage_accumulates_and_pays_bloodmoney() {
        // Immolation (def 7) = Area(300), 80 chaos. Two in-range enemies.
        let mut s = blank_state();
        s.weapons.clear();
        s.economy.gold_per_damage = Fixed::from_ratio(1, 4); // exactly representable
        give_weapon(&mut s, 7);
        mk_enemy(
            &mut s,
            0,
            10_000,
            Vec2::new(Fixed::from_int(100), Fixed::ZERO),
        );
        mk_enemy(
            &mut s,
            0,
            10_000,
            Vec2::new(Fixed::from_int(250), Fixed::ZERO),
        );
        let gold0 = s.economy.gold;
        fire_weapons(&mut s);
        // Chaos vs armor 0 = 1×; 80 each × 2 enemies = 160 total.
        assert_eq!(s.total_damage_dealt, 160);
        assert_eq!(s.economy.gold, gold0 + 40, "160 × 1/4 = 40 gold");
    }

    #[test]
    fn boss_is_immune_to_weapon_fire() {
        let mut s = blank_state();
        s.weapons.clear();
        let pos = Vec2::new(Fixed::from_int(10), Fixed::ZERO);
        let bid = mk_enemy(&mut s, content::BOSS, 10_000_000, pos);
        let _ = bid;
        let pid = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: s.enemies[0].id,
            last_target_pos: pos,
            damage: 1_000_000,
            damage_type: content::DMG_PIERCING,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: content::WeaponAbility::None,
        });
        advance_projectiles(&mut s);
        assert_eq!(s.enemies[0].hp, 10_000_000, "boss takes zero weapon damage");
    }

    fn give_weapon(s: &mut ArenaState, def: u16) {
        let id = s.alloc_entity_id();
        s.weapons.push(WeaponInstance {
            instance_id: id,
            def,
            next_fire_tick: 0,
        });
    }

    // ---- enemy roster: ranged attacks & Fortified armor ---------------------

    /// Find a roster def by name (so tests don't hardcode catalog indices).
    fn enemy_def_idx(name: &str) -> u16 {
        content::ENEMIES
            .iter()
            .position(|e| e.name == name)
            .expect("enemy in roster") as u16
    }

    #[test]
    fn ranged_enemy_damages_tank_at_range() {
        // A Spicy (ranged) sitting within its standoff range should pelt
        // the tank without ever reaching it.
        let mut s = blank_state();
        s.weapons.clear();
        let fb = enemy_def_idx("Spicy");
        let edef = &content::ENEMIES[fb as usize];
        let (range, cd, dmg) = match edef.ability {
            content::EnemyAbility::RangedAttack {
                range,
                cooldown_ticks,
                damage,
                ..
            } => (range, cooldown_ticks, damage),
            _ => panic!("Spicy must be a ranged attacker"),
        };
        // Place it well inside range but not at the origin.
        let pos = Vec2::new(Fixed::from_int(range - 50), Fixed::ZERO);
        let id = mk_enemy(&mut s, fb, edef.base_hp, pos);
        // Pick a tick on this enemy's firing phase: (tick + id) % cd == 0.
        s.tick = (cd - (id.0 % cd)) % cd;
        let hp0 = s.tank.hp;
        enemy_ranged_attacks(&mut s);
        // Damage applied through the defensive layer (no dodge/armor in blank state).
        assert_eq!(
            s.tank.hp,
            hp0 - dmg,
            "ranged enemy hit the tank at standoff range"
        );
        // The enemy is still alive and has not moved (ranged path doesn't move it).
        assert_eq!(s.enemies.len(), 1);
        assert_eq!(
            s.enemies[0].pos, pos,
            "ranged enemy attacks without closing"
        );
    }

    #[test]
    fn ranged_enemy_out_of_range_does_not_fire() {
        let mut s = blank_state();
        s.weapons.clear();
        let fb = enemy_def_idx("Spicy");
        let edef = &content::ENEMIES[fb as usize];
        let range = match edef.ability {
            content::EnemyAbility::RangedAttack { range, .. } => range,
            _ => unreachable!(),
        };
        // Just outside range.
        let pos = Vec2::new(Fixed::from_int(range + 100), Fixed::ZERO);
        mk_enemy(&mut s, fb, edef.base_hp, pos);
        let hp0 = s.tank.hp;
        // Sweep a full cooldown window of ticks: still no hit while out of range.
        for t in 0..60u32 {
            s.tick = t;
            enemy_ranged_attacks(&mut s);
        }
        assert_eq!(s.tank.hp, hp0, "out-of-range ranged enemy never fires");
    }

    #[test]
    fn ranged_attack_is_deterministic_same_seed() {
        let build = || {
            let mut s = blank_state();
            s.weapons.clear();
            let fb = enemy_def_idx("Croak");
            mk_enemy(
                &mut s,
                fb,
                1000,
                Vec2::new(Fixed::from_int(200), Fixed::ZERO),
            );
            s
        };
        let mut a = build();
        let mut b = build();
        for t in 0..120u32 {
            a.tick = t;
            b.tick = t;
            enemy_ranged_attacks(&mut a);
            enemy_ranged_attacks(&mut b);
        }
        assert_eq!(
            a.tank.hp, b.tank.hp,
            "same seed/ids → identical ranged damage"
        );
        assert!(
            a.tank.hp < build().tank.hp,
            "the spitter did damage over the window"
        );
    }

    #[test]
    fn fortified_armor_reduces_damage() {
        // Bonk is Fortified (armor class 2). Piercing is heavily
        // resisted; Siege is amplified — relative to a Light-armored swarm unit.
        let giant = enemy_def_idx("Bonk");
        let gdef = &content::ENEMIES[giant as usize];
        assert_eq!(gdef.armor_class, content::ARMOR_FORTIFIED);

        let cond = CondDamage::of(&blank_state());
        let base = 1000;

        // Piercing vs Fortified must be far less than piercing vs Light (2× there).
        let mut g_pierce = Enemy::new(EntityId(1), giant, 1_000_000, Vec2::ZERO);
        let mut accum = AbilityAccum::default();
        let dealt_pierce = apply_weapon_hit(
            &mut g_pierce,
            base,
            content::DMG_PIERCING,
            Fixed::ONE,
            cond,
            &content::StatusOnHit::NONE,
            content::WeaponAbility::None,
            Vec2::ZERO,
            &mut accum,
        );
        let light_pierce =
            content::damage_multiplier(content::DMG_PIERCING, content::ARMOR_LIGHT).scale_i64(base);
        assert!(dealt_pierce < base, "Fortified resists Piercing (<1×)");
        assert!(
            dealt_pierce < light_pierce,
            "Fortified takes far less Piercing than Light armor"
        );

        // Siege vs Fortified should be amplified (>1×) — siege is the counter.
        let mut g_siege = Enemy::new(EntityId(2), giant, 1_000_000, Vec2::ZERO);
        let dealt_siege = apply_weapon_hit(
            &mut g_siege,
            base,
            content::DMG_SIEGE,
            Fixed::ONE,
            cond,
            &content::StatusOnHit::NONE,
            content::WeaponAbility::None,
            Vec2::ZERO,
            &mut accum,
        );
        assert!(dealt_siege > base, "Siege bites Fortified harder (>1×)");
    }

    #[test]
    fn barrage_fires_one_projectile_per_target_capped() {
        // Ballista (def 6) = Barrage(4), range 1200.
        let mut s = blank_state();
        s.weapons.clear();
        give_weapon(&mut s, 6);
        for i in 0..6 {
            mk_enemy(
                &mut s,
                0,
                200,
                Vec2::new(Fixed::from_int(100 + i * 10), Fixed::ZERO),
            );
        }
        fire_weapons(&mut s);
        assert_eq!(s.projectiles.len(), 4, "barrage of 4 emits 4 projectiles");
        let distinct: std::collections::BTreeSet<u32> =
            s.projectiles.iter().map(|p| p.target.0).collect();
        assert_eq!(distinct.len(), 4, "barrage targets are distinct");

        // With fewer enemies than N, barrage caps at the available count.
        let mut s2 = blank_state();
        s2.weapons.clear();
        give_weapon(&mut s2, 6);
        mk_enemy(
            &mut s2,
            0,
            200,
            Vec2::new(Fixed::from_int(100), Fixed::ZERO),
        );
        mk_enemy(
            &mut s2,
            0,
            200,
            Vec2::new(Fixed::from_int(150), Fixed::ZERO),
        );
        fire_weapons(&mut s2);
        assert_eq!(s2.projectiles.len(), 2);
    }

    #[test]
    fn area_hits_all_in_radius_instantly() {
        // Immolation (def 7) = Area(300), range 300, 80 chaos, +2 fire.
        let mut s = blank_state();
        s.weapons.clear();
        give_weapon(&mut s, 7);
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(100), Fixed::ZERO)); // in
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(250), Fixed::ZERO)); // in
        mk_enemy(
            &mut s,
            0,
            200,
            Vec2::new(Fixed::from_int(1000), Fixed::ZERO),
        ); // out
        fire_weapons(&mut s);
        assert!(s.projectiles.is_empty(), "area is instant — no projectiles");
        assert_eq!(s.enemies.len(), 3, "200 hp survives 80 dmg");
        assert_eq!(s.enemies[0].hp, 120, "near enemy took 80");
        assert_eq!(s.enemies[0].status.fire_stacks, 2, "area applied fire");
        assert_eq!(s.enemies[2].hp, 200, "far enemy untouched");
    }

    #[test]
    fn wave_sweeps_out_to_extended_range() {
        // Shockwave Axe (def 8) = Wave(300), range 300 ⇒ reach 600, 500 normal.
        let mut s = blank_state();
        s.weapons.clear();
        give_weapon(&mut s, 8);
        let trigger = mk_enemy(&mut s, 0, 500, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        let _far_in_reach = mk_enemy(&mut s, 0, 500, Vec2::new(Fixed::from_int(500), Fixed::ZERO));
        let _out = mk_enemy(
            &mut s,
            0,
            500,
            Vec2::new(Fixed::from_int(1000), Fixed::ZERO),
        );
        let _ = trigger;
        fire_weapons(&mut s);
        // Two within reach 600 die (500 dmg); the one at 1000 survives.
        assert_eq!(s.enemies.len(), 1);
        assert_eq!(s.pending_kills.len(), 2);
    }

    #[test]
    fn bounce_chains_to_nearest_targets() {
        // Moon Glaive (def 9) = Bounce(4), range 600, 150 piercing (2× vs armor0).
        let mut s = blank_state();
        s.weapons.clear();
        give_weapon(&mut s, 9);
        for i in 0..4 {
            mk_enemy(
                &mut s,
                0,
                100,
                Vec2::new(Fixed::from_int(100 + i * 30), Fixed::ZERO),
            );
        }
        fire_weapons(&mut s);
        assert!(s.projectiles.is_empty(), "bounce is instant");
        // 4 enemies, Bounce(4) = target + 3 nearest = all 4, each takes 300 ⇒ dead.
        assert!(s.enemies.is_empty(), "all four chained and died");
        assert_eq!(s.pending_kills.len(), 4);
    }

    #[test]
    fn instant_attacks_are_deterministic() {
        let mut a = blank_state();
        a.weapons.clear();
        give_weapon(&mut a, 9); // bounce uses rng for the first target
        for i in 0..6 {
            mk_enemy(
                &mut a,
                0,
                100,
                Vec2::new(Fixed::from_int(100 + i * 20), Fixed::from_int(i)),
            );
        }
        let mut b = a.clone();
        fire_weapons(&mut a);
        fire_weapons(&mut b);
        assert_eq!(a.enemies, b.enemies);
        assert_eq!(a.pending_kills, b.pending_kills);
        assert_eq!(a.rng_targeting.state(), b.rng_targeting.state());
    }

    // ---- weapon abilities ---------------------------------------------------

    /// Catalog index of the weapon named `name` (so ability tests don't hardcode
    /// positions; the framework attaches abilities by index in `content`).
    fn weapon_idx(name: &str) -> u16 {
        content::WEAPONS
            .iter()
            .position(|w| w.name == name)
            .unwrap_or_else(|| panic!("weapon {name} not found")) as u16
    }

    /// Give the tank a single instance of the named weapon, clearing the Bow.
    fn only_weapon(s: &mut ArenaState, name: &str) {
        s.weapons.clear();
        give_weapon(s, weapon_idx(name));
    }

    #[test]
    fn life_drain_heals_tank_per_enemy_damaged() {
        // Soulstealer: Bounce(4) Heal 200/hit. Four enemies in range → 4×200 heal.
        let mut s = blank_state();
        only_weapon(&mut s, "Soulstealer");
        assert!(matches!(
            content::WEAPONS[weapon_idx("Soulstealer") as usize].ability,
            content::WeaponAbility::LifeDrain { per_hit: 200 }
        ));
        s.tank.max_hp = 1_000_000;
        s.tank.hp = 1000;
        for i in 0..4 {
            mk_enemy(
                &mut s,
                0,
                1_000_000,
                Vec2::new(Fixed::from_int(100 + i * 20), Fixed::ZERO),
            );
        }
        fire_weapons(&mut s); // instant Bounce → heals immediately
        assert_eq!(
            s.tank.hp,
            1000 + 4 * 200,
            "life-drain healed 200 per enemy hit"
        );
    }

    #[test]
    fn life_drain_is_capped_at_max_hp_and_scaled_by_healing() {
        // Single-projectile life drainer (Lifeleecher, Heal 40) heals on impact,
        // routed through Tank::heal so the cap and +% Healing apply.
        let mut s = blank_state();
        only_weapon(&mut s, "Lifeleecher");
        s.tank.max_hp = 1_000_000;
        s.tank.hp = 999_990;
        let pos = Vec2::new(Fixed::from_int(10), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 1_000_000, pos);
        let pid = s.alloc_entity_id();
        let wd = &content::WEAPONS[weapon_idx("Lifeleecher") as usize];
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: wd.damage,
            damage_type: wd.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: wd.ability,
        });
        advance_projectiles(&mut s);
        assert_eq!(s.tank.hp, 1_000_000, "life-drain heal capped at max hp");
    }

    #[test]
    fn mana_drain_refills_shield_capped() {
        // Manabolt: SingleTarget, ManaDrain 80. Restores shield on impact, capped.
        let mut s = blank_state();
        only_weapon(&mut s, "Manabolt");
        s.tank.mana_shield = 0;
        s.tank.mana_shield_max = 100;
        let pos = Vec2::new(Fixed::from_int(10), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 1_000_000, pos);
        let pid = s.alloc_entity_id();
        let wd = &content::WEAPONS[weapon_idx("Manabolt") as usize];
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: wd.damage,
            damage_type: wd.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: wd.ability,
        });
        advance_projectiles(&mut s);
        assert_eq!(
            s.tank.mana_shield, 80,
            "mana-drain restored 80 to the shield"
        );
        // A second hit caps at max (100), not 160.
        let eid2 = mk_enemy(&mut s, 0, 1_000_000, pos);
        let pid2 = s.alloc_entity_id();
        s.projectiles.push(Projectile {
            id: pid2,
            pos: Vec2::ZERO,
            target: eid2,
            last_target_pos: pos,
            damage: wd.damage,
            damage_type: wd.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: wd.ability,
        });
        advance_projectiles(&mut s);
        assert_eq!(s.tank.mana_shield, 100, "mana-drain shield capped at max");
    }

    #[test]
    fn mana_drain_noop_without_shield_pool() {
        let mut s = blank_state();
        s.tank.mana_shield_max = 0;
        s.tank.mana_shield = 0;
        let accum = AbilityAccum {
            mana: 80,
            ..Default::default()
        };
        accum.flush(&mut s, content::WeaponAbility::ManaDrain { per_hit: 80 }, 0);
        assert_eq!(
            s.tank.mana_shield, 0,
            "no shield pool → mana-drain is a no-op"
        );
    }

    #[test]
    fn knockback_pushes_enemy_away_from_tank() {
        // Slap: SingleTarget Knockback(300). Enemy on +x axis is pushed out.
        let mut s = blank_state();
        only_weapon(&mut s, "Slap");
        let pos = Vec2::new(Fixed::from_int(400), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 100_000_000, pos); // survives the hit
        let pid = s.alloc_entity_id();
        let wd = &content::WEAPONS[weapon_idx("Slap") as usize];
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: wd.damage,
            damage_type: wd.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(2000),
            on_hit: content::StatusOnHit::NONE,
            ability: wd.ability,
        });
        advance_projectiles(&mut s);
        let e = &s.enemies[0];
        // Pushed 300 directly away (along +x): 400 → 700.
        assert_eq!(
            e.pos.x,
            Fixed::from_int(700),
            "enemy knocked back 300 along +x"
        );
        assert_eq!(e.pos.y, Fixed::ZERO);
    }

    #[test]
    fn root_immobilizes_the_enemy() {
        // Tangle: SingleTarget Root(30). Rooted enemy is immobile (and poisoned).
        let mut s = blank_state();
        only_weapon(&mut s, "Tangle");
        let pos = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 100_000_000, pos);
        let pid = s.alloc_entity_id();
        let wd = &content::WEAPONS[weapon_idx("Tangle") as usize];
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: wd.damage,
            damage_type: wd.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: wd.ability,
        });
        advance_projectiles(&mut s);
        let e = &s.enemies[0];
        assert!(crate::status::is_immobile(e), "rooted enemy is immobile");
        assert!(e.status.poison_ticks > 0, "rooted enemy counts as poisoned");
        // It does not move while rooted.
        let before = s.enemies[0].pos;
        move_enemies(&mut s);
        assert_eq!(s.enemies[0].pos, before, "rooted enemy holds position");
    }

    #[test]
    fn vuln_on_hit_raises_damage_taken() {
        // Demon Eye: SingleTarget VulnOnHit(10). Adds 10 vuln stacks (+10% taken).
        let mut s = blank_state();
        only_weapon(&mut s, "Demon Eye");
        let pos = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        let eid = mk_enemy(&mut s, 0, 100_000_000, pos);
        let pid = s.alloc_entity_id();
        let wd = &content::WEAPONS[weapon_idx("Demon Eye") as usize];
        s.projectiles.push(Projectile {
            id: pid,
            pos: Vec2::ZERO,
            target: eid,
            last_target_pos: pos,
            damage: wd.damage,
            damage_type: wd.damage_type,
            splash_radius: Fixed::ZERO,
            speed: Fixed::from_int(1000),
            on_hit: content::StatusOnHit::NONE,
            ability: wd.ability,
        });
        advance_projectiles(&mut s);
        let e = &s.enemies[0];
        assert_eq!(e.status.vuln_stacks, 10, "10 vulnerability stacks applied");
        // +10% damage taken (10 stacks × 1%); fixed-point floors ≈1099/1000.
        let v = crate::status::vulnerability_mult(e).scale_i64(1000);
        assert!(
            (1099..=1100).contains(&v),
            "vuln stacks raise damage taken ≈+10%, got {v}"
        );
    }

    #[test]
    fn hazard_is_placed_and_damages_over_time() {
        // Boom Bloom: Wave Hazard(dmg 1000, radius 200, 90 ticks). Firing
        // drops a hazard at the first damaged enemy; it then pulses each tick.
        let mut s = blank_state();
        only_weapon(&mut s, "Boom Bloom");
        let pos = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        // A high-hp enemy so it survives the wave and the hazard can keep hitting.
        mk_enemy(&mut s, 0, 1_000_000_000, pos);
        fire_weapons(&mut s); // Wave hits the enemy and drops a hazard at its pos
        assert_eq!(s.hazards.len(), 1, "a hazard was placed");
        assert_eq!(s.hazards[0].dmg, 1000);
        assert_eq!(s.hazards[0].ticks_left, 90);

        // A separate enemy standing on the hazard takes its pulse each tick.
        let victim = Vec2::new(Fixed::from_int(100), Fixed::ZERO);
        let vid = mk_enemy(&mut s, 0, 1_000_000_000, victim);
        let hp0 = s.enemies.iter().find(|e| e.id == vid).unwrap().hp;
        tick_hazards(&mut s);
        let hp1 = s.enemies.iter().find(|e| e.id == vid).unwrap().hp;
        // Siege vs armor class 0 = 1× → exactly 1000 damage this tick.
        assert_eq!(hp0 - hp1, 1000, "hazard pulsed 1000 damage in range");
        assert_eq!(s.hazards[0].ticks_left, 89, "hazard lifetime decremented");
    }

    #[test]
    fn hazard_expires_after_its_lifetime() {
        let mut s = blank_state();
        s.weapons.clear();
        let id = s.alloc_entity_id();
        s.hazards.push(Hazard {
            id,
            pos: Vec2::ZERO,
            dmg: 100,
            damage_type: content::DMG_SIEGE,
            radius: 200,
            ticks_left: 2,
        });
        mk_enemy(
            &mut s,
            0,
            1_000_000,
            Vec2::new(Fixed::from_int(50), Fixed::ZERO),
        );
        tick_hazards(&mut s);
        assert_eq!(s.hazards.len(), 1, "still active after 1 tick");
        tick_hazards(&mut s);
        assert!(s.hazards.is_empty(), "hazard expired after its lifetime");
    }

    #[test]
    fn summon_ability_is_inert_for_now() {
        // Shroom Doom carries the deferred Summon variant; it must not crash and
        // behaves as pure damage (no extra entities spawned).
        let mut s = blank_state();
        only_weapon(&mut s, "Shroom Doom");
        for i in 0..3 {
            mk_enemy(
                &mut s,
                0,
                100_000_000,
                Vec2::new(Fixed::from_int(100 + i * 20), Fixed::ZERO),
            );
        }
        let before = s.enemies.len();
        fire_weapons(&mut s);
        assert_eq!(s.hazards.len(), 0, "summon places no hazard");
        assert_eq!(s.enemies.len(), before, "summon spawns no entities yet");
    }

    #[test]
    fn ability_effects_are_deterministic() {
        let build = || {
            let mut s = blank_state();
            only_weapon(&mut s, "Slap");
            for i in 0..4 {
                mk_enemy(
                    &mut s,
                    0,
                    100_000_000,
                    Vec2::new(Fixed::from_int(120 + i * 7), Fixed::from_int(i)),
                );
            }
            s
        };
        let mut a = build();
        let mut b = a.clone();
        for _ in 0..5 {
            fire_weapons(&mut a);
            advance_projectiles(&mut a);
            fire_weapons(&mut b);
            advance_projectiles(&mut b);
        }
        assert_eq!(a.enemies, b.enemies, "knockback ability is deterministic");
        assert_eq!(crate::checksum(&a), crate::checksum(&b));
    }

    // ---- Summon ability + minions ------------------------------------------

    fn summon_kill(s: &mut ArenaState, eid: EntityId, ability: WeaponAbility) {
        let cond = CondDamage::of(s);
        let mut accum = AbilityAccum::default();
        {
            let e = s.enemies.iter_mut().find(|e| e.id == eid).unwrap();
            apply_weapon_hit(
                e,
                9_999_999,
                content::DMG_CHAOS,
                Fixed::ONE,
                cond,
                &content::StatusOnHit::NONE,
                ability,
                Vec2::ZERO,
                &mut accum,
            );
        }
        accum.flush(s, ability, content::DMG_CHAOS);
    }

    #[test]
    fn summon_raises_a_minion_from_a_corpse_on_kill() {
        let mut s = blank_state();
        let eid = mk_enemy(&mut s, 0, 10, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        summon_kill(
            &mut s,
            eid,
            WeaponAbility::Summon {
                kind: 0,
                hp: 500,
                damage: 250,
            },
        );
        assert_eq!(s.minions.len(), 1, "one ally rises from the corpse");
        let m = s.minions[0];
        assert_eq!(m.kind, 0);
        assert_eq!(m.damage, 250);
        assert_eq!(m.damage_type, content::DMG_CHAOS);
        assert_eq!(m.expire_tick, s.tick + MINION_LIFETIME);
    }

    #[test]
    fn summon_does_not_raise_without_a_kill() {
        let mut s = blank_state();
        let eid = mk_enemy(&mut s, 0, 1_000_000, Vec2::ZERO);
        let ability = WeaponAbility::Summon {
            kind: 1,
            hp: 1500,
            damage: 600,
        };
        let cond = CondDamage::of(&s);
        let mut accum = AbilityAccum::default();
        {
            let e = s.enemies.iter_mut().find(|e| e.id == eid).unwrap();
            apply_weapon_hit(
                e,
                100,
                content::DMG_CHAOS,
                Fixed::ONE,
                cond,
                &content::StatusOnHit::NONE,
                ability,
                Vec2::ZERO,
                &mut accum,
            );
        }
        accum.flush(&mut s, ability, content::DMG_CHAOS);
        assert!(s.minions.is_empty(), "survivor ⇒ no minion raised");
    }

    #[test]
    fn summon_respects_the_minion_cap() {
        let mut s = blank_state();
        let ability = WeaponAbility::Summon {
            kind: 0,
            hp: 1,
            damage: 1,
        };
        let cond = CondDamage::of(&s);
        let mut accum = AbilityAccum::default();
        let mut ids = Vec::new();
        for i in 0..(MAX_MINIONS + 4) {
            ids.push(mk_enemy(
                &mut s,
                0,
                1,
                Vec2::new(Fixed::from_int(i as i64 * 10), Fixed::ZERO),
            ));
        }
        for id in &ids {
            let e = s.enemies.iter_mut().find(|e| e.id == *id).unwrap();
            apply_weapon_hit(
                e,
                9_999_999,
                content::DMG_CHAOS,
                Fixed::ONE,
                cond,
                &content::StatusOnHit::NONE,
                ability,
                Vec2::ZERO,
                &mut accum,
            );
        }
        accum.flush(&mut s, ability, content::DMG_CHAOS);
        assert_eq!(s.minions.len(), MAX_MINIONS, "capped at MAX_MINIONS");
    }

    #[test]
    fn minion_strikes_and_damages_a_nearby_enemy() {
        let mut s = blank_state();
        let now = s.tick;
        let mid = s.alloc_entity_id();
        s.minions.push(Minion {
            id: mid,
            pos: Vec2::ZERO,
            kind: 0,
            hp: 500,
            damage: 1000,
            damage_type: content::DMG_CHAOS,
            next_attack_tick: now,
            expire_tick: now + 1000,
        });
        let eid = mk_enemy(
            &mut s,
            0,
            100_000,
            Vec2::new(Fixed::from_int(50), Fixed::ZERO),
        );
        let before = s.enemies.iter().find(|e| e.id == eid).unwrap().hp;
        tick_minions(&mut s);
        let after = s.enemies.iter().find(|e| e.id == eid).unwrap().hp;
        assert!(after < before, "an in-reach minion strikes the enemy");
    }

    #[test]
    fn minion_expires_after_its_lifetime() {
        let mut s = blank_state();
        let mid = s.alloc_entity_id();
        s.minions.push(Minion {
            id: mid,
            pos: Vec2::ZERO,
            kind: 1,
            hp: 1,
            damage: 1,
            damage_type: content::DMG_CHAOS,
            next_attack_tick: 0,
            expire_tick: 5,
        });
        s.tick = 4;
        tick_minions(&mut s);
        assert_eq!(s.minions.len(), 1, "alive while tick < expire_tick");
        s.tick = 5;
        tick_minions(&mut s);
        assert!(
            s.minions.is_empty(),
            "removed once tick reaches expire_tick"
        );
    }

    #[test]
    fn minion_phase_is_deterministic() {
        let build = || {
            let mut s = blank_state();
            let mid = s.alloc_entity_id();
            s.minions.push(Minion {
                id: mid,
                pos: Vec2::ZERO,
                kind: 0,
                hp: 500,
                damage: 200,
                damage_type: content::DMG_CHAOS,
                next_attack_tick: 0,
                expire_tick: 500,
            });
            mk_enemy(
                &mut s,
                0,
                5000,
                Vec2::new(Fixed::from_int(400), Fixed::ZERO),
            );
            for _ in 0..120 {
                tick_minions(&mut s);
                s.tick += 1;
            }
            s
        };
        let a = build();
        let b = build();
        assert_eq!(
            a.minions, b.minions,
            "minion movement/attacks are deterministic"
        );
        assert_eq!(a.enemies, b.enemies);
        assert_eq!(crate::checksum(&a), crate::checksum(&b));
    }

    // ---- EXPANSION E2: damage/poison aura (Blight Aura) ----------------------

    fn aura_state() -> ArenaState {
        let mut s = blank_state();
        s.weapons.clear();
        s.tank.aura_range = 600;
        s.tank.aura_cadence = 30; // fire every 30 ticks
        s.tank.aura_damage = 200;
        s.tank.aura_poison_dps = 2;
        s.tank.aura_poison_ticks = 90;
        s
    }

    #[test]
    fn aura_fires_only_on_its_integer_cadence_and_hits_only_in_range() {
        let mut s = aura_state();
        // One enemy in range (≤600), one outside. Magic vs armor 0 → ×1 matrix.
        let near = mk_enemy(
            &mut s,
            0,
            1_000_000,
            Vec2::new(Fixed::from_int(500), Fixed::ZERO),
        );
        let far = mk_enemy(
            &mut s,
            0,
            1_000_000,
            Vec2::new(Fixed::from_int(1000), Fixed::ZERO),
        );

        // 29 ticks below the cadence: counter climbs, no fire.
        for _ in 0..29 {
            tick_aura(&mut s);
        }
        assert_eq!(s.tank.aura_tick, 29, "counter advanced but not yet fired");
        assert_eq!(
            s.enemies.iter().find(|e| e.id == near).unwrap().hp,
            1_000_000,
            "no damage pre-cadence"
        );

        // The 30th call fires: counter resets, in-range enemy takes damage + poison.
        tick_aura(&mut s);
        assert_eq!(s.tank.aura_tick, 0, "counter reset on fire");
        let n = s.enemies.iter().find(|e| e.id == near).unwrap();
        assert_eq!(n.hp, 1_000_000 - 200, "in-range enemy took aura damage");
        assert_eq!(n.status.poison_dps, 2, "in-range enemy poisoned");
        assert_eq!(n.status.poison_ticks, 90);
        let f = s.enemies.iter().find(|e| e.id == far).unwrap();
        assert_eq!(f.hp, 1_000_000, "out-of-range enemy untouched");
        assert_eq!(f.status.poison_dps, 0, "out-of-range enemy not poisoned");
    }

    #[test]
    fn aura_is_a_noop_when_unconfigured() {
        let mut s = blank_state();
        s.weapons.clear();
        // aura_cadence == 0 (default) ⇒ disabled.
        mk_enemy(
            &mut s,
            0,
            1000,
            Vec2::new(Fixed::from_int(100), Fixed::ZERO),
        );
        for _ in 0..120 {
            tick_aura(&mut s);
        }
        assert_eq!(s.enemies[0].hp, 1000, "no aura ⇒ no damage");
        assert_eq!(
            s.tank.aura_tick, 0,
            "counter does not advance when disabled"
        );
    }

    #[test]
    fn aura_is_deterministic_across_two_runs_via_checksum() {
        let build = || {
            let mut s = aura_state();
            // Several enemies at varied positions, inserted out of spatial order so a
            // stable id-ordered pass is what guarantees the match.
            mk_enemy(
                &mut s,
                0,
                5000,
                Vec2::new(Fixed::from_int(550), Fixed::ZERO),
            );
            mk_enemy(
                &mut s,
                0,
                5000,
                Vec2::new(Fixed::from_int(100), Fixed::from_int(200)),
            );
            mk_enemy(
                &mut s,
                0,
                5000,
                Vec2::new(Fixed::from_int(2000), Fixed::ZERO),
            ); // far
            for _ in 0..95 {
                tick_aura(&mut s);
                s.tick += 1;
            }
            s
        };
        let a = build();
        let b = build();
        assert_eq!(a.enemies, b.enemies, "aura AoE identical across runs");
        assert_eq!(
            crate::checksum(&a),
            crate::checksum(&b),
            "checksum stable across runs"
        );
    }
}
