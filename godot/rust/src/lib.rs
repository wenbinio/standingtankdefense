//! GDExtension binding: exposes the deterministic `sim` to Godot as a single
//! `StSim` node. It owns an `ArenaState`, advances it one tick per call, and
//! hands the engine flat `Packed*Array`s built from `sim::view` (the
//! engine-agnostic render contract). No game logic lives here — only marshaling.
//!
//! GDScript surface (see ../main.gd):
//!   var sim = StSim.new_match(seed)                    # Normal difficulty
//!   var sim = StSim.new_match_with_difficulty(seed, d) # SP: 0 Easy·1 Normal·2 Hard
//!   sim.difficulty()
//!   sim.step(code, slot)          # 0 Noop · 1 Buy(slot) · 2 Reroll · 3 Clear
//!                                 # 4 BMPick weapon · 5 BMPick upgrade (slot = catalog idx)
//!   sim.black_market_pending()
//!   sim.black_market_choices(weapons) / black_market_choice_names(weapons)
//!   sim.tick(); sim.round(); sim.is_dead()
//!   sim.clear_state() -> [ready_in_ticks, cooldown_total_ticks]
//!   sim.timing() -> [ticks_to_next_round, round_len_ticks, boss_spawn_tick]
//!   sim.tank() -> [x,y,hp,max_hp,revives]
//!   sim.economy() -> [gold,income,rerolls,reroll_cost]
//!   sim.enemies_pos() / enemies_boss() / enemies_kind() / enemies_id()
//!       / enemies_hp_permille() / enemies_status()
//!   sim.projectiles_pos() / projectiles_id() / projectiles_kind()
//!       / projectiles_target()
//!   sim.hazards()                 # flat [x,y,radius,ticks_left,damage_type, …]
//!   sim.minions_pos() / minions_kind() / minions_id()
//!   sim.shop_names() / shop_meta()  # meta: [cost,flags, cost,flags, …]
//!   sim.arsenal_lines()
//!   sim.arsenal_names() / arsenal_meta()   # grouped-arsenal panel (5-int recs)
//!   sim.synergy_names() / synergies_meta() # active self-scalers (5-int recs)
//!   sim.arsenal_rev()             # monotone; GDScript decode-cache key
//!   sim.damage_names() / damage_meta()     # DPS-meter rows (4-int recs)
//!   sim.take_events()             # read-and-clear; see EVENT RECORD LAYOUT
//!
//! Perf note: ONE `RenderView` is built per `step()` and cached; every accessor
//! serves from the cache (`docs/09 §9.3` — kills the rebuild-per-accessor cost).

use godot::prelude::*;
use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::lobby::{Lobby, MatchPlan, Phase, Ruleset, StartReject, MAX_PARTY};
use net::transport::{PeerId, DIRECTOR};
use net::GameSpeed;
use sim::bot::Bot;
use sim::view;
use sim::{ArenaState, Input, SimEvent};

// ===================== sim→render event marshaling =====================
//
// EVENT RECORD LAYOUT (the GDScript consumption contract, `docs/09 §9.3`).
//
// `take_events()` returns a PackedInt64Array of FIXED-WIDTH 6-int records:
//
//     [kind, a, b, c, d, e,  kind, a, b, c, d, e,  …]
//
// Read-and-clear per sim: events buffered since the last `step()` are returned
// once; a second call (or the next `step()`) yields/clears them. Unused slots
// are 0. Positions are integer world units (tank at the origin), the same
// space as `enemies_pos()`.
//
// | kind | event             | a           | b        | c                    | d           | e                     |
// |------|-------------------|-------------|----------|----------------------|-------------|-----------------------|
// |  1   | EnemyKilled       | x           | y        | enemy_kind + 65536*boss_flag | base bounty | fire_explosion_radius (0 = none) |
// |  2   | EnemyDespawned    | enemy id    | —        | —                    | —           | —                     |
// |  3   | Impact            | x           | y        | damage               | damage_type | splash_radius (0 = single-target) |
// |  4   | ProjectileSpawned | weapon_kind | x        | y                    | target_x    | target_y              |
// |  5   | TankHit           | damage      | —        | —                    | —           | —                     |
// |  6   | RoundStart        | round       | —        | —                    | —           | —                     |
// |  7   | BossSpawned       | enemy id    | —        | —                    | —           | —                     |
// |  8   | HazardPlaced      | x           | y        | radius               | ticks       | damage_type           |
// |  9   | HazardExpired     | hazard id   | —        | —                    | —           | —                     |
// | 10   | FreezeProc        | enemy id    | —        | —                    | —           | —                     |
// | 11   | ShieldBroke       | —           | —        | —                    | —           | —                     |
// | 12   | GoldBounty        | amount      | —        | —                    | —           | —                     |
//
// GDScript unpacking for kind 1: `enemy_kind = c & 0xFFFF`, `boss = c >> 16`.
// EnemyKilled's `bounty` is the CATALOG base bounty (for kill popups); the gold
// actually paid this tick (multipliers/procs applied) is kind 12.

/// Number of i64 slots per event record.
const EVENT_RECORD_WIDTH: usize = 6;

