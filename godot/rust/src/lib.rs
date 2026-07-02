//! GDExtension binding: exposes the deterministic `sim` to Godot as a single
//! `StSim` node. It owns an `ArenaState`, advances it one tick per call, and
//! hands the engine flat `Packed*Array`s built from `sim::view` (the
//! engine-agnostic render contract). No game logic lives here — only marshaling.
//!
//! GDScript surface (see ../main.gd):
//!   var sim = StSim.new_match(seed)
//!   sim.step(code, slot)          # 0 Noop · 1 Buy(slot) · 2 Reroll · 3 Clear
//!   sim.tick(); sim.round(); sim.is_dead()
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
    /// Start a fresh match with the given RNG seed.
    #[func]
    fn new_match(seed: i64) -> Gd<StSim> {
        let state = ArenaState::new(seed as u64, 0);
        let view = view::snapshot(&state);
        Gd::from_init_fn(|base| StSim {
            state,
            view,
            events: Vec::new(),
            base,
        })
    }

    /// Advance exactly one 30 Hz tick with the player's action this tick.
    #[func]
    fn step(&mut self, input_code: i64, slot: i64) {
        let inp = match input_code {
            1 => Input::BuyOffer { slot: slot as u8 },
            2 => Input::Reroll,
            3 => Input::Clear,
            _ => Input::Noop,
        };
        sim::step(&mut self.state, inp);
        self.view = view::snapshot(&self.state);
        self.events = self.state.events.take();
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
}

// ===================== Multi-arena / net view =====================

/// Arbitrary content hash for the demo clients (the M2 director doesn't gate it).
const DEMO_CONTENT_HASH: u64 = 0xC0DE_C0DE;

/// A full N-player match running the REAL netcode loop — authoritative
/// [`Director`] + per-player [`Client`]s wired through the deterministic [`Hub`],
/// each client driven by the shared [`Bot`]. It renders every player's
/// authoritative shadow arena, showcasing the sharded-simulation architecture:
/// N independent arenas advancing under one director, no entity replication.
#[derive(GodotClass)]
#[class(no_init, base = RefCounted)]
pub struct StMatch {
    director: Director,
    clients: Vec<Client>,
    bots: Vec<Bot>,
    hub: Hub,
    peers: Vec<PeerId>,
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
    /// Start an `n`-player match (clamped 1..=16) seeded with `seed`.
    #[func]
    fn new_match(n: i64, seed: i64) -> Gd<StMatch> {
        let n = n.clamp(1, 16) as u32;
        let peers: Vec<PeerId> = (1..=n).map(PeerId).collect();
        let director = Director::new(&peers, seed as u64);
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
            views,
            events,
            base,
        })
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
            let desired = match self.director.shadow(*p) {
                Some(sh) => self.bots[i].decide(sh),
                None => Input::Noop,
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

    /// `[gold, income_per_tick]` for player `i`.
    #[func]
    fn economy(&self, i: i64) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        if let Some(v) = self.snap(i) {
            a.push(v.economy.gold);
            a.push(v.economy.income_per_tick);
        }
        a
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
