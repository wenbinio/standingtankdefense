//! Combat behavior — AGENT B. Implement the three phases below. Read content via
//! `crate::content`, use `s.rng_targeting` for any randomness, keep all math
//! integer/Fixed (no floats), and iterate in a stable order (by `instance_id` /
//! `id`); never iterate a HashMap. No cross-module calls: record kills by pushing
//! the dead enemy's `def` onto `s.pending_kills` (economy drains it later).
use crate::content::{self, Attack};
use crate::ids::EntityId;
use crate::state::*;
use determinism::Fixed;

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
fn apply_weapon_hit(
    e: &mut Enemy,
    base: i64,
    damage_type: u8,
    mod_mult: Fixed,
    cond: CondDamage,
    on_hit: &content::StatusOnHit,
) -> i64 {
    let edef = &content::ENEMIES[e.def as usize];
    if edef.boss {
        return 0;
    }
    let armor = content::damage_multiplier(damage_type, edef.armor_class);
    let vuln = crate::status::vulnerability_mult(e);
    let dmg = armor.mul(mod_mult).mul(vuln).mul(cond.mult(e)).scale_i64(base);
    e.hp -= dmg;
    crate::status::apply_on_hit(e, on_hit);
    dmg
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
        let wmult = static_mult + add_self.mul(s.modifiers.mul_global);
        let baked = wmult.scale_i64(wdef.damage);
        // Poison-damage / stun-duration scalers depend only on the player's
        // modifiers, so (like base damage) they bake into the hit at fire time.
        let on_hit = s.modifiers.scale_on_hit(wdef.on_hit);
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
                    if s.tank.pos.dist_sq(e.pos) <= r2 {
                        instant_damage +=
                            apply_weapon_hit(e, wdef.damage, wdef.damage_type, wmult, cond, &on_hit);
                    }
                }
                any_instant_damage = true;
            }
            Attack::Wave(extra) => {
                // Instant sweep out to range + extra around the tank.
                let reach = range + Fixed::from_int(extra);
                let r2 = reach.mul(reach);
                for e in s.enemies.iter_mut() {
                    if s.tank.pos.dist_sq(e.pos) <= r2 {
                        instant_damage +=
                            apply_weapon_hit(e, wdef.damage, wdef.damage_type, wmult, cond, &on_hit);
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
                order.sort_by_key(|&i| {
                    (s.enemies[i].pos.dist_sq(origin).raw(), s.enemies[i].id.0)
                });
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
                    );
                }
                any_instant_damage = true;
            }
        }

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
    }

    let mut impacts: Vec<Impact> = Vec::new();
    let mut survivors: Vec<Projectile> = Vec::with_capacity(s.projectiles.len());

    // Take ownership of the projectile list to iterate without aliasing `s`.
    let projectiles = std::mem::take(&mut s.projectiles);
    for mut p in projectiles {
        // Resolve current target position (if the enemy still exists).
        let target_pos = s
            .enemies
            .iter()
            .find(|e| e.id == p.target)
            .map(|e| e.pos);

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
    let mut impact_damage: i64 = 0;
    for imp in impacts {
        let mod_mult = Fixed::ONE; // weapon multiplier was baked into projectile damage at fire time
        if imp.splash_radius > Fixed::ZERO {
            let radius_sq = imp.splash_radius.mul(imp.splash_radius);
            for e in s.enemies.iter_mut() {
                if imp.point.dist_sq(e.pos) <= radius_sq {
                    impact_damage +=
                        apply_weapon_hit(e, imp.damage, imp.damage_type, mod_mult, cond, &imp.on_hit);
                }
            }
        } else if let Some(e) = s.enemies.iter_mut().find(|e| e.id == imp.target) {
            impact_damage +=
                apply_weapon_hit(e, imp.damage, imp.damage_type, mod_mult, cond, &imp.on_hit);
        }
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
            // Contact: deal contact damage through the defensive layer, remove
            // the enemy (no bounty for self-destruct).
            crate::defense::hit_tank(s, contact);
        } else {
            e.pos = moved;
            survivors.push(e);
        }
    }

    s.enemies = survivors;
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
        s.weapons.push(WeaponInstance { instance_id: wid, def: 0, next_fire_tick: 0 });

        // Enemy out of range (1500 > 900) → no fire, no cooldown advance.
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(1500), Fixed::ZERO));
        fire_weapons(&mut s);
        assert!(s.projectiles.is_empty(), "should not fire at out-of-range enemy");
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
        s.weapons.push(WeaponInstance { instance_id: wid, def: 0, next_fire_tick: 100 });
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
            s.weapons.push(WeaponInstance { instance_id: wid, def: 0, next_fire_tick: 0 });
            mk_enemy(&mut s, 0, 1_000_000, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
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
        assert_eq!(s2.modifiers.weapon_damage_mult(mortar).scale_i64(1000), 2000);
    }

    #[test]
    fn attack_speed_modifier_shortens_cooldown() {
        let mut s = blank_state();
        s.weapons.clear();
        let wid = s.alloc_entity_id();
        s.weapons.push(WeaponInstance { instance_id: wid, def: 0, next_fire_tick: 0 });
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
        let _far = mk_enemy(&mut s, 0, 100, Vec2::new(Fixed::from_int(1000), Fixed::ZERO)); // 900 away
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
        });
        advance_projectiles(&mut s);
        assert_eq!(s.enemies.len(), 1, "bystander untouched by single-target miss");
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
        mk_enemy(&mut early, 0, 200, Vec2::new(Fixed::from_int(3), Fixed::ZERO));
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
        mk_enemy(&mut late, 0, 200, Vec2::new(Fixed::from_int(3), Fixed::ZERO));
        move_enemies(&mut late);
        assert_eq!(late.tank.hp, hp_late - expected, "late contact damage scaled");
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

    // ---- determinism --------------------------------------------------------

    #[test]
    fn fire_weapons_is_deterministic_same_seed() {
        // Multiple in-range candidates so the random target pick is exercised.
        let mut a = blank_state();
        a.weapons.clear();
        let wid = a.alloc_entity_id();
        a.weapons.push(WeaponInstance { instance_id: wid, def: 0, next_fire_tick: 0 });
        for i in 0..5 {
            mk_enemy(&mut a, 0, 200, Vec2::new(Fixed::from_int(100 + i), Fixed::from_int(i)));
        }
        let mut b = a.clone();

        fire_weapons(&mut a);
        fire_weapons(&mut b);
        assert_eq!(a.projectiles, b.projectiles, "same seed → same target chosen");
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
        apply_weapon_hit(&mut plain, 100, content::DMG_PIERCING, Fixed::ONE, cond, &content::StatusOnHit::NONE);
        apply_weapon_hit(&mut stunned, 100, content::DMG_PIERCING, Fixed::ONE, cond, &content::StatusOnHit::NONE);
        assert_eq!(1_000_000 - plain.hp, 200, "no conditional bonus on un-stunned");
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
        assert_eq!(s.projectiles[0].on_hit.poison_dps, 30, "20 × 1.5 baked at fire time");
    }

    #[test]
    fn stun_duration_modifier_scales_applied_stun() {
        // A weapon with stun on-hit gets its stun extended by the modifier.
        let mut s = blank_state();
        s.modifiers.stun_dur_mult = Fixed::from_int(2); // ×2
        let on_hit = content::StatusOnHit { stun_ticks: 30, ..content::StatusOnHit::NONE };
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
        s.modifiers.weapon_count_scaling.push(crate::state::WeaponCountScale {
            weapon_def: 0,
            dmg_type: content::DMG_PIERCING,
            per: Fixed::from_ratio(1, 100),
        });
        for _ in 0..3 {
            give_weapon(&mut s, 0);
        }
        mk_enemy(&mut s, 0, 1_000_000, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        fire_weapons(&mut s);
        assert_eq!(s.projectiles.len(), 3, "all three Bows fire");
        // add_self = 3 × 1% = 3% ⇒ 75 × 1.03 = 77 (floored).
        assert_eq!(s.projectiles[0].damage, 77);

        // Wrong damage type is unaffected: a Siege rule does nothing for the Bow.
        let mut s2 = blank_state();
        s2.weapons.clear();
        s2.modifiers.weapon_count_scaling.push(crate::state::WeaponCountScale {
            weapon_def: 0,
            dmg_type: content::DMG_SIEGE,
            per: Fixed::from_ratio(1, 100),
        });
        give_weapon(&mut s2, 0);
        mk_enemy(&mut s2, 0, 1_000_000, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        fire_weapons(&mut s2);
        assert_eq!(s2.projectiles[0].damage, content::WEAPONS[0].damage, "no piercing scaling");
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
        });
        advance_projectiles(&mut s);
        assert_eq!(s.total_damage_dealt, 400, "actual armor-scaled damage scored");
    }

    #[test]
    fn instant_attack_damage_accumulates_and_pays_bloodmoney() {
        // Immolation (def 7) = Area(300), 80 chaos. Two in-range enemies.
        let mut s = blank_state();
        s.weapons.clear();
        s.economy.gold_per_damage = Fixed::from_ratio(1, 4); // exactly representable
        give_weapon(&mut s, 7);
        mk_enemy(&mut s, 0, 10_000, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        mk_enemy(&mut s, 0, 10_000, Vec2::new(Fixed::from_int(250), Fixed::ZERO));
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
        let bid = mk_enemy(&mut s, content::SAMWISE, 10_000_000, pos);
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
        });
        advance_projectiles(&mut s);
        assert_eq!(s.enemies[0].hp, 10_000_000, "boss takes zero weapon damage");
    }

    fn give_weapon(s: &mut ArenaState, def: u16) {
        let id = s.alloc_entity_id();
        s.weapons.push(WeaponInstance { instance_id: id, def, next_fire_tick: 0 });
    }

    #[test]
    fn barrage_fires_one_projectile_per_target_capped() {
        // Ballista (def 6) = Barrage(4), range 1200.
        let mut s = blank_state();
        s.weapons.clear();
        give_weapon(&mut s, 6);
        for i in 0..6 {
            mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(100 + i * 10), Fixed::ZERO));
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
        mk_enemy(&mut s2, 0, 200, Vec2::new(Fixed::from_int(100), Fixed::ZERO));
        mk_enemy(&mut s2, 0, 200, Vec2::new(Fixed::from_int(150), Fixed::ZERO));
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
        mk_enemy(&mut s, 0, 200, Vec2::new(Fixed::from_int(1000), Fixed::ZERO)); // out
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
        let _out = mk_enemy(&mut s, 0, 500, Vec2::new(Fixed::from_int(1000), Fixed::ZERO));
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
            mk_enemy(&mut s, 0, 100, Vec2::new(Fixed::from_int(100 + i * 30), Fixed::ZERO));
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
            mk_enemy(&mut a, 0, 100, Vec2::new(Fixed::from_int(100 + i * 20), Fixed::from_int(i)));
        }
        let mut b = a.clone();
        fire_weapons(&mut a);
        fire_weapons(&mut b);
        assert_eq!(a.enemies, b.enemies);
        assert_eq!(a.pending_kills, b.pending_kills);
        assert_eq!(a.rng_targeting.state(), b.rng_targeting.state());
    }
}