/// Flatten drained [`SimEvent`]s to the fixed-width record layout above.
fn encode_events(events: &[SimEvent]) -> PackedInt64Array {
    let mut a = PackedInt64Array::new();
    a.resize(events.len() * EVENT_RECORD_WIDTH);
    for (i, ev) in events.iter().enumerate() {
        let rec: [i64; EVENT_RECORD_WIDTH] = match *ev {
            SimEvent::EnemyKilled {
                x,
                y,
                kind,
                boss,
                bounty,
                fire_explosion_radius,
            } => [
                1,
                x,
                y,
                kind as i64 + ((boss as i64) << 16),
                bounty,
                fire_explosion_radius,
            ],
            SimEvent::EnemyDespawned { id } => [2, id as i64, 0, 0, 0, 0],
            SimEvent::Impact {
                x,
                y,
                damage,
                damage_type,
                splash_radius,
            } => [3, x, y, damage, damage_type as i64, splash_radius],
            SimEvent::ProjectileSpawned {
                weapon_kind,
                x,
                y,
                target_x,
                target_y,
            } => [4, weapon_kind as i64, x, y, target_x, target_y],
            SimEvent::TankHit { damage } => [5, damage, 0, 0, 0, 0],
            SimEvent::RoundStart { round } => [6, round as i64, 0, 0, 0, 0],
            SimEvent::BossSpawned { id } => [7, id as i64, 0, 0, 0, 0],
            SimEvent::HazardPlaced {
                x,
                y,
                radius,
                ticks,
                damage_type,
            } => [8, x, y, radius, ticks as i64, damage_type as i64],
            SimEvent::HazardExpired { id } => [9, id as i64, 0, 0, 0, 0],
            SimEvent::FreezeProc { id } => [10, id as i64, 0, 0, 0, 0],
            SimEvent::ShieldBroke => [11, 0, 0, 0, 0, 0],
            SimEvent::GoldBounty { amount } => [12, amount, 0, 0, 0, 0],
        };
        for (j, v) in rec.iter().enumerate() {
            a[i * EVENT_RECORD_WIDTH + j] = *v;
        }
    }
    a
}

struct StandingTankExt;

#[gdextension]
unsafe impl ExtensionLibrary for StandingTankExt {}

#[derive(GodotClass)]
#[class(no_init, base = RefCounted)]
pub struct StSim {
    state: ArenaState,
    /// The ONE `RenderView` per tick — rebuilt in `step()`, served by every
    /// accessor (no per-accessor snapshot rebuilds).
    view: view::RenderView,
    /// Events drained from the sim at `step()`, held for `take_events()`.
    /// REPLACED each step: undrained events are dropped, never accumulated.
    events: Vec<SimEvent>,
    base: Base<RefCounted>,
}

#[godot_api]
impl StSim {
    /// Start a fresh match with the given RNG seed (Normal difficulty).
    #[func]
    fn new_match(seed: i64) -> Gd<StSim> {
        Self::new_match_with_difficulty(seed, sim::content::DIFF_NORMAL as i64)
    }

    /// Start a fresh SINGLE-PLAYER match at an explicit difficulty preset
    /// (0 Easy · 1 Normal · 2 Hard — out-of-range clamps to Normal). SP-only
    /// by construction: `StMatch`/the director path has no difficulty
    /// parameter and always runs Normal, so the competitive arc is untouched.
    /// The code is authoritative sim state (checksummed, snapshot v23).
    #[func]
    fn new_match_with_difficulty(seed: i64, difficulty: i64) -> Gd<StSim> {
        let code = u8::try_from(difficulty).unwrap_or(sim::content::DIFF_NORMAL);
        let state = ArenaState::new_with_difficulty(seed as u64, 0, code);
        let view = view::snapshot(&state);
        Gd::from_init_fn(|base| StSim {
            state,
            view,
            events: Vec::new(),
            base,
        })
    }

    /// The arena's difficulty preset code (0 Easy · 1 Normal · 2 Hard).
    #[func]
    fn difficulty(&self) -> i64 {
        self.state.difficulty as i64
    }

    /// Advance exactly one sim tick with the player's action this tick.
    ///
    /// Input codes: 0 Noop · 1 BuyOffer(slot) · 2 Reroll · 3 Clear ·
    /// 4 BlackMarketPick WEAPON (`slot` = weapon catalog index) ·
    /// 5 BlackMarketPick UPGRADE (`slot` = modifier catalog index).
    ///
    /// GAME SPEED (local play): the sim is tick-indexed and needs NOTHING
    /// sim-side for Fast/Faster/Hyper — the frontend simply calls `step()`
    /// more often (45/60/90 times per second instead of 30; the
    /// Normal/Fast/Faster/Hyper table lives on `net::wire::GameSpeed` and is
    /// surfaced by `StLobby.ticks_per_second()` / `StMatch.ticks_per_second()`).
    #[func]
    fn step(&mut self, input_code: i64, slot: i64) {
        let inp = match input_code {
            1 => Input::BuyOffer { slot: slot as u8 },
            2 => Input::Reroll,
            3 => Input::Clear,
            4 => Input::BlackMarketPick {
                is_weapon: true,
                index: slot as u8,
            },
            5 => Input::BlackMarketPick {
                is_weapon: false,
                index: slot as u8,
            },
            _ => Input::Noop,
        };
        sim::step(&mut self.state, inp);
        self.view = view::snapshot(&self.state);
        self.events = self.state.events.take();
    }

    /// Whether a Black Market pick is currently held ("Buy 1 Uncommon Weapon
    /// or Spikes Damage Upgrade of your choosing. The Black Market lasts until
    /// a choice is made.") — when true, the UI should offer the picker and
    /// submit input code 4 (weapon) or 5 (upgrade) with the chosen catalog
    /// index. An illegal pick is a deterministic sim no-op.
    #[func]
    fn black_market_pending(&self) -> bool {
        self.state.pending_black_market
    }

