--[[
	state_test.luau — schema oracle for `src/shared/sim/State.luau` and
	`src/shared/sim/Input.luau` (SIM-SPEC §S1/§S3/§S4).

	WHERE THE EXPECTED VALUES COME FROM
	-----------------------------------
	Every number asserted below was PRINTED BY THE RUST, not derived here. A
	throwaway crate outside the repo linked `sim` as a path dependency and dumped
	`ArenaState::new(seed, player)` field by field, plus the results of
	`buy_modifier` / `grant_offer` / `record_player_damage` / `Tank::heal` /
	`Vec2::*` / `alloc_entity_id` / `enemy_index` (all `pub`). The five RNG
	stream states for `seed = 0xdeadbeefcafef00d, player = 0` additionally match
	`test/vectors/rng_vectors.json`'s Rust-emitted `derive` rows for purposes 0
	and 1, so the oracle is cross-checked against a checked-in artifact.

	`input::apply` is `pub(crate)`, so it could not be driven from outside the
	crate. The `Input.apply` cases below therefore mirror the assertions of the
	16 `input.rs` unit tests one-for-one (all green in `cargo test -p sim`), which
	are themselves the Rust's authoritative statement of that function's
	behavior. Where a case asserts something those tests do not (the
	`total_damage_dealt` credited by `Clear`), it is marked.

	HOW TO RUN
	----------
	The standalone `luau` interpreter has no file I/O; this test needs no input
	data, so it just runs:

	    luau roblox/test/state_test.luau

	It requires `../src/shared/sim/State` and `Input`, which in turn require
	`Content.luau`, `Modifiers.luau` and `Shop.luau`. Exits non-zero on failure.
]]

local State = require("../src/shared/sim/State")
local Input = require("../src/shared/sim/Input")
local Fixed = require("../src/shared/Fixed")
local Rng = require("../src/shared/Rng")
local Content = require("../src/shared/Content")

local failures = 0
local checks = 0
local group = "?"

local function fail(what, got, want)
	failures += 1
	print(string.format("  FAIL [%s] %s: got %s, want %s", group, what, tostring(got), tostring(want)))
end

local function eq(what, got, want)
	checks += 1
	if got ~= want then
		fail(what, got, want)
	end
end

--- Compare a `Fixed` against a raw i64 (as a plain number).
local function raw(what, got, want)
	checks += 1
	local g = Fixed.raw(got)
	if g ~= want then
		fail(what, g, want)
	end
end

--- Compare an `Rng` stream state against the Rust's 16-hex-digit rendering.
local function rng(what, r, want)
	checks += 1
	local got = Rng.toHex(Rng.state(r))
	if got ~= "0x" .. want then
		fail(what, got, "0x" .. want)
	end
end

local function G(name)
	group = name
end

--=============================================================================
-- 1. ArenaState::new — every field, against the Rust dump.
--=============================================================================

G("ArenaState::new(0x12345678, 0)")
local s = State.new(0x12345678, 0)

eq("tick", s.tick, 0)
eq("round", s.round, 4294967295) -- u32::MAX sentinel
eq("round == State.U32_MAX", s.round, State.U32_MAX)
eq("master_seed.hi", s.master_seed.hi, 0)
eq("master_seed.lo", s.master_seed.lo, 0x12345678)
eq("player_id", s.player_id, 0)
eq("next_entity_id", s.next_entity_id, 2) -- 1 was consumed by the free weapon
eq("dead", s.dead, false)
eq("death_tick", s.death_tick, nil)

