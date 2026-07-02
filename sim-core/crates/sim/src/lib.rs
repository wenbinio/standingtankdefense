//! Standing Tank Defense — deterministic simulation core (M0).
//!
//! Engine-independent (no Godot types). Pure, fixed-tick, fixed-point. The
//! whole netcode rests on `step()` being a deterministic function of
//! `(state, input)` (`docs/03 §3.3`, `docs/05 §5.6`).
//!
//! Module ownership (see `CLAUDE.md`):
//! - centrally owned seams: `ids`, `state`, `content`, this file (`step`/`checksum`).
//! - behavior modules: `combat`, `waves` (Agent B); `economy`, `shop`, `input` (Agent C).

pub mod bot;
mod combat;
pub mod content;
mod defense;
pub mod descriptions;
mod economy;
mod ids;
mod input;
mod modifiers;
mod shop;
pub mod snapshot;
mod state;
mod status;
pub mod view;
mod waves;

pub use ids::*;
pub use state::*;

use determinism::Checksum;

/// Simulation frequency. The only clock the sim knows.
pub const TICK_HZ: u32 = 30;
/// Ticks per round (a new shop opens each round). 30 s @ 30 Hz.
pub const ROUND_TICKS: u32 = 30 * TICK_HZ;

/// Advance the arena by exactly one tick, applying `inp` (this player's action).
///
/// The order of operations below IS part of the spec: client and the
/// validating shadow-sim must apply phases in this exact sequence so their
/// `checksum()` agree (`docs/05 §5.6.1`).
pub fn step(s: &mut ArenaState, inp: Input) {
    // 0. Drop last tick's render events (drained or not — `docs/09 §9.3`). The
    // clear runs before the dead early-out so a frozen arena never re-serves
    // its death-tick events.
    s.events.clear();
    if s.dead {
        // Frozen after death, but keep advancing the tick so checksum traces
        // across runs stay length-aligned.
        s.tick += 1;
        return;
    }

    // 1. Round boundary → refresh shop + per-round economy.
    let new_round = s.tick / ROUND_TICKS;
    if new_round != s.round {
        s.round = new_round;
        s.emit(SimEvent::RoundStart { round: new_round });
        shop::generate_offers(s);
        economy::on_round_start(s);
        // Stacking spikes (source: Bloody Spikes) reset to 0 at the round boundary
        // ("resets when a new shop is made available"). Deterministic: keyed purely
        // off the round transition (integer tick). At round 0 the counter is already
        // 0, so this is a no-op there; from round 1 on it clears the prior round.
        s.tank.spikes_stacks = 0;
    }
    // 2. Player input (buy / reroll / clear).
    input::apply(s, inp);
    // 2b. Time-scaling growth: apply any ramping modifiers whose interval elapsed.
    modifiers::apply_ramps(s);
    // 3. Spawn enemies for the current wave.
    waves::spawn(s);
    // 3b. Vulnerability-Pulse auras stack a damage-taken debuff on enemies in range.
    status::pulse(s);
    // 4. Weapons select targets and emit projectiles.
    combat::fire_weapons(s);
    // 5. Projectiles move; on arrival apply (splash) damage; kills → pending_kills.
    combat::advance_projectiles(s);
    // 6. Enemies advance toward the tank; contact damage on arrival.
    combat::move_enemies(s);
    // 6c. Ranged enemies (breathers/spitters/casters) attack the tank at standoff.
    combat::enemy_ranged_attacks(s);
    // 6b. Spikes: if the tank was hit, retaliate against nearby enemies.
    defense::spikes(s);
    // 6b'. Shield-break stun (Energy Pulse): if the Mana Shield broke this tick,
    // stun enemies in range. Runs after `spikes` (which consumes the hit flag) but
    // before hazards/auras; the break flag is set by `hit_tank` during combat.
    defense::shield_break_stun(s);
    // 6c. Hazards (land mines / burning oil) pulse damage to enemies in range.
    combat::tick_hazards(s);
    // 6c'. Damage/poison aura (Blight Aura): periodic AoE on its integer cadence.
    combat::tick_aura(s);
    // 6d. Summoned allies (Larvae / Spores) move and strike enemies.
    combat::tick_minions(s);
    // 7. Status effects: poison DoT, frost/stun decay (poison kills → pending_kills).
    status::tick(s);
    // 8. Drain pending_kills → award bounty (scaled by bounty_mult).
    economy::collect_bounties(s);
    // 9. Passive income.
    economy::tick_income(s);
    // 10. Tank regeneration (mana shield + HP).
    defense::regen(s);
    // 11. Death check (tank hp ≤ 0).
    economy::resolve_deaths(s);

    s.tick += 1;
}