    /// Catalog indices of the legal Black Market picks: the Uncommon weapons
    /// (`weapons = true`) or the Uncommon Spikes-damage upgrades
    /// (`weapons = false`). Parallel to `black_market_choice_names`.
    #[func]
    fn black_market_choices(&self, weapons: bool) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        let len = if weapons {
            sim::content::WEAPONS.len()
        } else {
            sim::content::MODIFIERS.len()
        };
        for i in 0..len {
            if sim::content::black_market_eligible(weapons, i) {
                a.push(i as i64);
            }
        }
        a
    }

    /// Display names for `black_market_choices(weapons)`, in the same order.
    #[func]
    fn black_market_choice_names(&self, weapons: bool) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        let len = if weapons {
            sim::content::WEAPONS.len()
        } else {
            sim::content::MODIFIERS.len()
        };
        for i in 0..len {
            if sim::content::black_market_eligible(weapons, i) {
                let name = if weapons {
                    sim::content::WEAPONS[i].name
                } else {
                    sim::content::MODIFIERS[i].name
                };
                a.push(&GString::from(name));
            }
        }
        a
    }

    /// Drain this tick's sim→render events as flat 6-int records — see the
    /// EVENT RECORD LAYOUT table at the top of this file. Read-and-clear.
    #[func]
    fn take_events(&mut self) -> PackedInt64Array {
        let drained = std::mem::take(&mut self.events);
        encode_events(&drained)
    }

    #[func]
    fn tick(&self) -> i64 {
        self.state.tick as i64
    }
    #[func]
    fn round(&self) -> i64 {
        self.state.round as i64
    }
    #[func]
    fn is_dead(&self) -> bool {
        self.state.dead
    }

    /// `[ready_in_ticks, cooldown_total_ticks]` for the Clear ability.
    /// `ready_in_ticks == 0` ⇒ Clear is ready NOW; otherwise it is the ticks
    /// remaining, out of `cooldown_total_ticks` (HUD cooldown indicator).
    #[func]
    fn clear_state(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for v in [
            self.view.clear_ready_in as i64,
            self.view.clear_cooldown_total as i64,
        ] {
            a.push(v);
        }
        a
    }

    /// `[ticks_to_next_round, round_len_ticks, boss_spawn_tick]` — match
    /// pacing for the HUD: countdown to the next round/shop refresh, the round
    /// length, and the fixed tick the boss enters the arena.
    #[func]
    fn timing(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for v in [
            self.view.ticks_to_next_round as i64,
            sim::ROUND_TICKS as i64,
            sim::content::BOSS_SPAWN_TICK as i64,
        ] {
            a.push(v);
        }
        a
    }

    /// `[x, y, hp, max_hp, revives]`.
    #[func]
    fn tank(&self) -> PackedInt64Array {
        let t = self.view.tank;
        let mut a = PackedInt64Array::new();
        for v in [t.x, t.y, t.hp, t.max_hp, t.revives as i64] {
            a.push(v);
        }
        a
    }

    /// `[gold, income_per_tick, rerolls_remaining, reroll_cost]`.
    #[func]
    fn economy(&self) -> PackedInt64Array {
        let e = self.view.economy;
        let mut a = PackedInt64Array::new();
        for v in [
            e.gold,
            e.income_per_tick,
            e.rerolls_remaining as i64,
            e.reroll_cost,
        ] {
            a.push(v);
        }
        a
    }

    /// World positions of every enemy (units); parallel to the flag arrays below.
    #[func]
    fn enemies_pos(&self) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        for e in &self.view.enemies {
            a.push(Vector2::new(e.x as f32, e.y as f32));
        }
        a
    }

    /// Per-enemy boss flag (1 boss, 0 normal).
    #[func]
    fn enemies_boss(&self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for e in &self.view.enemies {
            a.push(e.boss as u8);
        }
        a
    }

    /// Per-enemy catalog kind index (0 Squeakzilla · 1 Fanged Death · 2 boss),
    /// parallel to `enemies_pos()` — selects the sprite.
    #[func]
    fn enemies_kind(&self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for e in &self.view.enemies {
            a.push(e.kind as u8);
        }
        a
    }

    /// Per-enemy stable id, parallel to `enemies_pos()`. The front-end diffs
    /// these between frames to drive juice (hit flash on hp drop, death poof on
    /// an id that vanished).
    #[func]
    fn enemies_id(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for e in &self.view.enemies {
            a.push(e.id as i64);
        }
        a
    }

    /// Per-enemy HP as a permille of catalog base HP (for a health bar).
    #[func]
    fn enemies_hp_permille(&self) -> PackedInt32Array {
        let mut a = PackedInt32Array::new();
        for e in &self.view.enemies {
            let r = if e.base_hp > 0 {
                (e.hp.max(0) * 1000 / e.base_hp) as i32
            } else {
                0
            };
            a.push(r);
        }
        a
    }

    /// Per-enemy status flag byte, parallel to `enemies_pos()`: bit0 frost ·
    /// bit1 poison · bit2 fire · bit3 vuln · bit4 stun · bit5 freeze — drives
    /// status tints and looping FX.
    #[func]
    fn enemies_status(&self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for e in &self.view.enemies {
            a.push(e.status_flags);
        }
        a
    }

    /// World positions of in-flight projectiles.
    #[func]
    fn projectiles_pos(&self) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        for p in &self.view.projectiles {
            a.push(Vector2::new(p.x as f32, p.y as f32));
        }
        a
    }

    /// Per-projectile stable id, parallel to `projectiles_pos()` (drives
    /// cross-frame interpolation).
    #[func]
    fn projectiles_id(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for p in &self.view.projectiles {
            a.push(p.id as i64);
        }
        a
    }

    /// Per-projectile weapon catalog index, parallel to `projectiles_pos()`
    /// (selects the projectile sprite).
    #[func]
    fn projectiles_kind(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for p in &self.view.projectiles {
            a.push(p.kind as i64);
        }
        a
    }

    /// Per-projectile last known target position, parallel to
    /// `projectiles_pos()` (orients the sprite along its flight path).
    #[func]
    fn projectiles_target(&self) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        for p in &self.view.projectiles {
            a.push(Vector2::new(p.target_x as f32, p.target_y as f32));
        }
        a
    }

    /// Active hazards as flat 5-int records `[x, y, radius, ticks_left,
    /// damage_type, …]` (mine fields / burning oil to draw).
    #[func]
    fn hazards(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for h in &self.view.hazards {
            for v in [
                h.x,
                h.y,
                h.radius,
                h.ticks_left as i64,
                h.damage_type as i64,
            ] {
                a.push(v);
            }
        }
        a
    }

    /// World positions of summoned allies (parallel to `minions_kind`).
    #[func]
    fn minions_pos(&self) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        for m in &self.view.minions {
            a.push(Vector2::new(m.x as f32, m.y as f32));
        }
        a
    }

    /// Per-minion sprite kind (0 larvae · 1 spores), parallel to `minions_pos`.
    #[func]
    fn minions_kind(&self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for m in &self.view.minions {
            a.push(m.kind);
        }
        a
    }

    /// Per-minion stable id, parallel to `minions_pos()` (drives cross-frame
    /// interpolation).
    #[func]
    fn minions_id(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for m in &self.view.minions {
            a.push(m.id as i64);
        }
        a
    }

    /// Names of the current shop offers (slot order).
    #[func]
    fn shop_names(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for o in &self.view.shop {
            a.push(&GString::from(o.name));
        }
        a
    }

    /// Flat `[cost, flags, rarity, …]` per offer; flags bit0 = is_weapon,
    /// bit1 = affordable; rarity 0 common · 1 uncommon · 2 rare · 3 epic.
    #[func]
    fn shop_meta(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for o in &self.view.shop {
            a.push(o.cost);
            a.push((o.is_weapon as i64) | ((o.affordable as i64) << 1));
            a.push(o.rarity as i64);
        }
        a
    }

    /// Flat `[flavor, tip, …]` per offer (slot order): the item's flavor blurb
    /// and a terse mechanical tip, for shop tooltips.
    #[func]
    fn shop_desc(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for off in &self.state.shop.offers {
            let (flavor, tip) = match off.kind {
                sim::OfferKind::Weapon => sim::descriptions::weapon_text(off.def),
                sim::OfferKind::Modifier => sim::descriptions::modifier_text(off.def),
            };
            a.push(&GString::from(flavor));
            a.push(&GString::from(tip));
        }
        a
    }

    /// `"Name xN"` lines for the owned arsenal.
    #[func]
    fn arsenal_lines(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for e in &self.view.arsenal {
            a.push(&GString::from(format!("{} x{}", e.name, e.count).as_str()));
        }
        a
    }

    /// Per owned-weapon-stack display names (name-sorted), parallel to
    /// `arsenal_meta()`.
    #[func]
    fn arsenal_names(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for e in &self.view.arsenal {
            a.push(&GString::from(e.name));
        }
        a
    }

    /// Flat 5-int records per owned weapon stack, parallel to
    /// `arsenal_names()`: `[kind, count, class_id, damage_type, rarity, …]` —
    /// `kind` = weapon catalog index; `class_id` 0 Single · 1 Splash ·
    /// 2 Barrage · 3 Area · 4 Wave · 5 Bounce; `damage_type` 0 Normal ·
    /// 1 Piercing · 2 Magic · 3 Siege · 4 Chaos; `rarity` 0..=3.
    #[func]
    fn arsenal_meta(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for e in &self.view.arsenal {
            for v in [
                e.kind as i64,
                e.count as i64,
                e.class_id as i64,
                e.damage_type as i64,
                e.rarity as i64,
            ] {
                a.push(v);
            }
        }
        a
    }

    /// Source-weapon display names per owned self-scaling rule, parallel to
    /// `synergies_meta()`.
    #[func]
    fn synergy_names(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for s in &self.view.synergies {
            a.push(&GString::from(s.source_name));
        }
        a
    }

    /// Flat 5-int records per owned self-scaling rule ("+X% <type> per
    /// <weapon>"), parallel to `synergy_names()`:
    /// `[source_kind, damage_type, per_copy_milli_pct, count,
    /// bonus_milli_pct, …]` — percentages in milli-percent (1000 ⇒ +1%);
    /// `count` is the CURRENT owned count of `source_kind` and
    /// `bonus_milli_pct` the resolved per-copy × count bonus.
    #[func]
    fn synergies_meta(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for s in &self.view.synergies {
            for v in [
                s.source_kind as i64,
                s.damage_type as i64,
                s.per_copy_milli_pct,
                s.count as i64,
                s.bonus_milli_pct,
            ] {
                a.push(v);
            }
        }
        a
    }

    /// Monotone arsenal revision: moves iff a weapon instance or a
    /// self-scaling rule was added (both are append-only in the sim), so the
    /// GDScript side caches its decoded arsenal/synergy dicts on this edge
    /// instead of re-decoding per frame.
    #[func]
    fn arsenal_rev(&self) -> i64 {
        ((self.state.weapons.len() as i64) << 20)
            | self.state.modifiers.weapon_count_scaling.len() as i64
    }

    /// `[damage_dealt, gold_earned]` — match scoreboard totals.
    #[func]
    fn stats(&self) -> PackedInt64Array {
        let v = &self.view;
        let mut a = PackedInt64Array::new();
        a.push(v.stats.damage_dealt);
        a.push(v.stats.gold_earned);
        a.push(v.stats.bought_attack_mask as i64);
        a.push(v.stats.weapons_bought as i64);
        a.push(v.stats.economy_purchases as i64);
        a
    }

    /// Per-source display names for the damage-attribution rows (DPS meter),
    /// parallel to `damage_meta()`. English catalog strings ("Spikes" /
    /// "Clear" / "Other" for the pseudo rows) — tr() them at the draw boundary.
    #[func]
    fn damage_names(&self) -> PackedStringArray {
        damage_names_impl(&self.view)
    }

    /// Flat 4-int records per damage-attribution row, parallel to
    /// `damage_names()`: `[source, damage_type, count, total, …]` — `source` =
    /// weapon catalog index (or a reserved pseudo id ≥ 0xFFFD: Spikes / Clear /
    /// Other); `damage_type` 0..=4 (255 for pseudo rows); `count` = copies
    /// owned; `total` = lifetime damage attributed. Rows arrive in ascending
    /// source order; ranking/percentages are the overlay's job.
    #[func]
    fn damage_meta(&self) -> PackedInt64Array {
        damage_meta_impl(&self.view)
    }
}

