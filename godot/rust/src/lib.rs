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
}
