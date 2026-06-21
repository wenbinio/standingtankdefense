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
//!   sim.enemies_pos() / enemies_boss() / enemies_hp_permille()
//!   sim.projectiles_pos()
//!   sim.shop_names() / shop_meta()  # meta: [cost,flags, cost,flags, …]
//!   sim.arsenal_lines()

use godot::prelude::*;
use net::client::Client;
use net::director::Director;
use net::hub::Hub;
use net::transport::{PeerId, DIRECTOR};
use sim::bot::Bot;
use sim::view;
use sim::{ArenaState, Input};

struct StandingTankExt;

#[gdextension]
unsafe impl ExtensionLibrary for StandingTankExt {}

#[derive(GodotClass)]
#[class(no_init, base = RefCounted)]
pub struct StSim {
    state: ArenaState,
    base: Base<RefCounted>,
}

#[godot_api]
impl StSim {
    /// Start a fresh match with the given RNG seed.
    #[func]
    fn new_match(seed: i64) -> Gd<StSim> {
        Gd::from_init_fn(|base| StSim {
            state: ArenaState::new(seed as u64, 0),
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
        let t = view::snapshot(&self.state).tank;
        let mut a = PackedInt64Array::new();
        for v in [t.x, t.y, t.hp, t.max_hp, t.revives as i64] {
            a.push(v);
        }
        a
    }

    /// `[gold, income_per_tick, rerolls_remaining, reroll_cost]`.
    #[func]
    fn economy(&self) -> PackedInt64Array {
        let e = view::snapshot(&self.state).economy;
        let mut a = PackedInt64Array::new();
        for v in [e.gold, e.income_per_tick, e.rerolls_remaining as i64, e.reroll_cost] {
            a.push(v);
        }
        a
    }

    /// World positions of every enemy (units); parallel to the flag arrays below.
    #[func]
    fn enemies_pos(&self) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        for e in &view::snapshot(&self.state).enemies {
            a.push(Vector2::new(e.x as f32, e.y as f32));
        }
        a
    }

    /// Per-enemy boss flag (1 boss, 0 normal).
    #[func]
    fn enemies_boss(&self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for e in &view::snapshot(&self.state).enemies {
            a.push(e.boss as u8);
        }
        a
    }

    /// Per-enemy catalog kind index (0 grunt · 1 steam tank · 2 samwise),
    /// parallel to `enemies_pos()` — selects the sprite.
    #[func]
    fn enemies_kind(&self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for e in &view::snapshot(&self.state).enemies {
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
        for e in &view::snapshot(&self.state).enemies {
            a.push(e.id as i64);
        }
        a
    }

    /// Per-enemy HP as a permille of catalog base HP (for a health bar).
    #[func]
    fn enemies_hp_permille(&self) -> PackedInt32Array {
        let mut a = PackedInt32Array::new();
        for e in &view::snapshot(&self.state).enemies {
            let r = if e.base_hp > 0 {
                (e.hp.max(0) * 1000 / e.base_hp) as i32
            } else {
                0
            };
            a.push(r);
        }
        a
    }

    /// World positions of in-flight projectiles.
    #[func]
    fn projectiles_pos(&self) -> PackedVector2Array {
        let mut a = PackedVector2Array::new();
        for p in &view::snapshot(&self.state).projectiles {
            a.push(Vector2::new(p.x as f32, p.y as f32));
        }
        a
    }

    /// Names of the current shop offers (slot order).
    #[func]
    fn shop_names(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for o in &view::snapshot(&self.state).shop {
            a.push(&GString::from(o.name));
        }
        a
    }

    /// Flat `[cost, flags, rarity, …]` per offer; flags bit0 = is_weapon,
    /// bit1 = affordable; rarity 0 common · 1 uncommon · 2 rare · 3 epic.
    #[func]
    fn shop_meta(&self) -> PackedInt64Array {
        let mut a = PackedInt64Array::new();
        for o in &view::snapshot(&self.state).shop {
            a.push(o.cost);
            a.push((o.is_weapon as i64) | ((o.affordable as i64) << 1));
            a.push(o.rarity as i64);
        }
        a
    }

    /// `"Name xN"` lines for the owned arsenal.
    #[func]
    fn arsenal_lines(&self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        for e in &view::snapshot(&self.state).arsenal {
            a.push(&GString::from(format!("{} x{}", e.name, e.count).as_str()));
        }
        a
    }

    /// `[damage_dealt, gold_earned]` — match scoreboard totals.
    #[func]
    fn stats(&self) -> PackedInt64Array {
        let v = view::snapshot(&self.state);
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
        let clients = peers.iter().map(|p| Client::new(*p, DEMO_CONTENT_HASH)).collect();
        let bots = peers.iter().map(|_| Bot::default()).collect();
        Gd::from_init_fn(|base| StMatch {
            director,
            clients,
            bots,
            hub: Hub::new(),
            peers,
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
        self.peers.iter().filter(|p| self.director.is_alive(**p)).count() as i64
    }
    #[func]
    fn is_alive(&self, i: i64) -> bool {
        self.peers.get(i as usize).is_some_and(|p| self.director.is_alive(*p))
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

    fn snap(&self, i: i64) -> Option<view::RenderView> {
        self.peers
            .get(i as usize)
            .and_then(|p| self.director.shadow(*p))
            .map(view::snapshot)
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