/// Marshal the view's damage-attribution rows' names (shared StSim/StMatch).
fn damage_names_impl(v: &view::RenderView) -> PackedStringArray {
    let mut a = PackedStringArray::new();
    for d in &v.damage_by_weapon {
        a.push(&GString::from(d.name));
    }
    a
}

/// Marshal the view's damage-attribution rows' facts (shared StSim/StMatch).
fn damage_meta_impl(v: &view::RenderView) -> PackedInt64Array {
    let mut a = PackedInt64Array::new();
    for d in &v.damage_by_weapon {
        for x in [
            d.source as i64,
            d.damage_type as i64,
            d.count as i64,
            d.total,
        ] {
            a.push(x);
        }
    }
    a
}

// ===================== Multi-arena / net view =====================

/// Arbitrary content hash for the demo clients (the M2 director doesn't gate it).
const DEMO_CONTENT_HASH: u64 = 0xC0DE_C0DE;

/// Decode the GDScript-facing `(code, slot)` input pair into a sim [`Input`] —
/// the SAME table as `StSim::step` (0 Noop · 1 BuyOffer(slot) · 2 Reroll ·
/// 3 Clear · 4 BlackMarketPick weapon · 5 BlackMarketPick upgrade, `slot` =
/// catalog index for 4/5). Used by [`StMatch::queue_player_input`] so the human
/// seat's intents share the single-arena numbering exactly.
fn input_from_code(code: i64, slot: i64) -> Input {
    match code {
        1 => Input::BuyOffer { slot: slot as u8 },
        2 => Input::Reroll,
        3 => Input::Clear,
        4 => Input::BlackMarketPick {
            is_weapon: true,
            index: slot as u8,
        },
        5 => Input::BlackMarketPick {
            is_weapon: false,
            index: slot as u8,
        },
        _ => Input::Noop,
    }
}