eq("tank.hp", s.tank.hp, 24000)
eq("tank.max_hp", s.tank.max_hp, 24000)
raw("tank.pos.x", s.tank.pos.x, 0)
raw("tank.pos.y", s.tank.pos.y, 0)
eq("tank.clear_cooldown_end", s.tank.clear_cooldown_end, 0)
eq("tank.armor", s.tank.armor, 0)
eq("tank.dodge_num", s.tank.dodge_num, 0)
eq("tank.dodge_den", s.tank.dodge_den, 100)
eq("tank.mana_shield", s.tank.mana_shield, 0)
eq("tank.mana_shield_max", s.tank.mana_shield_max, 0)
eq("tank.mana_regen_per_tick", s.tank.mana_regen_per_tick, 0)
eq("tank.hp_regen_per_tick", s.tank.hp_regen_per_tick, 0)
eq("tank.spikes_damage", s.tank.spikes_damage, 0)
raw("tank.spikes_mult", s.tank.spikes_mult, 65536) -- Fixed::ONE
raw("tank.shield_active_dr", s.tank.shield_active_dr, 0)
eq("tank.heal_on_damaged", s.tank.heal_on_damaged, 0)
eq("tank.heal_on_kill", s.tank.heal_on_kill, 0)
eq("tank.mana_on_kill", s.tank.mana_on_kill, 0)
eq("tank.heal_on_poison", s.tank.heal_on_poison, 0)
raw("tank.healing_mult", s.tank.healing_mult, 65536)
raw("tank.missing_hp_heal_pct", s.tank.missing_hp_heal_pct, 0)
eq("tank.revives", s.tank.revives, 0)
eq("tank.revive_bonus_hp", s.tank.revive_bonus_hp, 0)
eq("tank.shieldbreak_stun_range", s.tank.shieldbreak_stun_range, 0)
eq("tank.shieldbreak_stun_ticks", s.tank.shieldbreak_stun_ticks, 0)
eq("tank.spikes_poison_dps", s.tank.spikes_poison_dps, 0)
eq("tank.spikes_poison_ticks", s.tank.spikes_poison_ticks, 0)
eq("tank.spikes_stack_per", s.tank.spikes_stack_per, 0)
eq("tank.spikes_stacks", s.tank.spikes_stacks, 0)
eq("tank.spikes_stacks_max", s.tank.spikes_stacks_max, 0)
eq("tank.aura_range", s.tank.aura_range, 0)
eq("tank.aura_cadence", s.tank.aura_cadence, 0)
eq("tank.aura_damage", s.tank.aura_damage, 0)
eq("tank.aura_poison_dps", s.tank.aura_poison_dps, 0)
eq("tank.aura_poison_ticks", s.tank.aura_poison_ticks, 0)
eq("tank.aura_tick", s.tank.aura_tick, 0)

eq("economy.gold", s.economy.gold, 500)
eq("economy.income_per_tick", s.economy.income_per_tick, 20)
raw("economy.income_mult", s.economy.income_mult, 65536)
raw("economy.income_regen_pct", s.economy.income_regen_pct, 0)
raw("economy.bounty_mult", s.economy.bounty_mult, 65536)
eq("economy.bounty_proc_chance_pct", s.economy.bounty_proc_chance_pct, 0)
raw("economy.bounty_proc_bonus", s.economy.bounty_proc_bonus, 0)
raw("economy.gold_per_damage", s.economy.gold_per_damage, 0)
raw("economy.income_shield_pct", s.economy.income_shield_pct, 0)
eq("economy.rerolls_remaining", s.economy.rerolls_remaining, 5)
eq("economy.reroll_cost", s.economy.reroll_cost, 100)