/// Deterministic `state_checksum` over all authoritative-relevant fields.
/// Iterates entities in id-sorted order so it is independent of vector
/// ordering (`docs/04 §4.4.4`). Render-only state is excluded by construction.
pub fn checksum(s: &ArenaState) -> u64 {
    let mut c = Checksum::new();
    c.write_u32(s.tick);
    c.write_u32(s.round);
    c.write_u64(s.master_seed);
    c.write_u32(s.player_id);

    c.write_i64(s.tank.hp);
    c.write_i64(s.tank.max_hp);
    c.write_fixed(s.tank.pos.x);
    c.write_fixed(s.tank.pos.y);
    c.write_u32(s.tank.clear_cooldown_end);
    c.write_i64(s.tank.armor);
    c.write_u32(s.tank.dodge_num);
    c.write_u32(s.tank.dodge_den);
    c.write_i64(s.tank.mana_shield);
    c.write_i64(s.tank.mana_shield_max);
    c.write_i64(s.tank.mana_regen_per_tick);
    c.write_i64(s.tank.hp_regen_per_tick);
    c.write_i64(s.tank.spikes_damage);
    c.write_fixed(s.tank.spikes_mult);
    c.write_fixed(s.tank.shield_active_dr);
    c.write_i64(s.tank.heal_on_damaged);
    c.write_i64(s.tank.heal_on_kill);
    c.write_i64(s.tank.mana_on_kill);
    c.write_i64(s.tank.heal_on_poison);
    c.write_fixed(s.tank.healing_mult);
    c.write_fixed(s.tank.missing_hp_heal_pct);
    c.write_u32(s.tank.revives);
    c.write_i64(s.tank.revive_bonus_hp);
    // EXPANSION E2 exotic-mechanic tank state. The dynamic counters (`spikes_stacks`,
    // `aura_tick`) MUST be checksummed (they drive behavior and change per tick); the
    // config fields are included alongside for a clean snapshot↔checksum mirror.
    c.write_i64(s.tank.shieldbreak_stun_range);
    c.write_u32(s.tank.shieldbreak_stun_ticks);
    c.write_i64(s.tank.spikes_poison_dps);
    c.write_u32(s.tank.spikes_poison_ticks);
    c.write_i64(s.tank.spikes_stack_per);
    c.write_u32(s.tank.spikes_stacks);
    c.write_u32(s.tank.spikes_stacks_max);
    c.write_i64(s.tank.aura_range);
    c.write_u32(s.tank.aura_cadence);
    c.write_i64(s.tank.aura_damage);
    c.write_i64(s.tank.aura_poison_dps);
    c.write_u32(s.tank.aura_poison_ticks);
    c.write_u32(s.tank.aura_tick);

    c.write_i64(s.economy.gold);
    c.write_i64(s.economy.income_per_tick);
    c.write_fixed(s.economy.income_mult);
    c.write_fixed(s.economy.income_regen_pct);
    c.write_fixed(s.economy.bounty_mult);
    c.write_i64(s.economy.bounty_proc_chance_pct);
    c.write_fixed(s.economy.bounty_proc_bonus);
    c.write_fixed(s.economy.gold_per_damage);
    c.write_fixed(s.economy.income_shield_pct);
    c.write_u32(s.economy.rerolls_remaining);
    c.write_i64(s.economy.reroll_cost);

    c.write_u32(s.next_entity_id);
    c.write_u32(s.dead as u32);
    c.write_u32(s.death_tick.unwrap_or(u32::MAX));
    c.write_u32(s.pending_kills.len() as u32);
    for k in &s.pending_kills {
        c.write_u32(*k as u32);
    }
    c.write_i64(s.total_damage_dealt);
    c.write_i64(s.total_gold_earned);
    c.write_u32(s.bought_attack_mask as u32);
    c.write_u32(s.weapons_bought);
    c.write_u32(s.economy_purchases);

    c.write_u64(s.rng_spawn.state());
    c.write_u64(s.rng_targeting.state());
    c.write_u64(s.rng_shop.state());
    c.write_u64(s.rng_reroll.state());
    c.write_u64(s.rng_proc.state());

    // Weapons (instance ids are monotonic; checksum in id order).
    let mut w: Vec<&WeaponInstance> = s.weapons.iter().collect();
    w.sort_by_key(|x| x.instance_id);
    c.write_u32(w.len() as u32);
    for x in w {
        c.write_u32(x.instance_id.0);
        c.write_u32(x.def as u32);
        c.write_u32(x.next_fire_tick);
    }

    // Modifiers aggregate.
    c.write_fixed(s.modifiers.add_global);
    for a in &s.modifiers.add_by_type {
        c.write_fixed(*a);
    }
    for a in &s.modifiers.add_by_scope {
        c.write_fixed(*a);
    }
    c.write_fixed(s.modifiers.mul_global);
    c.write_fixed(s.modifiers.attack_speed);
    c.write_fixed(s.modifiers.vs_stunned);
    c.write_fixed(s.modifiers.vs_poisoned);
    c.write_fixed(s.modifiers.poison_dmg_mult);
    c.write_fixed(s.modifiers.stun_dur_mult);
    c.write_u32(s.modifiers.weapon_count_scaling.len() as u32);
    for r in &s.modifiers.weapon_count_scaling {
        c.write_u32(r.weapon_def as u32);
        c.write_u32(r.dmg_type as u32);
        c.write_fixed(r.per);
    }
    c.write_fixed(s.modifiers.dmg_per_maxhp_rate);
    c.write_fixed(s.modifiers.dmg_per_bounty_rate);
    c.write_fixed(s.modifiers.shield_active_dmg);

    // Active time-scaling ramps (append-only order).
    c.write_u32(s.ramps.len() as u32);
    for r in &s.ramps {
        let (tag, a, b, d) = r.effect.words();
        c.write_u32(tag as u32);
        c.write_i64(a);
        c.write_i64(b);
        c.write_i64(d);
        c.write_u32(r.interval_ticks);
        c.write_u32(r.next_apply);
    }

    // Active vulnerability pulses.
    c.write_u32(s.vuln_pulses.len() as u32);
    for p in &s.vuln_pulses {
        c.write_u32(p.magnitude as u32);
        c.write_i64(p.range);
        c.write_u32(p.interval_ticks);
        c.write_u32(p.next_tick);
    }

    // Pending meta perk (duplicator/voucher) armed for the next purchase.
    match s.pending_perk {
        Some(p) => {
            c.write_u32(1);
            c.write_u32(p.rarity as u32);
            c.write_u32(p.extra_copies);
            c.write_u32(p.free as u32);
        }
        None => c.write_u32(0),
    }

    // Shop offers (slot order is meaningful).
    c.write_u32(s.shop.shop_seq);
    c.write_u32(s.shop.offers.len() as u32);
    for o in &s.shop.offers {
        c.write_u32(match o.kind {
            OfferKind::Weapon => 0,
            OfferKind::Modifier => 1,
        });
        c.write_u32(o.def as u32);
        c.write_i64(o.cost);
    }

    // Enemies in id order.
    let mut e: Vec<&Enemy> = s.enemies.iter().collect();
    e.sort_by_key(|x| x.id);
    c.write_u32(e.len() as u32);
    for x in e {
        c.write_u32(x.id.0);
        c.write_u32(x.def as u32);
        c.write_i64(x.hp);
        c.write_fixed(x.pos.x);
        c.write_fixed(x.pos.y);
        c.write_i64(x.status.poison_dps);
        c.write_u32(x.status.poison_ticks);
        c.write_u32(x.status.frost_stacks as u32);
        c.write_u32(x.status.frost_ticks);
        c.write_u32(x.status.fire_stacks as u32);
        c.write_u32(x.status.vuln_stacks as u32);
        c.write_u32(x.status.stun_ticks);
        c.write_u32(x.status.freeze_ticks);
    }

    // Projectiles in id order.
    let mut p: Vec<&Projectile> = s.projectiles.iter().collect();
    p.sort_by_key(|x| x.id);
    c.write_u32(p.len() as u32);
    for x in p {
        c.write_u32(x.id.0);
        c.write_u32(x.weapon_kind as u32);
        c.write_fixed(x.pos.x);
        c.write_fixed(x.pos.y);
        c.write_u32(x.target.0);
        c.write_fixed(x.last_target_pos.x);
        c.write_fixed(x.last_target_pos.y);
        c.write_i64(x.damage);
        c.write_u32(x.damage_type as u32);
        c.write_fixed(x.splash_radius);
        c.write_fixed(x.speed);
        c.write_i64(x.on_hit.poison_dps);
        c.write_u32(x.on_hit.poison_ticks);
        c.write_u32(x.on_hit.frost_stacks as u32);
        c.write_u32(x.on_hit.fire_stacks as u32);
        c.write_u32(x.on_hit.stun_ticks);
        let (atag, a, b, d) = x.ability.words();
        c.write_u32(atag as u32);
        c.write_i64(a);
        c.write_i64(b);
        c.write_i64(d);
    }

    // Hazards in id order (land mines / burning oil).
    let mut hz: Vec<&Hazard> = s.hazards.iter().collect();
    hz.sort_by_key(|x| x.id);
    c.write_u32(hz.len() as u32);
    for x in hz {
        c.write_u32(x.id.0);
        c.write_fixed(x.pos.x);
        c.write_fixed(x.pos.y);
        c.write_i64(x.dmg);
        c.write_u32(x.damage_type as u32);
        c.write_i64(x.radius);
        c.write_u32(x.ticks_left);
    }

    // Summoned allies (minions) in id order.
    let mut mn: Vec<&Minion> = s.minions.iter().collect();
    mn.sort_by_key(|x| x.id);
    c.write_u32(mn.len() as u32);
    for x in mn {
        c.write_u32(x.id.0);
        c.write_fixed(x.pos.x);
        c.write_fixed(x.pos.y);
        c.write_u32(x.kind as u32);
        c.write_i64(x.hp);
        c.write_i64(x.damage);
        c.write_u32(x.damage_type as u32);
        c.write_u32(x.next_attack_tick);
        c.write_u32(x.expire_tick);
    }

    c.finish()
}