/// Catalog indices of the legal Black Market picks (mirrors
/// `StSim::black_market_choices` — a pure function of the catalog, identical
/// for every player, so `StMatch` serves it without a player index).
fn black_market_choices_impl(weapons: bool) -> PackedInt64Array {
    let mut a = PackedInt64Array::new();
    let len = if weapons {
        sim::content::WEAPONS.len()
    } else {
        sim::content::MODIFIERS.len()
    };
    for i in 0..len {
        if sim::content::black_market_eligible(weapons, i) {
            a.push(i as i64);
        }
    }
    a
}

/// Display names parallel to [`black_market_choices_impl`] (mirrors
/// `StSim::black_market_choice_names`).
fn black_market_choice_names_impl(weapons: bool) -> PackedStringArray {
    let mut a = PackedStringArray::new();
    let len = if weapons {
        sim::content::WEAPONS.len()
    } else {
        sim::content::MODIFIERS.len()
    };
    for i in 0..len {
        if sim::content::black_market_eligible(weapons, i) {
            let name = if weapons {
                sim::content::WEAPONS[i].name
            } else {
                sim::content::MODIFIERS[i].name
            };
            a.push(&GString::from(name));
        }
    }
    a
}

/// A full N-player match running the REAL netcode loop — authoritative
/// [`Director`] + per-player [`Client`]s wired through the deterministic [`Hub`],
/// each client driven by the shared [`Bot`]. It renders every player's
/// authoritative shadow arena, showcasing the sharded-simulation architecture:
/// N independent arenas advancing under one director, no entity replication.
///
/// HUMAN MODE: `set_human_seat(i)` disables seat `i`'s bot; the frontend then
/// feeds that seat's intents via `queue_player_input(code, slot)` (one per
/// `step()`, same code table as `StSim.step`). The intent flows the NORMAL
/// client→director path — sent as a wire `Msg::Input`, validated/acked by the
/// director, scheduled at the acked apply tick on both shadows (`docs/03`
/// authority: no back door, the human is just another client). Without
/// `set_human_seat` every seat stays bot-driven (the spectate demo).
#[derive(GodotClass)]
#[class(no_init, base = RefCounted)]
pub struct StMatch {
    director: Director,
    clients: Vec<Client>,
    bots: Vec<Bot>,
    hub: Hub,
    peers: Vec<PeerId>,
    /// Seat driven by a human instead of its bot (`None` = all bots).
    human_seat: Option<usize>,
    /// The human seat's ONE queued intent, consumed (and reset to `Noop`) by
    /// the next `step()`. REPLACED by a newer intent, never accumulated — the
    /// frontend's FIFO owns ordering and feeds exactly one intent per tick.
    human_input: Input,
    /// ONE cached `RenderView` per player per `step()` (`docs/09 §9.3`) —
    /// every per-player accessor serves from here, not a fresh snapshot.
    views: Vec<Option<view::RenderView>>,
    /// Per-player events copied from the authoritative shadows at `step()`,
    /// held for `take_events(i)`. REPLACED each step (drop-if-undrained).
    events: Vec<Vec<SimEvent>>,
    base: Base<RefCounted>,
}

#[godot_api]
impl StMatch {
    /// Start an `n`-player match (clamped 1..=16) seeded with `seed`, at
    /// Normal speed. Use [`StMatch::new_match_at_speed`] for a host-set pace.
    #[func]
    fn new_match(n: i64, seed: i64) -> Gd<StMatch> {
        Self::new_match_at_speed(n, seed, 0)
    }

    /// Start an `n`-player match at the host-set game speed (`speed_code`:
    /// 0 Normal ×1.0 / 1 Fast ×1.5 / 2 Faster ×2.0 / 3 Hyper ×3.0 — i.e.
    /// 30/45/60/90 ticks per second; out-of-range codes fall back to Normal;
    /// pass `StLobby.game_speed()` after a started plan). Speed is CADENCE
    /// only: the frontend calls `step()` `ticks_per_second()` times per
    /// wall-clock second; per step everything is bit-identical to Normal.
    #[func]
    fn new_match_at_speed(n: i64, seed: i64, speed_code: i64) -> Gd<StMatch> {
        let speed = GameSpeed::from_u8(u8::try_from(speed_code).unwrap_or(255)).unwrap_or_default();
        let n = n.clamp(1, 16) as u32;
        let peers: Vec<PeerId> = (1..=n).map(PeerId).collect();
        let director = Director::with_config(&peers, seed as u64, 0, speed);
        let clients = peers
            .iter()
            .map(|p| Client::new(*p, DEMO_CONTENT_HASH))
            .collect();
        let bots = peers.iter().map(|_| Bot::default()).collect();
        let views = peers
            .iter()
            .map(|p| director.shadow(*p).map(view::snapshot))
            .collect();
        let events = peers.iter().map(|_| Vec::new()).collect();
        Gd::from_init_fn(|base| StMatch {
            director,
            clients,
            bots,
            hub: Hub::new(),
            peers,
            human_seat: None,
            human_input: Input::Noop,
            views,
            events,
            base,
        })
    }