eq("shop.offers len", #s.shop.offers, 0)
eq("shop.shop_seq", s.shop.shop_seq, 0)

eq("weapons len", #s.weapons, 1)
eq("weapons[1].instance_id", s.weapons[1].instance_id, 1)
eq("weapons[1].def (STARTING_WEAPON)", s.weapons[1].def, 0)
eq("weapons[1].next_fire_tick", s.weapons[1].next_fire_tick, 0)

eq("enemies len", #s.enemies, 0)
eq("projectiles len", #s.projectiles, 0)
eq("hazards len", #s.hazards, 0)
eq("minions len", #s.minions, 0)
eq("ramps len", #s.ramps, 0)
eq("vuln_pulses len", #s.vuln_pulses, 0)
eq("pending_kills len", #s.pending_kills, 0)
eq("pending_perk", s.pending_perk, nil)

-- TRANSIENT (never checksummed; false at every tick boundary)
eq("tank_hit_this_tick", s.tank_hit_this_tick, false)
eq("shield_broke_this_tick", s.shield_broke_this_tick, false)

eq("total_damage_dealt", s.total_damage_dealt, 0)
eq("total_gold_earned", s.total_gold_earned, 0)

-- RENDER/PROFILE-ONLY (never checksummed)
eq("bought_attack_mask", s.bought_attack_mask, 0)
eq("weapons_bought", s.weapons_bought, 0)
eq("economy_purchases", s.economy_purchases, 0)

raw("modifiers.add_global", s.modifiers.add_global, 0)
eq("modifiers.add_by_type len", #s.modifiers.add_by_type, 5)
eq("modifiers.add_by_scope len", #s.modifiers.add_by_scope, 12)
raw("modifiers.mul_global", s.modifiers.mul_global, 65536)
raw("modifiers.attack_speed", s.modifiers.attack_speed, 0)
raw("modifiers.vs_stunned", s.modifiers.vs_stunned, 0)
raw("modifiers.vs_poisoned", s.modifiers.vs_poisoned, 0)
raw("modifiers.poison_dmg_mult", s.modifiers.poison_dmg_mult, 65536)
raw("modifiers.stun_dur_mult", s.modifiers.stun_dur_mult, 65536)
eq("modifiers.weapon_count_scaling len", #s.modifiers.weapon_count_scaling, 0)
raw("modifiers.dmg_per_maxhp_rate", s.modifiers.dmg_per_maxhp_rate, 0)
raw("modifiers.dmg_per_bounty_rate", s.modifiers.dmg_per_bounty_rate, 0)
raw("modifiers.shield_active_dmg", s.modifiers.shield_active_dmg, 0)

rng("rng_spawn", s.rng_spawn, "d544e8f2d9f73d7e")
rng("rng_targeting", s.rng_targeting, "ecbf365872ac29c0")
rng("rng_shop", s.rng_shop, "cbaefd7817dd2722")
rng("rng_reroll", s.rng_reroll, "3c7895b334fb5ece")
rng("rng_proc", s.rng_proc, "fc1c2a0aa394b27d")

--=============================================================================
-- 2. Stream derivation across seeds / players / seed input forms.
--=============================================================================

G("stream derivation")
-- 0xdeadbeefcafef00d cannot be a plain Luau number; both string forms must give
-- the same streams. Purposes 0 and 1 here also appear in rng_vectors.json.
for _, seed in { "0xdeadbeefcafef00d", "16045690984503111693" } do
	local a = State.new(seed, 0)
	rng("deadbeef rng_spawn", a.rng_spawn, "411d1fa3cdf5b0fd")
	rng("deadbeef rng_targeting", a.rng_targeting, "a7cd042ba510a0f3")
	rng("deadbeef rng_shop", a.rng_shop, "191a9ae3bb7f4a12")
	rng("deadbeef rng_reroll", a.rng_reroll, "03ac839e6a5cb90a")
	rng("deadbeef rng_proc", a.rng_proc, "74f9264e730071ae")
	eq("deadbeef master_seed.hi", a.master_seed.hi, 0xdeadbeef)
	eq("deadbeef master_seed.lo", a.master_seed.lo, 0xcafef00d)
end

local b = State.new(12345, 0)
rng("12345 rng_spawn", b.rng_spawn, "a9fb86aff8ee1b6d")
rng("12345 rng_targeting", b.rng_targeting, "2be4fc466b333702")
rng("12345 rng_shop", b.rng_shop, "b9cd968b10cce2dd")
rng("12345 rng_reroll", b.rng_reroll, "c6e123102d11ee41")
rng("12345 rng_proc", b.rng_proc, "3c76620c08ae8cb7")

-- player_id is part of the derivation, so a different player is a different sim.
local c = State.new(0, 1)
rng("seed0/player1 rng_spawn", c.rng_spawn, "80abe802ac1e182e")
rng("seed0/player1 rng_targeting", c.rng_targeting, "ee7c8c5d62a26893")
rng("seed0/player1 rng_shop", c.rng_shop, "7fbc92dbdb3313e0")
rng("seed0/player1 rng_reroll", c.rng_reroll, "28882038ead83f26")
rng("seed0/player1 rng_proc", c.rng_proc, "c09856ed8ed7a4ad")

eq("Purpose.Spawn", State.Purpose.Spawn, 0)
eq("Purpose.Targeting", State.Purpose.Targeting, 1)
eq("Purpose.Shop", State.Purpose.Shop, 2)
eq("Purpose.Reroll", State.Purpose.Reroll, 3)
eq("Purpose.Proc", State.Purpose.Proc, 4)

--=============================================================================
-- 3. Vec2 helpers.
--=============================================================================

G("Vec2")
local Vec2 = State.Vec2
local origin = Vec2.new(Fixed.fromInt(0), Fixed.fromInt(0))
local far = Vec2.new(Fixed.fromInt(300), Fixed.fromInt(400))

raw("dist_sq((0,0),(300,400))", Vec2.distSq(origin, far), 16384000000)

local st = Vec2.stepToward(origin, far, Fixed.fromInt(100))
raw("step_toward(100).x", st.x, 3932160) -- 60 << 16
raw("step_toward(100).y", st.y, 5242880) -- 80 << 16

local clamped = Vec2.stepToward(origin, far, Fixed.fromInt(5000))
raw("step_toward(overshoot).x", clamped.x, 19660800) -- 300 << 16
raw("step_toward(overshoot).y", clamped.y, 26214400) -- 400 << 16
checks += 1
if clamped == far then
	fail("step_toward must return a FRESH Vec2, not alias the target", "aliased", "copy")
end

local away = Vec2.stepAway(far, origin, Fixed.fromInt(50))
raw("step_away(50).x", away.x, 21626880) -- 330 << 16
raw("step_away(50).y", away.y, 28835840) -- 440 << 16

local degenerate = Vec2.stepAway(origin, origin, Fixed.fromInt(50))
raw("step_away degenerate .x", degenerate.x, 0)
raw("step_away degenerate .y", degenerate.y, 0)

raw("Vec2.ZERO.x", Vec2.ZERO.x, 0)
raw("Vec2.ZERO.y", Vec2.ZERO.y, 0)
checks += 1
if not table.isfrozen(Vec2.ZERO) then
	fail("Vec2.ZERO must be frozen (Rust's Vec2 is Copy; sharing it would alias)", false, true)
end

--=============================================================================
-- 4. Tank::heal / Tank::restore_mana.
--=============================================================================

G("Tank")
local t = State.new(0x12345678, 0)
t.tank.hp = 100
State.Tank.heal(t.tank, 50)
eq("heal 50 at x1.0", t.tank.hp, 150)
t.tank.hp = 23990
State.Tank.heal(t.tank, 50)
eq("heal caps at max_hp", t.tank.hp, 24000)
t.tank.hp = 100
State.Tank.heal(t.tank, 0)
eq("heal 0 is a no-op", t.tank.hp, 100)
State.Tank.heal(t.tank, -5)
eq("heal negative is a no-op", t.tank.hp, 100)
t.tank.healing_mult = Fixed.fromRatio(3, 2)
State.Tank.heal(t.tank, 51)
eq("heal 51 at x1.5 floors to +76", t.tank.hp, 176)

State.Tank.restoreMana(t.tank, 10)
eq("restore_mana with no pool", t.tank.mana_shield, 0)
t.tank.mana_shield_max = 100
State.Tank.restoreMana(t.tank, 30)
State.Tank.restoreMana(t.tank, 500)
eq("restore_mana caps at the pool max", t.tank.mana_shield, 100)

--=============================================================================
-- 5. alloc_entity_id / enemy_index.
--=============================================================================

G("ids")
local e = State.new(0x12345678, 0)
eq("alloc_entity_id #1", State.allocEntityId(e), 2)
eq("alloc_entity_id #2", State.allocEntityId(e), 3)
eq("next_entity_id after two allocs", e.next_entity_id, 4)

e.enemies = {
	State.Enemy.new(10, 0, 200, State.Vec2.ZERO),
	State.Enemy.new(11, 1, 1200, State.Vec2.ZERO),
}
-- NOTE: the Rust returns a 0-based `usize` (Some(0)/Some(1)/None); the Luau
-- returns the 1-BASED array index so the result indexes `s.enemies` directly.
eq("enemy_index(10)", State.enemyIndex(e, 10), 1)
eq("enemy_index(11)", State.enemyIndex(e, 11), 2)
eq("enemy_index(12)", State.enemyIndex(e, 12), nil)
eq("Enemy.new status.poison_dps", e.enemies[1].status.poison_dps, 0)
eq("Enemy.new status.freeze_ticks", e.enemies[1].status.freeze_ticks, 0)

--=============================================================================
-- 6. award_gold / record_player_damage.
--=============================================================================

G("scoreboard")
local g = State.new(0x12345678, 0)
State.awardGold(g, 0)
eq("award_gold(0) is a no-op", g.total_gold_earned, 0)
State.recordPlayerDamage(g, 1000)
eq("record_player_damage total", g.total_damage_dealt, 1000)
eq("record_player_damage: no bloodmoney, gold unchanged", g.economy.gold, 500)
eq("record_player_damage: no bloodmoney, total_gold 0", g.total_gold_earned, 0)
State.recordPlayerDamage(g, 0)
eq("record_player_damage(0) is a no-op", g.total_damage_dealt, 1000)

-- MODIFIERS[53] is Bloodmoney: GoldPerDamagePct(1, 60) -> gold_per_damage raw 1092.
local bm = State.new(0x12345678, 0)
State.buyModifier(bm, 53)
raw("bloodmoney gold_per_damage", bm.economy.gold_per_damage, 1092)
State.recordPlayerDamage(bm, 1000)
eq("bloodmoney total_damage", bm.total_damage_dealt, 1000)
eq("bloodmoney gold", bm.economy.gold, 516)
eq("bloodmoney total_gold_earned", bm.total_gold_earned, 16)

--=============================================================================
-- 7. buy_modifier — the six effects state.rs intercepts, plus the ramp tail.
--=============================================================================

G("buy_modifier")
-- MODIFIERS[37] "Vulnerability Totem": GrantVulnPulse(5, 1200, 30).
local vp = State.new(0x12345678, 0)
vp.tick = 100
State.buyModifier(vp, 37)
eq("vuln_pulses len", #vp.vuln_pulses, 1)
eq("vuln_pulse.magnitude", vp.vuln_pulses[1].magnitude, 5)
eq("vuln_pulse.range", vp.vuln_pulses[1].range, 1200)
eq("vuln_pulse.interval_ticks", vp.vuln_pulses[1].interval_ticks, 30)
eq("vuln_pulse.next_tick", vp.vuln_pulses[1].next_tick, 130)

-- MODIFIERS[16] "Power Generator": ramp DamageGlobalPct(1,100) every 900 ticks.
local rp = State.new(0x12345678, 0)
rp.tick = 100
State.buyModifier(rp, 16)
eq("ramps len", #rp.ramps, 1)
eq("ramp.effect.tag", rp.ramps[1].effect.tag, 0)
eq("ramp.effect.a", rp.ramps[1].effect.a, 1)
eq("ramp.effect.b", rp.ramps[1].effect.b, 100)
eq("ramp.interval_ticks", rp.ramps[1].interval_ticks, 900)
eq("ramp.next_apply", rp.ramps[1].next_apply, 1000)

-- MODIFIERS[44] "Magic Treasure": GrantGold(250), plus an IncomeFlat(5) ramp.
local mt = State.new(0x12345678, 0)
State.buyModifier(mt, 44)
eq("grant gold: gold", mt.economy.gold, 750)
eq("grant gold: total_gold_earned", mt.total_gold_earned, 250)
eq("grant gold: ramp registered", #mt.ramps, 1)
eq("grant gold: ramp next_apply", mt.ramps[1].next_apply, 900)

-- MODIFIERS[51] "Philosopher's Stone": TradeMaxHpForGold(1000, 2000).
local th = State.new(0x12345678, 0)
State.buyModifier(th, 51)
eq("trade max hp: max_hp", th.tank.max_hp, 23000)
eq("trade max hp: hp clamped", th.tank.hp, 23000)
eq("trade max hp: gold", th.economy.gold, 2500)
eq("trade max hp: total_gold_earned", th.total_gold_earned, 2000)

-- MODIFIERS[52] "Cursed Treasure": TradeRegenForGold(100, 5000) — may go negative.
local tr = State.new(0x12345678, 0)
State.buyModifier(tr, 52)
eq("trade regen: hp_regen_per_tick goes negative", tr.tank.hp_regen_per_tick, -100)
eq("trade regen: gold", tr.economy.gold, 5500)
eq("trade regen: total_gold_earned", tr.total_gold_earned, 5000)

-- MODIFIERS[41] "Multiplication Gems": GrantDuplicator(rarity 0, 3 copies).
local dp = State.new(0x12345678, 0)
State.buyModifier(dp, 41)
eq("duplicator perk.rarity", dp.pending_perk.rarity, 0)
eq("duplicator perk.extra_copies", dp.pending_perk.extra_copies, 3)
eq("duplicator perk.free", dp.pending_perk.free, false)

-- MODIFIERS[43] "Black Market": GrantVoucher(rarity 1).
local vc = State.new(0x12345678, 0)
State.buyModifier(vc, 43)
eq("voucher perk.rarity", vc.pending_perk.rarity, 1)
eq("voucher perk.extra_copies", vc.pending_perk.extra_copies, 0)
eq("voucher perk.free", vc.pending_perk.free, true)

G("PendingPerk::matches")
eq("matches same rarity", State.PendingPerk.matches({ rarity = 2, extra_copies = 0, free = false }, 2), true)
eq("matches other rarity", State.PendingPerk.matches({ rarity = 2, extra_copies = 0, free = false }, 1), false)
eq("255 matches anything", State.PendingPerk.matches({ rarity = 255, extra_copies = 0, free = false }, 3), true)

--=============================================================================
-- 8. grant_offer / offer_rarity / offer_is_meta.
--=============================================================================

G("grant_offer")
local OK = State.OfferKind
eq("OfferKind.Weapon", OK.Weapon, 0)
eq("OfferKind.Modifier", OK.Modifier, 1)

local go = State.new(0x12345678, 0)
State.grantOffer(go, { kind = OK.Weapon, def = 0, cost = 100 })
eq("weapons len", #go.weapons, 2)
eq("granted instance_id", go.weapons[2].instance_id, 2)
eq("granted def", go.weapons[2].def, 0)
eq("granted next_fire_tick", go.weapons[2].next_fire_tick, 0)
eq("bought_attack_mask (SingleTarget -> bit 0)", go.bought_attack_mask, 1)
eq("weapons_bought", go.weapons_bought, 1)
eq("next_entity_id", go.next_entity_id, 3)

-- MODIFIERS[7] "Entangled Gold Mine" — the first economy modifier.
local ge = State.new(0x12345678, 0)
State.grantOffer(ge, { kind = OK.Modifier, def = 7, cost = 0 })
eq("economy_purchases", ge.economy_purchases, 1)
eq("IncomeFlat(20) reached Modifiers.applyEffect", ge.economy.income_per_tick, 40)

eq("offer_rarity(weapon 0)", State.offerRarity({ kind = OK.Weapon, def = 0, cost = 0 }), 0)
eq("offer_rarity(modifier 41)", State.offerRarity({ kind = OK.Modifier, def = 41, cost = 0 }), 1)
eq("offer_is_meta(weapon 0)", State.offerIsMeta({ kind = OK.Weapon, def = 0, cost = 0 }), false)
eq("offer_is_meta(modifier 41 dup)", State.offerIsMeta({ kind = OK.Modifier, def = 41, cost = 0 }), true)
eq("offer_is_meta(modifier 43 voucher)", State.offerIsMeta({ kind = OK.Modifier, def = 43, cost = 0 }), true)
eq("offer_is_meta(modifier 44 gold)", State.offerIsMeta({ kind = OK.Modifier, def = 44, cost = 0 }), true)
eq("offer_is_meta(modifier 51 trade hp)", State.offerIsMeta({ kind = OK.Modifier, def = 51, cost = 0 }), true)
eq("offer_is_meta(modifier 52 trade regen)", State.offerIsMeta({ kind = OK.Modifier, def = 52, cost = 0 }), true)
eq("offer_is_meta(modifier 37 vuln)", State.offerIsMeta({ kind = OK.Modifier, def = 37, cost = 0 }), false)

--=============================================================================
-- 9. The input code encoding (SIM-SPEC §S6 / net::wire::InputCode tags).
--=============================================================================

G("input encoding")
eq("NOOP", Input.NOOP, 0)
-- Encoding is `kind * 256 + slot` (kind in the HIGH byte), matching
-- `net::wire::InputCode`'s byte order and the `code` field in the R3 trace
-- corpus. SIM-SPEC S6 originally left this unpinned and two agents chose
-- opposite conventions; the corpus won, and this is now the single definition.
eq("REROLL", Input.REROLL, 2 * 256)
eq("CLEAR", Input.CLEAR, 3 * 256)
eq("buyOffer(0)", Input.buyOffer(0), 256)
eq("buyOffer(1)", Input.buyOffer(1), 257)
eq("buyOffer(7)", Input.buyOffer(7), 263)
eq("buyOffer(255)", Input.buyOffer(255), 511)
eq("kind(NOOP)", Input.kind(Input.NOOP), Input.Kind.Noop)
eq("kind(REROLL)", Input.kind(Input.REROLL), Input.Kind.Reroll)
eq("kind(CLEAR)", Input.kind(Input.CLEAR), Input.Kind.Clear)
for slot = 0, 255 do
	local code = Input.buyOffer(slot)
	if Input.kind(code) ~= Input.Kind.BuyOffer or Input.slot(code) ~= slot then
		fail("buyOffer round-trip", slot, "kind/slot round-trip")
	end
	checks += 1
end

--=============================================================================
-- 10. Input.apply — mirrors the 16 `input.rs` unit tests.
--=============================================================================

local function fresh()
	return State.new(0x12345678, 0)
end

local function mkEnemy(id, def, hp)
	return State.Enemy.new(id, def, hp, State.Vec2.new(Fixed.ZERO, Fixed.ZERO))
end

G("input.rs::noop_changes_nothing")
local n = fresh()
Input.apply(n, Input.NOOP)
eq("gold", n.economy.gold, 500)
eq("weapons", #n.weapons, 1)
eq("next_entity_id", n.next_entity_id, 2)
eq("shop_seq", n.shop.shop_seq, 0)
rng("rng_shop untouched", n.rng_shop, "cbaefd7817dd2722")

G("input.rs::buy_offer_deducts_gold_and_adds_weapon_when_affordable")
local b1 = fresh()
b1.shop.offers = { { kind = OK.Weapon, def = 1, cost = 200 } }
b1.economy.gold = 500
Input.apply(b1, Input.buyOffer(0))
eq("gold", b1.economy.gold, 300)
eq("weapons", #b1.weapons, 2)
eq("bought def", b1.weapons[2].def, 1)
eq("next_fire_tick == s.tick", b1.weapons[2].next_fire_tick, b1.tick)

G("input.rs::buy_offer_ignored_when_unaffordable")
local b2 = fresh()
b2.shop.offers = { { kind = OK.Weapon, def = 1, cost = 600 } }
b2.economy.gold = 500
Input.apply(b2, Input.buyOffer(0))
eq("gold", b2.economy.gold, 500)
eq("weapons", #b2.weapons, 1)

G("input.rs::buy_offer_ignored_for_invalid_slot")
local b3 = fresh()
b3.shop.offers = { { kind = OK.Weapon, def = 0, cost = 0 } }
b3.economy.gold = 500
Input.apply(b3, Input.buyOffer(5))
eq("gold", b3.economy.gold, 500)
eq("weapons", #b3.weapons, 1)

G("input.rs::reroll_consumes_free_reroll_first")
local r1 = fresh()
r1.economy.rerolls_remaining = 2
r1.economy.gold = 1000
Input.apply(r1, Input.REROLL)
eq("rerolls_remaining", r1.economy.rerolls_remaining, 1)
eq("free reroll must not charge gold", r1.economy.gold, 1000)
eq("cost unchanged on free reroll", r1.economy.reroll_cost, 100)
eq("offers", #r1.shop.offers, 8)

G("input.rs::reroll_charges_escalating_gold_when_no_free_rerolls")
local r2 = fresh()
r2.economy.rerolls_remaining = 0
r2.economy.gold = 1000
r2.economy.reroll_cost = 100
Input.apply(r2, Input.REROLL)
eq("gold after first paid reroll", r2.economy.gold, 900)
eq("cost after first paid reroll", r2.economy.reroll_cost, 200)
Input.apply(r2, Input.REROLL)
eq("gold after second paid reroll", r2.economy.gold, 700)
eq("cost after second paid reroll", r2.economy.reroll_cost, 300)

G("input.rs::reroll_ignored_when_broke_and_no_free")
local r3 = fresh()
r3.economy.rerolls_remaining = 0
r3.economy.gold = 50
r3.economy.reroll_cost = 100
r3.shop.offers = { { kind = OK.Weapon, def = 7, cost = 1 } }
Input.apply(r3, Input.REROLL)
eq("gold", r3.economy.gold, 50)
eq("no regeneration when ignored", r3.shop.shop_seq, 0)
eq("offers untouched", r3.shop.offers[1].def, 7)
-- §S4: an ignored reroll must not consume an rng_shop word.
rng("rng_shop untouched", r3.rng_shop, "cbaefd7817dd2722")

G("input.rs::duplicator_grants_extra_copies_of_next_matching_purchase")
local d1 = fresh()
State.buyModifier(d1, 41) -- GrantDuplicator(rarity 0, 3 copies)
eq("perk armed", d1.pending_perk ~= nil, true)
d1.shop.offers = { { kind = OK.Weapon, def = 0, cost = 100 } } -- WEAPONS[0] is rarity 0
d1.economy.gold = 100
local before = #d1.weapons
Input.apply(d1, Input.buyOffer(0))
eq("duplicated copies granted", #d1.weapons, before + 1 + 3)
eq("perk consumed", d1.pending_perk, nil)
eq("only the base copy costs gold", d1.economy.gold, 0)

G("input.rs::duplicator_ignores_non_matching_rarity")
local d2 = fresh()
State.buyModifier(d2, 41)
d2.shop.offers = { { kind = OK.Weapon, def = 5, cost = 0 } } -- WEAPONS[5] is rarity 2
local before2 = #d2.weapons
Input.apply(d2, Input.buyOffer(0))
eq("no extra copies for wrong rarity", #d2.weapons, before2 + 1)
eq("perk still armed", d2.pending_perk ~= nil, true)

G("input.rs::voucher_makes_next_matching_purchase_free")
local v1 = fresh()
State.buyModifier(v1, 43) -- GrantVoucher(rarity 1)
v1.shop.offers = { { kind = OK.Weapon, def = 2, cost = 9999 } } -- WEAPONS[2] is rarity 1
v1.economy.gold = 0
local before3 = #v1.weapons
Input.apply(v1, Input.buyOffer(0))
eq("free purchase happened", #v1.weapons, before3 + 1)
eq("voucher charged no gold", v1.economy.gold, 0)
eq("voucher consumed", v1.pending_perk, nil)

G("input.rs::meta_items_do_not_consume_or_chain_perks")
local m1 = fresh()
State.buyModifier(m1, 41) -- duplicator armed
m1.shop.offers = { { kind = OK.Modifier, def = 43, cost = 0 } } -- another META item
m1.economy.gold = 0
Input.apply(m1, Input.buyOffer(0))
eq("perk is now the voucher", m1.pending_perk.free, true)
eq("voucher rarity", m1.pending_perk.rarity, 1)

G("input.rs::clear_kills_enemies_and_records_bounties")
local c1 = fresh()
c1.tick = 0
c1.tank.clear_cooldown_end = 0
c1.enemies = { mkEnemy(10, 0, 200), mkEnemy(11, 1, 1200) }
Input.apply(c1, Input.CLEAR)
eq("all enemies wiped", #c1.enemies, 0)
eq("pending_kills len", #c1.pending_kills, 2)
eq("pending_kills[1]", c1.pending_kills[1], 0)
eq("pending_kills[2]", c1.pending_kills[2], 1)
eq("cooldown set", c1.tank.clear_cooldown_end, 300)
-- NOT asserted by the Rust test: `cleared_damage` is the CLAMPED HP removed
-- (200 + 1200), per the `before - e.hp.max(0)` line in `input.rs`.
eq("total_damage_dealt is clamped HP removed", c1.total_damage_dealt, 1400)

G("input.rs::clear_respects_cooldown")
local c2 = fresh()
c2.tick = 100
c2.tank.clear_cooldown_end = 300
c2.enemies = { mkEnemy(10, 0, 200) }
Input.apply(c2, Input.CLEAR)
eq("clear must not fire on cooldown", #c2.enemies, 1)
eq("no kills", #c2.pending_kills, 0)
eq("cooldown unchanged", c2.tank.clear_cooldown_end, 300)

G("input.rs::clear_fires_exactly_at_cooldown_end")
local c3 = fresh()
c3.tick = 300
c3.tank.clear_cooldown_end = 300
c3.enemies = { mkEnemy(10, 0, 200) }
Input.apply(c3, Input.CLEAR)
eq("fires at the boundary", #c3.enemies, 0)
eq("cooldown re-armed", c3.tank.clear_cooldown_end, 600)

G("input.rs::clear_chips_the_boss_over_several_uses")
local BOSS = 2
-- Sourced from the artifact rather than hardcoded: the fun pass moved boss HP
-- 33M -> 6300000 and gave the boss its own Clear chunk (`BOSS_CLEAR_DAMAGE`),
-- separate from the 3M every other enemy takes. Hardcoding either meant this
-- block asserted against a boss that no longer exists.
local BOSS_HP = Content.ENEMIES[BOSS + 1].base_hp
local CLEAR_DAMAGE = Content.BOSS_CLEAR_DAMAGE or Content.constants.boss_clear_damage
local c4 = fresh()
c4.tick = 0
c4.tank.clear_cooldown_end = 0
c4.enemies = { mkEnemy(10, BOSS, BOSS_HP) }
Input.apply(c4, Input.CLEAR)
eq("boss survives a single Clear", #c4.enemies, 1)
eq("boss hp after one Clear", c4.enemies[1].hp, BOSS_HP - CLEAR_DAMAGE)
local needed = math.floor((BOSS_HP + CLEAR_DAMAGE - 1) / CLEAR_DAMAGE) -- ceil
for _ = 2, needed do
	c4.tick = c4.tank.clear_cooldown_end -- come off cooldown
	Input.apply(c4, Input.CLEAR)
end
eq("boss dies after enough Clears", #c4.enemies, 0)
eq("boss def recorded in pending_kills", c4.pending_kills[#c4.pending_kills], BOSS)

G("input: unknown code")
checks += 1
-- kind 4 (= 4*256) is not an `ids::Input` variant. Under the corpus encoding
-- the kind lives in the HIGH byte, so a small integer like 9 decodes to
-- kind 0 (Noop) with slot 9 -- valid, not unknown.
if pcall(Input.apply, fresh(), 4 * 256) then
	fail("an unknown input kind must error", "ok", "error")
end

--=============================================================================

print(string.format("state_test: %d checks, %d failures", checks, failures))
if failures > 0 then
	error(string.format("state_test FAILED (%d)", failures), 0)
end
print("state_test: OK")