    /// Hand seat `i` to a human: that seat's bot is fully disabled and its
    /// client's desired action each iteration comes from `queue_player_input`
    /// instead (Noop when nothing is queued). An out-of-range `i` reverts to
    /// all-bots (the spectate demo). Call once right after construction.
    #[func]
    fn set_human_seat(&mut self, i: i64) {
        self.human_seat = usize::try_from(i).ok().filter(|x| *x < self.peers.len());
        self.human_input = Input::Noop;
    }

    /// Queue the human seat's ONE `(code, slot)` intent for the NEXT `step()`
    /// — same input table as `StSim.step` (1 Buy(slot) · 2 Reroll · 3 Clear ·
    /// 4/5 Black-Market pick with `slot` = catalog index). Replaces any
    /// unconsumed intent; a no-op while no human seat is set. The intent is
    /// SUBMITTED by the seat's client on the next iteration and applies at the
    /// director-acked apply tick (input lead), like every other client input.
    #[func]
    fn queue_player_input(&mut self, code: i64, slot: i64) {
        if self.human_seat.is_some() {
            self.human_input = input_from_code(code, slot);
        }
    }

    /// Constrain player `i`'s bot to a challenge (see `bot::Challenge::from_code`:
    /// 0 none, 1..=6 purist of attack-class 0..5, 7 no-economy, 8 jack-of-all).
    /// Cosmetic preview aid — it only filters that bot's shop choices.
    #[func]
    fn set_challenge(&mut self, i: i64, code: i64) {
        let c = sim::bot::Challenge::from_code(code);
        let idx = i as usize;
        if let Some(b) = self.bots.get_mut(idx) {
            *b = Bot::with_challenge(c);
        }
        // Mirror onto the authoritative director + this client so the buy-filter
        // is applied identically on both shadows (they stay in lockstep).
        self.director.set_challenge(idx, c);
        if let Some(cl) = self.clients.get_mut(idx) {
            cl.set_challenge(c);
        }
    }

    /// Advance the whole match one server iteration: the director steps every
    /// alive shadow, and each client sends its bot's chosen input — all through
    /// the real `Hub` transport, exactly like the netcode integration tests.
    #[func]
    fn step(&mut self) {
        let in_d = self.hub.take(DIRECTOR);
        let out_d = self.director.tick(in_d);
        self.hub.send(DIRECTOR, out_d);
        for (i, p) in self.peers.iter().enumerate() {
            let in_i = self.hub.take(*p);
            // The human seat's bot never runs: its desired action is the queued
            // player intent (consumed here, at most one per step). Everything
            // downstream — submit, ack, schedule, apply — is the identical
            // client path the bots use.
            let desired = if self.human_seat == Some(i) {
                std::mem::replace(&mut self.human_input, Input::Noop)
            } else {
                match self.director.shadow(*p) {
                    Some(sh) => self.bots[i].decide(sh),
                    None => Input::Noop,
                }
            };
            let out_i = self.clients[i].tick(in_i, desired);
            self.hub.send(*p, out_i);
        }
        self.hub.advance();

        // Refresh the per-player render caches from the authoritative shadows:
        // one view per player per step, plus this tick's events. A shadow that
        // did not advance (same tick as the cached view) contributes NO events —
        // its buffer is last tick's leftovers, already delivered once.
        for (i, p) in self.peers.iter().enumerate() {
            match self.director.shadow(*p) {
                Some(st) => {
                    let advanced = self.views[i].as_ref().is_none_or(|v| v.tick != st.tick);
                    self.events[i] = if advanced {
                        st.events.as_slice().to_vec()
                    } else {
                        Vec::new()
                    };
                    self.views[i] = Some(view::snapshot(st));
                }
                None => {
                    self.events[i].clear();
                    self.views[i] = None;
                }
            }
        }
    }

    /// Drain player `i`'s sim→render events from the last `step()` as flat
    /// 6-int records — see the EVENT RECORD LAYOUT table at the top of this
    /// file. Read-and-clear per player.
    #[func]
    fn take_events(&mut self, i: i64) -> PackedInt64Array {
        match self.events.get_mut(i as usize) {
            Some(evs) => {
                let drained = std::mem::take(evs);
                encode_events(&drained)
            }
            None => PackedInt64Array::new(),
        }
    }

    #[func]
    fn player_count(&self) -> i64 {
        self.peers.len() as i64
    }
    #[func]
    fn server_tick(&self) -> i64 {
        self.director.server_tick() as i64
    }
    /// The active host-set game speed code (0 Normal · 1 Fast · 2 Faster ·
    /// 3 Hyper), fixed at match start.
    #[func]
    fn game_speed(&self) -> i64 {
        self.director.game_speed().as_u8() as i64
    }
    /// How many times per wall-clock second the frontend should call `step()`
    /// for the active speed (30/45/60/90).
    #[func]
    fn ticks_per_second(&self) -> i64 {
        self.director.ticks_per_second() as i64
    }
    #[func]
    fn alive_count(&self) -> i64 {
        self.peers
            .iter()
            .filter(|p| self.director.is_alive(**p))
            .count() as i64
    }
    #[func]
    fn is_alive(&self, i: i64) -> bool {
        self.peers
            .get(i as usize)
            .is_some_and(|p| self.director.is_alive(*p))
    }
    #[func]
    fn match_over(&self) -> bool {
        self.director.result().is_some()
    }
    /// 1-based final placement for player `i` (1 = winner), or 0 if undecided.
    #[func]
    fn placement(&self, i: i64) -> i64 {
        let p = match self.peers.get(i as usize) {
            Some(p) => *p,
            None => return 0,
        };
        match self.director.result() {
            Some(r) => r
                .iter()
                .find(|(pp, _)| *pp == p)
                .map(|(_, place)| *place as i64)
                .unwrap_or(0),
            None => 0,
        }
    }

    /// Player `i`'s cached view (rebuilt once per `step()`).
    fn snap(&self, i: i64) -> Option<&view::RenderView> {
        self.views.get(i as usize).and_then(|v| v.as_ref())
    }

    /// `[x, y, hp, max_hp, revives, round, tick, dead]` for player `i`.
    #[func]
    fn arena(&self, i: i64) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(v) = self.snap(i) {
            for x in [
                v.tank.x,
                v.tank.y,
                v.tank.hp,
                v.tank.max_hp,
                v.tank.revives as i64,
                v.round as i64,
                v.tick as i64,
                v.dead as i64,
            ] {
                a.push(x);
            }
        }
        a
    }

    /// `[gold, income_per_tick, rerolls_remaining, reroll_cost]` for player `i`
    /// — the SAME layout as `StSim.economy()`, so the shared SimView economy
    /// accessors (gold/income/free_rerolls/reroll_cost) read both identically.
    #[func]
    fn economy(&self, i: i64) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(v) = self.snap(i) {
            for x in [
                v.economy.gold,
                v.economy.income_per_tick,
                v.economy.rerolls_remaining as i64,
                v.economy.reroll_cost,
            ] {
                a.push(x);
            }
        }
        a
    }

    /// `[ready_in_ticks, cooldown_total_ticks]` for player `i`'s Clear ability
    /// (mirrors `StSim.clear_state()`; empty while the shadow is unreadable).
    #[func]
    fn clear_state(&self, i: i64) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(v) = self.snap(i) {
            a.push(v.clear_ready_in as i64);
            a.push(v.clear_cooldown_total as i64);
        }
        a
    }

    /// Names of player `i`'s current shop offers, slot order (mirrors
    /// `StSim.shop_names()`; empty once the shop closes at the boss).
    #[func]
    fn shop_names(&self, i: i64) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        if let Some(v) = self.snap(i) {
            for o in &v.shop {
                a.push(&GString::from(o.name));
            }
        }
        a
    }

    /// Flat `[cost, flags, rarity, …]` per offer for player `i` (mirrors
    /// `StSim.shop_meta()`: flags bit0 = is_weapon, bit1 = affordable).
    #[func]
    fn shop_meta(&self, i: i64) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(v) = self.snap(i) {
            for o in &v.shop {
                a.push(o.cost);
                a.push((o.is_weapon as i64) | ((o.affordable as i64) << 1));
                a.push(o.rarity as i64);
            }
        }
        a
    }

    /// Flat `[flavor, tip, …]` per offer for player `i` (mirrors
    /// `StSim.shop_desc()` — shop tooltips for the human seat).
    #[func]
    fn shop_desc(&self, i: i64) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        let shadow = self
            .peers
            .get(i as usize)
            .and_then(|p| self.director.shadow(*p));
        if let Some(st) = shadow {
            for off in &st.shop.offers {
                let (flavor, tip) = match off.kind {
                    sim::OfferKind::Weapon => sim::descriptions::weapon_text(off.def),
                    sim::OfferKind::Modifier => sim::descriptions::modifier_text(off.def),
                };
                a.push(&GString::from(flavor));
                a.push(&GString::from(tip));
            }
        }
        a
    }

    /// Whether player `i`'s authoritative shadow holds a Black Market pick
    /// (mirrors `StSim.black_market_pending()` — drives the picker overlay for
    /// the human seat; bots redeem theirs on their own).
    #[func]
    fn black_market_pending(&self, i: i64) -> bool {
        self.peers
            .get(i as usize)
            .and_then(|p| self.director.shadow(*p))
            .is_some_and(|st| st.pending_black_market)
    }

    /// Catalog indices of the legal Black Market picks (identical for every
    /// player — a pure function of the catalog; mirrors
    /// `StSim.black_market_choices`).
    #[func]
    fn black_market_choices(&self, weapons: bool) -> PackedInt64Array {
        black_market_choices_impl(weapons)
    }

    /// Display names for `black_market_choices(weapons)`, in the same order.
    #[func]
    fn black_market_choice_names(&self, weapons: bool) -> PackedStringArray {
        black_market_choice_names_impl(weapons)
    }

    /// Enemy world positions for player `i`'s arena.
    #[func]
    fn enemies_pos(&self, i: i64) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        if let Some(v) = self.snap(i) {
            for e in &v.enemies {
                a.push(Vector2::new(e.x as f32, e.y as f32));
            }
        }
        a
    }

    /// Enemy sprite kinds for player `i`'s arena (parallel to `enemies_pos`).
    #[func]
    fn enemies_kind(&self, i: i64) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        if let Some(v) = self.snap(i) {
            for e in &v.enemies {
                a.push(e.kind as u8);
            }
        }
        a
    }

    /// Summoned-ally world positions for player `i`'s arena.
    #[func]
    fn minions_pos(&self, i: i64) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        if let Some(v) = self.snap(i) {
            for m in &v.minions {
                a.push(Vector2::new(m.x as f32, m.y as f32));
            }
        }
        a
    }

    /// Summoned-ally sprite kinds for player `i` (0 larvae · 1 spores).
    #[func]
    fn minions_kind(&self, i: i64) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        if let Some(v) = self.snap(i) {
            for m in &v.minions {
                a.push(m.kind);
            }
        }
        a
    }

    /// Number of weapons player `i` has bought (for a quick HUD readout).
    #[func]
    fn weapon_count(&self, i: i64) -> i64 {
        self.snap(i)
            .map_or(0, |v| v.arsenal.iter().map(|a| a.count as i64).sum())
    }

    /// Damage-attribution row names for player `i` (mirrors
    /// `StSim.damage_names()`; empty while the shadow is unreadable).
    #[func]
    fn damage_names(&self, i: i64) -> PackedStringArray {
        self.snap(i)
            .map_or_else(PackedStringArray::new, damage_names_impl)
    }

    /// Damage-attribution row facts for player `i` (mirrors
    /// `StSim.damage_meta()`: flat `[source, damage_type, count, total, …]`).
    #[func]
    fn damage_meta(&self, i: i64) -> PackedInt64Array {
        self.snap(i)
            .map_or_else(PackedInt64Array::new, damage_meta_impl)
    }

    /// `[damage_dealt, gold_earned]` scoreboard totals for player `i`.
    #[func]
    fn stats(&self, i: i64) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(v) = self.snap(i) {
            a.push(v.stats.damage_dealt);
            a.push(v.stats.gold_earned);
            a.push(v.stats.bought_attack_mask as i64);
            a.push(v.stats.weapons_bought as i64);
            a.push(v.stats.economy_purchases as i64);
        }
        a
    }
}

/// Host-authoritative **lobby** (`docs/07 §7.3`) exposed to the engine. Thin
/// wrapper over the deterministic `net::lobby::Lobby` state machine: membership,
/// ready-gating, and the `MatchPlan` (`master_seed` + player list) that launches
/// a match. Cosmetics (each player's theme/skin) live in the engine layer, not
/// here — the GDScript lobby UI layers them on by peer index. Over Steam the
/// same model is fed by `ISteamMatchmaking`; this binding drives a local lobby.
#[derive(GodotClass)]
#[class(no_init, base = RefCounted)]
pub struct StLobby {
    lobby: Lobby,
    plan: Option<MatchPlan>,
    base: Base<RefCounted>,
}

#[godot_api]
impl StLobby {
    /// Open a lobby with you as host (peer 0), seated and ready, phase Filling.
    #[func]
    fn host() -> Gd<StLobby> {
        Gd::from_init_fn(|base| StLobby {
            lobby: Lobby::new(DIRECTOR, DEMO_CONTENT_HASH, Ruleset::standard()),
            plan: None,
            base,
        })
    }

    /// Seat the next free peer (1..=MAX_PARTY-1) — simulates another player
    /// joining. Returns the new peer id, or -1 if the lobby is full/closed.
    #[func]
    fn add_member(&mut self) -> i64 {
        for id in 1..MAX_PARTY as u32 {
            let p = PeerId(id);
            if !self.lobby.contains(p) {
                return match self.lobby.join(p, DEMO_CONTENT_HASH) {
                    Ok(()) => id as i64,
                    Err(_) => -1,
                };
            }
        }
        -1
    }

    /// Remove a member by peer id. Returns true if one was removed.
    #[func]
    fn leave(&mut self, peer: i64) -> bool {
        self.lobby.leave(PeerId(peer as u32))
    }

    /// Set a member's ready flag. Returns true if the member exists.
    #[func]
    fn set_ready(&mut self, peer: i64, ready: bool) -> bool {
        self.lobby.set_ready(PeerId(peer as u32), ready)
    }

    #[func]
    fn all_ready(&self) -> bool {
        self.lobby.all_ready()
    }

    /// Host sets the match pace before start (`speed_code`: 0 Normal ×1.0 /
    /// 1 Fast ×1.5 / 2 Faster ×2.0 / 3 Hyper ×3.0 = 30/45/60/90 ticks/sec).
    /// Returns false for an invalid code or once the match has started —
    /// game speed is match-start-only, there is no mid-match change.
    #[func]
    fn set_game_speed(&mut self, speed_code: i64) -> bool {
        match GameSpeed::from_u8(u8::try_from(speed_code).unwrap_or(255)) {
            Some(speed) => self.lobby.set_game_speed(speed),
            None => false,
        }
    }
    /// The lobby's current game speed code (0 Normal · 1 Fast · 2 Faster ·
    /// 3 Hyper). After a successful `try_start` this is the speed the match
    /// runs at (pass it to `StMatch.new_match_at_speed`).
    #[func]
    fn game_speed(&self) -> i64 {
        self.lobby.ruleset().game_speed.as_u8() as i64
    }
    /// Driver cadence for the lobby's current speed: how many sim ticks per
    /// wall-clock second the match will run at (30/45/60/90).
    #[func]
    fn ticks_per_second(&self) -> i64 {
        self.lobby.ruleset().game_speed.ticks_per_second() as i64
    }
    /// 0 = Filling, 1 = Ready, 2 = Started.
    #[func]
    fn phase(&self) -> i64 {
        match self.lobby.phase() {
            Phase::Filling => 0,
            Phase::Ready => 1,
            Phase::Started => 2,
        }
    }
    #[func]
    fn host_peer(&self) -> i64 {
        self.lobby.host().0 as i64
    }
    #[func]
    fn member_count(&self) -> i64 {
        self.lobby.members().len() as i64
    }
    /// Peer id of the `i`-th member (members are kept sorted by peer id).
    #[func]
    fn member_peer(&self, i: i64) -> i64 {
        self.lobby
            .members()
            .get(i as usize)
            .map_or(-1, |m| m.peer.0 as i64)
    }
    #[func]
    fn member_ready(&self, i: i64) -> bool {
        self.lobby
            .members()
            .get(i as usize)
            .is_some_and(|m| m.ready)
    }
    /// True if the `i`-th member is the host.
    #[func]
    fn is_host_member(&self, i: i64) -> bool {
        let host = self.lobby.host();
        self.lobby
            .members()
            .get(i as usize)
            .is_some_and(|m| m.peer == host)
    }

    /// Try to start with host-minted `seed`. Returns 0 on success (the plan is
    /// stored — read it via `plan_*`), or a negative reject code:
    /// -1 not all ready, -2 not enough players, -3 already started.
    #[func]
    fn try_start(&mut self, seed: i64) -> i64 {
        match self.lobby.start(seed as u64) {
            Ok(plan) => {
                self.plan = Some(plan);
                0
            }
            Err(StartReject::NotAllReady) => -1,
            Err(StartReject::NotEnoughPlayers) => -2,
            Err(StartReject::AlreadyStarted) => -3,
        }
    }

    /// Non-host player peers in the started plan (empty until `try_start` succeeds).
    #[func]
    fn plan_players(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(p) = &self.plan {
            for peer in &p.players {
                a.push(peer.0 as i64);
            }
        }
        a
    }
    /// The started match's master seed (0 until `try_start` succeeds).
    #[func]
    fn plan_seed(&self) -> i64 {
        self.plan.as_ref().map_or(0, |p| p.master_seed as i64)
    }
    /// Number of players in the started plan (0 until `try_start` succeeds).
    #[func]
    fn plan_player_count(&self) -> i64 {
        self.plan.as_ref().map_or(0, |p| p.players.len() as i64)
    }
}
