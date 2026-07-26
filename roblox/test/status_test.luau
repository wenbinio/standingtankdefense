--[[
	status_test.luau — unit tests for `src/shared/sim/Status.luau` and
	`src/shared/sim/Defense.luau`.

	These are transcriptions of the `#[cfg(test)]` modules in `status.rs` and
	`defense.rs`, plus a few extra cases for the transcription hazards called out
	in SIM-SPEC §S4 (fixed-point flooring, transient-flag lifecycle, RNG stream
	discipline). The Rust tests are the oracle: where a case exists there, the
	expected value here is copied from it rather than re-derived.

	RUN (from `roblox/test`):

	    luau status_test.luau

	Exits non-zero on any failure. Takes no arguments and does no file I/O, so it
	runs under the standalone interpreter unchanged.
]]

local Fixed = require("../src/shared/Fixed")
local Rng = require("../src/shared/Rng")
local Content = require("../src/shared/Content")
local State = require("../src/shared/sim/State")
local Status = require("../src/shared/sim/Status")
local Defense = require("../src/shared/sim/Defense")

-- ============================ harness ============================

local passed, failed = 0, 0
local current = "?"

local function check(ok: boolean, msg: string)
	if ok then
		passed += 1
	else
		failed += 1
		print("FAIL [" .. current .. "] " .. msg)
	end
end

local function eq(got, want, msg: string)
	check(got == want, msg .. " (got " .. tostring(got) .. ", want " .. tostring(want) .. ")")
end

local function test(name: string, fn: () -> ())
	current = name
	local ok, err = pcall(fn)
	if not ok then
		failed += 1
		print("ERROR [" .. name .. "] " .. tostring(err))
	end
end

-- ============================ fixtures ============================

local ZERO = Fixed.ZERO

-- INTEGRATION NOTE (mirrors the same lookup in `Status.luau`): SIM-SPEC §S2 does
-- not pin how `Content.luau` spells its scalar constants. Load-time only.
local function constant(topLevel: string, inConstants: string): number
	local v = (Content :: any)[topLevel]
	if v == nil and (Content :: any).constants ~= nil then
		v = (Content :: any).constants[inConstants]
	end
	if v == nil then
		error("status_test: Content is missing " .. topLevel .. " / constants." .. inConstants)
	end
	return v
end

local FROST_MAX_STACKS = constant("FROST_MAX_STACKS", "frost_max_stacks")
local BOSS = constant("BOSS", "boss")

local function vec(x: number)
	return State.Vec2.new(Fixed.fromInt(x), ZERO)
end

local function enemyAt(id: number, def: number, hp: number, x: number)
	return State.Enemy.new(id, def, hp, vec(x))
end

--- `status.rs::tests::enemy` — a single enemy at the origin.
local function enemy(def: number, hp: number)
	return State.Enemy.new(1, def, hp, State.Vec2.ZERO)
end

--- `status.rs::tests::arena_with`.
local function arenaWith(enemies)
	local s = State.new(1, 0)
	s.enemies = enemies
	return s
end

--- `defense.rs::tests::fresh`.
local function fresh()
	return State.new(0xD3F, 0)
end

--- `StatusOnHit { .. , ..StatusOnHit::NONE }`.
local function onHit(t)
	return {
		poison_dps = t.poison_dps or 0,
		poison_ticks = t.poison_ticks or 0,
		frost_stacks = t.frost_stacks or 0,
		fire_stacks = t.fire_stacks or 0,
		stun_ticks = t.stun_ticks or 0,
	}
end

--- `Fixed::scale_i64` down to a plain number, for the ×1000 assertions the Rust
--- tests use.
local function scale(f, v: number): number
	return Fixed.toInt(Fixed.scaleI64(f, v))
end

-- ============================ Status: frost ============================

test("frost_slows_below_cap_then_expires", function()
	local e = enemy(0, 1000)
	-- Stay strictly below the cap so no freeze triggers (24 stacks).
	Status.applyOnHit(e, onHit({ frost_stacks = 24 }))
	eq(e.status.frost_stacks, 24, "frost stacks accumulate below cap")
	eq(e.status.freeze_ticks, 0, "no freeze below cap")
	-- 24 stacks × 2% = 48% slow → ×0.52.
	eq(scale(Status.moveSpeedMult(e), 1000), 520, "×0.52 move speed")

	local s = arenaWith({ e })
	s.enemies[1].status.frost_ticks = 1
	Status.tick(s)
	eq(s.enemies[1].status.frost_stacks, 0, "stacks clear on expiry")
end)

test("frost_stacks_accumulate_one_hit_at_a_time_to_the_cap", function()
	-- 12 applications of 2 stacks: 2,4,…,24 with no freeze, then the 13th
	-- reaches 26 ≥ 25 and freezes (resetting to 0).
	local e = enemy(0, 1000)
	for i = 1, 12 do
		Status.applyOnHit(e, onHit({ frost_stacks = 2 }))
		eq(e.status.frost_stacks, i * 2, "stack " .. i)
		eq(e.status.freeze_ticks, 0, "no freeze at " .. (i * 2))
		eq(e.status.frost_ticks, 150, "frost duration refreshed")
	end
	Status.applyOnHit(e, onHit({ frost_stacks = 2 }))
	eq(e.status.frost_stacks, 0, "26 ≥ 25 ⇒ freeze resets stacks")
	eq(e.status.frost_ticks, 0, "frost slow cleared on freeze")
	eq(e.status.freeze_ticks, 45, "freeze armed for 1.5 s")
end)

test("frost_freezes_at_max_resetting_stacks_and_boosting_damage", function()
	local e = enemy(0, 1000)
	-- Reaching the cap (25) freezes: stacks reset, freeze timer set.
	Status.applyOnHit(e, onHit({ frost_stacks = FROST_MAX_STACKS }))
	eq(e.status.frost_stacks, 0, "stacks reset on freeze")
	eq(e.status.frost_ticks, 0, "frost slow cleared on freeze")
	eq(e.status.freeze_ticks, 45, "freeze armed for 1.5 s")
	check(Status.isImmobile(e), "frozen enemy is immobile")
	-- Frozen enemy takes +50% damage.
	eq(scale(Status.vulnerabilityMult(e), 1000), 1500, "+50% while frozen")

	-- Overshooting the cap in one application still freezes once.
	local e2 = enemy(0, 1000)
	Status.applyOnHit(e2, onHit({ frost_stacks = 30 }))
	eq(e2.status.frost_stacks, 0, "overshoot resets")
	eq(e2.status.freeze_ticks, 45, "overshoot freezes")

	-- Freeze decays to nothing and restores normal damage taken.
	local s = arenaWith({ e })
	for _ = 1, 45 do
		Status.tick(s)
	end
	eq(s.enemies[1].status.freeze_ticks, 0, "freeze wears off")
	check(not Status.isImmobile(s.enemies[1]), "no longer immobile")
	check(Fixed.eq(Status.vulnerabilityMult(s.enemies[1]), Fixed.ONE), "back to identity")
end)

test("frost_slow_is_floored_at_ten_percent", function()
	-- The cap is 25, but the floor is what protects the multiplier if stacks
	-- ever exceed 50 (2% each). Set them directly to probe the curve.
	local e = enemy(0, 1000)
	e.status.frost_stacks = 45
	-- 90% slow: `1 - from_ratio(90,100)` = 65536 - 58982 = 6554 raw, which is
	-- (just) above the floor, so the computed value is returned.
	eq(scale(Status.moveSpeedMult(e), 1000), 100, "45 × 2% = 90% slow → ×0.10")
	e.status.frost_stacks = 60
	-- Past 100% slow the floor is returned instead — and the floor is
	-- `from_ratio(1,10)` = 6553 raw, one raw unit BELOW the 6554 above, so it
	-- scales to 99, not 100. Fixed-point flooring, not a rounding accident.
	eq(scale(Status.moveSpeedMult(e), 1000), 99, "past 100% slow, floored at from_ratio(1,10)")
	e.status.frost_stacks = 0
	check(Fixed.eq(Status.moveSpeedMult(e), Fixed.ONE), "no frost ⇒ identity")
end)

-- ============================ Status: poison ============================

test("poison_application_keeps_stronger_dot", function()
	local e = enemy(0, 1000)
	Status.applyOnHit(e, onHit({ poison_dps = 20, poison_ticks = 90 }))
	eq(e.status.poison_dps, 20, "poison applied")
	-- Weaker incoming poison does not replace.
	Status.applyOnHit(e, onHit({ poison_dps = 5, poison_ticks = 10 }))
	eq(e.status.poison_dps, 20, "weaker DoT rejected")
	eq(e.status.poison_ticks, 90, "duration kept")
end)

test("poison_ticks_deal_damage_and_expire", function()
	local e = enemy(0, 1000)
	e.status.poison_dps = 100
	e.status.poison_ticks = 3
	local s = arenaWith({ e })
	for _ = 1, 3 do
		Status.tick(s)
	end
	eq(s.enemies[1].hp, 700, "3 ticks × 100 dps")
	eq(s.enemies[1].status.poison_ticks, 0, "expired")
	eq(s.enemies[1].status.poison_dps, 0, "poison cleared on expiry")
end)

test("poison_kill_records_bounty", function()
	local e = enemy(0, 50)
	e.status.poison_dps = 100
	e.status.poison_ticks = 5
	local s = arenaWith({ e })
	Status.tick(s)
	eq(#s.enemies, 0, "poison killed the enemy")
	eq(#s.pending_kills, 1, "one kill recorded")
	eq(s.pending_kills[1], 0, "the enemy's def")
end)

test("boss_is_immune_to_poison", function()
	local e = enemy(BOSS, 1000)
	e.status.poison_dps = 100
	e.status.poison_ticks = 5
	local s = arenaWith({ e })
	Status.tick(s)
	eq(s.enemies[1].hp, 1000, "boss takes no poison damage")
	eq(s.enemies[1].status.poison_ticks, 4, "but the timer still decays")
end)

test("poison_damage_accumulates_on_scoreboard", function()
	local function poisoned()
		local e = enemy(0, 1000)
		e.status.poison_dps = 30
		e.status.poison_ticks = 5
		return e
	end
	local s = arenaWith({ poisoned(), poisoned() })
	s.economy.gold_per_damage = Fixed.fromRatio(1, 4) -- exactly representable
	local gold0 = s.economy.gold
	Status.tick(s)
	-- 2 enemies × 30 dps = 60 poison damage this tick.
	eq(s.total_damage_dealt, 60, "poison counted as player damage")
	eq(s.economy.gold, gold0 + 15, "60 × 1/4 = 15 gold")
end)

test("heal_on_poison_heals_per_poisoned_enemy_capped", function()
	local function poisoned()
		local e = enemy(0, 1000)
		e.status.poison_dps = 10
		e.status.poison_ticks = 5
		return e
	end
	local s = arenaWith({ poisoned(), poisoned() })
	s.tank.max_hp = 1000
	s.tank.hp = 500
	s.tank.heal_on_poison = 5
	Status.tick(s)
	-- 2 enemies took poison ⇒ +10 HP.
	eq(s.tank.hp, 510, "heal per poisoned enemy")

	-- Cap at max_hp.
	s.tank.hp = 998
	Status.tick(s)
	eq(s.tank.hp, 1000, "capped at max_hp")
end)

-- ============================ Status: fire ============================

test("fire_increases_vulnerability", function()
	local e = enemy(0, 1000)
	Status.applyOnHit(e, onHit({ fire_stacks = 10 }))
	-- 10 × 0.5% = +5% ⇒ ≈×1.05 (fixed-point floors deterministically).
	local v = scale(Status.vulnerabilityMult(e), 1000000)
	check(v >= 1049000 and v <= 1050000, "≈+5% damage taken, got " .. v)
	check(Fixed.eq(Status.vulnerabilityMult(enemy(0, 1000)), Fixed.ONE), "no fire ⇒ identity")
end)

test("fire_stacked_enemy_explodes_on_death_damaging_neighbors", function()
	-- A 50-fire-stack enemy dies → explosion = floor(50/5) = 10 damage to
	-- each neighbor within FIRE_EXPLOSION_RADIUS; a far enemy is untouched.
	local dead = enemyAt(1, 0, -1, 0) -- already dead
	dead.status.fire_stacks = 50
	local s = arenaWith({
		dead,
		-- Neighbor in range, low hp → the 10 explosion damage chains a kill.
		enemyAt(2, 0, 8, 100),
		-- Neighbor in range, high hp → survives, takes 10.
		enemyAt(3, 0, 1000, 200),
		-- Out of range (> 300) → untouched.
		enemyAt(4, 0, 1000, 1000),
	})
	Status.reapDead(s)
	eq(#s.enemies, 2, "two survivors remain")
	eq(#s.pending_kills, 2, "source + chained kill recorded")
	eq(s.pending_kills[1], 0, "source def")
	eq(s.pending_kills[2], 0, "chained def")
	eq(s.enemies[1].id, 3, "id 3 survived")
	eq(s.enemies[1].hp, 990, "in-range survivor took 10 explosion damage")
	eq(s.enemies[2].id, 4, "id 4 survived")
	eq(s.enemies[2].hp, 1000, "out-of-range enemy untouched")
	-- Explosion damage (10 to the chained kill + 10 to the survivor) scores.
	eq(s.total_damage_dealt, 20, "explosion damage scores")
end)

test("no_fire_no_explosion", function()
	local s = arenaWith({ enemyAt(1, 0, -1, 0), enemyAt(2, 0, 100, 50) })
	Status.reapDead(s)
	eq(#s.enemies, 1, "one survivor")
	eq(s.enemies[1].hp, 100, "neighbor unharmed without Fire")
	eq(#s.pending_kills, 1, "one kill")
	eq(s.pending_kills[1], 0, "def recorded")
end)

test("fire_explosion_is_id_ordered_and_deterministic", function()
	local function build()
		local d = enemyAt(1, 0, -1, 0)
		d.status.fire_stacks = 100
		return arenaWith({ d, enemyAt(2, 0, 5, 80), enemyAt(3, 0, 5, 120) })
	end
	local a, b = build(), build()
	Status.reapDead(a)
	Status.reapDead(b)
	eq(#a.enemies, #b.enemies, "same survivor count")
	eq(#a.pending_kills, #b.pending_kills, "same kill count")
	eq(a.total_damage_dealt, b.total_damage_dealt, "same damage")
	-- Kills are recorded in id order across the chained passes.
	eq(#a.pending_kills, 3, "all three died")
end)

test("boss_neither_explodes_nor_takes_explosion_damage", function()
	local d = enemyAt(1, 0, -1, 0)
	d.status.fire_stacks = 500 -- floor(500/5) = 100 damage
	local s = arenaWith({ d, enemyAt(2, BOSS, 1000, 100), enemyAt(3, 0, 1000, 100) })
	Status.reapDead(s)
	eq(s.enemies[1].hp, 1000, "boss immune to the explosion")
	eq(s.enemies[2].hp, 900, "non-boss neighbor took 100")
end)

-- ============================ Status: stun / pulse ============================

test("stun_immobilizes_then_wears_off", function()
	local e = enemy(0, 1000)
	Status.applyOnHit(e, onHit({ stun_ticks = 2 }))
	check(Status.isImmobile(e), "stunned ⇒ immobile")
	local s = arenaWith({ e })
	Status.tick(s) -- 2 → 1
	check(Status.isImmobile(s.enemies[1]), "still stunned")
	Status.tick(s) -- 1 → 0
	check(not Status.isImmobile(s.enemies[1]), "stun wore off")
end)

test("stun_takes_the_longer_remaining", function()
	local e = enemy(0, 1000)
	Status.applyOnHit(e, onHit({ stun_ticks = 20 }))
	Status.applyOnHit(e, onHit({ stun_ticks = 5 }))
	eq(e.status.stun_ticks, 20, "shorter stun does not shorten")
	Status.applyOnHit(e, onHit({ stun_ticks = 30 }))
	eq(e.status.stun_ticks, 30, "longer stun extends")
end)

test("vulnerability_pulse_stacks_and_raises_damage_taken", function()
	local s = State.new(7, 0)
	s.vuln_pulses = { { magnitude = 5, range = 1200, interval_ticks = 30, next_tick = 30 } }

	-- An enemy in range (≤1200), and one far outside.
	s.enemies = { enemyAt(1, 0, 1000000, 500), enemyAt(2, 0, 1000000, 5000) }
	check(Fixed.eq(Status.vulnerabilityMult(s.enemies[1]), Fixed.ONE), "clean to start")

	-- First pulse at tick 30 adds 5 stacks to the near enemy only.
	s.tick = 30
	Status.pulse(s)
	eq(s.enemies[1].status.vuln_stacks, 5, "near enemy stacked")
	eq(s.enemies[2].status.vuln_stacks, 0, "far enemy unaffected")
	local v = scale(Status.vulnerabilityMult(s.enemies[1]), 1000)
	check(v >= 1049 and v <= 1050, "≈+5% damage taken, got " .. v)

	-- Second pulse stacks further.
	s.tick = 60
	Status.pulse(s)
	eq(s.enemies[1].status.vuln_stacks, 10, "stacks accumulate")
end)

test("vulnerability_pulse_catches_up_on_missed_intervals", function()
	-- The `while s.tick >= p.next_tick` loop fires once per elapsed interval, so
	-- a jump forward applies every pulse it skipped.
	local s = State.new(7, 0)
	s.vuln_pulses = { { magnitude = 1, range = 1200, interval_ticks = 30, next_tick = 30 } }
	s.enemies = { enemyAt(1, 0, 1000000, 0) }
	s.tick = 120
	Status.pulse(s)
	eq(s.enemies[1].status.vuln_stacks, 4, "ticks 30/60/90/120 all fired")
	eq(s.vuln_pulses[1].next_tick, 150, "next_tick advanced past the current tick")
end)

-- ============================ Defense: dodge ============================

test("dodge_can_avoid_a_hit_and_is_deterministic", function()
	-- 100% dodge → no damage, regardless of roll.
	local s = fresh()
	s.tank.dodge_num = 100
	s.tank.dodge_den = 100
	local hp0 = s.tank.hp
	Defense.hitTank(s, 9999)
	eq(s.tank.hp, hp0, "guaranteed dodge takes no damage")
	check(not s.tank_hit_this_tick, "a dodged hit does not arm Spikes")

	-- Partial dodge: deterministic for a fixed rng.
	local a, b = fresh(), fresh()
	a.tank.dodge_num = 50
	b.tank.dodge_num = 50
	for _ = 1, 100 do
		Defense.hitTank(a, 100)
		Defense.hitTank(b, 100)
	end
	eq(a.tank.hp, b.tank.hp, "same seed → same dodge outcomes")
	local ah, al = Rng.state(a.rng_proc)
	local bh, bl = Rng.state(b.rng_proc)
	eq(ah, bh, "rng_proc hi in lockstep")
	eq(al, bl, "rng_proc lo in lockstep")
	check(a.tank.hp < fresh().tank.hp, "some hits landed")
end)

test("dodge_stream_discipline", function()
	-- SIM-SPEC §S4: the proc draw is conditional on `dodge_den > 0` and happens
	-- BEFORE the `< dodge_num` comparison — so it is consumed on landed hits too.
	-- A `raw <= 0` hit returns before the roll and must consume nothing.
	local s = fresh()
	local h0, l0 = Rng.state(s.rng_proc)
	Defense.hitTank(s, 0)
	Defense.hitTank(s, -5)
	local h1, l1 = Rng.state(s.rng_proc)
	eq(h1, h0, "no draw for non-positive damage (hi)")
	eq(l1, l0, "no draw for non-positive damage (lo)")

	-- dodge_den == 0 ⇒ no roll at all, even on a real hit.
	s.tank.dodge_den = 0
	Defense.hitTank(s, 100)
	local h2, l2 = Rng.state(s.rng_proc)
	eq(h2, h0, "no draw when dodge_den == 0 (hi)")
	eq(l2, l0, "no draw when dodge_den == 0 (lo)")
	check(s.tank_hit_this_tick, "the hit still landed")

	-- dodge_den > 0 with dodge_num == 0 ⇒ the roll is consumed and always fails.
	s.tank.dodge_den = 100
	Defense.hitTank(s, 100)
	local h3, l3 = Rng.state(s.rng_proc)
	check(h3 ~= h0 or l3 ~= l0, "a roll was consumed even though dodge is 0%")
end)

-- ============================ Defense: armor / shield ============================

test("plain_damage_reduces_hp", function()
	local s = fresh()
	local hp0 = s.tank.hp
	Defense.hitTank(s, 500)
	eq(s.tank.hp, hp0 - 500, "full damage lands")
end)

test("armor_reduces_but_min_one_lands", function()
	local s = fresh()
	s.tank.armor = 100
	local hp0 = s.tank.hp
	Defense.hitTank(s, 500)
	eq(s.tank.hp, hp0 - 400, "armor subtracts flat")
	-- A hit fully absorbed by armor still deals 1.
	s.tank.armor = 10000
	local hp1 = s.tank.hp
	Defense.hitTank(s, 500)
	eq(s.tank.hp, hp1 - 1, "min 1 always lands")
end)

test("mana_shield_absorbs_then_overflows_to_hp", function()
	local s = fresh()
	s.tank.mana_shield = 300
	s.tank.mana_shield_max = 300
	local hp0 = s.tank.hp
	Defense.hitTank(s, 500) -- 300 absorbed, 200 to HP
	eq(s.tank.mana_shield, 0, "pool drained")
	eq(s.tank.hp, hp0 - 200, "overflow to HP")
end)

test("shield_active_dr_reduces_hits_while_shield_up_then_stops", function()
	-- 50% DR (1/2 is exactly representable) with a 300 shield pool.
	local s = fresh()
	s.tank.mana_shield = 300
	s.tank.mana_shield_max = 300
	s.tank.shield_active_dr = Fixed.fromRatio(1, 2)
	local hp0 = s.tank.hp

	-- Hit of 500 while shield up: reduced to 250, fully absorbed (≤300 pool),
	-- no HP loss. Shield drops to 50.
	Defense.hitTank(s, 500)
	eq(s.tank.mana_shield, 50, "250 absorbed of a 300 pool")
	eq(s.tank.hp, hp0, "fully absorbed → no HP loss")

	-- Next 500 hit, shield still up (50): reduced to 250, 50 absorbed, 200 to HP.
	Defense.hitTank(s, 500)
	eq(s.tank.mana_shield, 0, "remaining shield drained")
	eq(s.tank.hp, hp0 - 200, "overflow also reduced (250-50=200)")

	-- Shield now 0 → DR no longer applies. A 500 hit lands in full to HP.
	local hp1 = s.tank.hp
	Defense.hitTank(s, 500)
	eq(s.tank.hp, hp1 - 500, "no reduction once shield is 0")
end)

test("shield_active_dr_keeps_min_one_and_clamps", function()
	-- 100% DR must still let at least 1 through while the shield is up.
	local s = fresh()
	s.tank.mana_shield = 1000
	s.tank.mana_shield_max = 1000
	s.tank.shield_active_dr = Fixed.ONE
	Defense.hitTank(s, 500)
	eq(s.tank.mana_shield, 999, "min-1 still absorbed at 100% DR")

	-- Over-100% DR is clamped to 100% (multiplier never negative): same result.
	local s2 = fresh()
	s2.tank.mana_shield = 1000
	s2.tank.mana_shield_max = 1000
	s2.tank.shield_active_dr = Fixed.fromInt(5) -- 500% → clamped to 100%
	local hp0 = s2.tank.hp
	Defense.hitTank(s2, 500)
	eq(s2.tank.mana_shield, 999, "clamped DR still lets 1 land")
	eq(s2.tank.hp, hp0, "no negative-damage HP gain from over-clamp")
end)

test("heal_on_damaged_heals_landed_hit_not_dodged_and_caps", function()
	local s = fresh()
	s.tank.hp = s.tank.max_hp - 1000
	s.tank.heal_on_damaged = 8
	local before = s.tank.hp
	Defense.hitTank(s, 100)
	eq(s.tank.hp, before + 8 - 100, "landed hit heals then takes damage")

	-- healing_mult scales the heal.
	s = fresh()
	s.tank.hp = s.tank.max_hp - 1000
	s.tank.heal_on_damaged = 8
	s.tank.healing_mult = Fixed.fromInt(2)
	before = s.tank.hp
	Defense.hitTank(s, 100)
	eq(s.tank.hp, before + 16 - 100, "heal scaled by healing_mult")

	-- Dodged hit does NOT heal (and takes no damage).
	s = fresh()
	s.tank.dodge_num = 100
	s.tank.dodge_den = 100
	s.tank.hp = s.tank.max_hp - 1000
	s.tank.heal_on_damaged = 8
	before = s.tank.hp
	Defense.hitTank(s, 100)
	eq(s.tank.hp, before, "dodged hit neither heals nor damages")

	-- Max-HP cap honored.
	s = fresh()
	s.tank.hp = s.tank.max_hp
	s.tank.heal_on_damaged = 8
	Defense.hitTank(s, 4)
	eq(s.tank.hp, s.tank.max_hp - 4, "heal capped at max, damage still applies")
end)

-- ============================ Defense: spikes ============================

test("spikes_retaliate_on_nearby_enemies_only_when_hit", function()
	local s = fresh()
	s.tank.spikes_damage = 100
	s.enemies = { enemyAt(1, 0, 200, 100), enemyAt(2, 0, 200, 1000) } -- near, far

	-- Not hit this tick → no retaliation.
	Defense.spikes(s)
	eq(s.enemies[1].hp, 200, "no hit ⇒ no retaliation")

	-- Take a hit, then retaliate: near enemy takes 100, far enemy untouched.
	Defense.hitTank(s, 500)
	check(s.tank_hit_this_tick, "hit flag armed by hitTank")
	Defense.spikes(s)
	check(not s.tank_hit_this_tick, "hit flag consumed")
	eq(s.enemies[1].hp, 100, "near enemy took spikes")
	eq(s.enemies[2].hp, 200, "far enemy out of range")
end)

test("spikes_flag_is_false_at_the_tick_boundary_even_with_no_spikes_stat", function()
	-- The flag must be cleared by `spikes` whether or not retaliation happens —
	-- it is excluded from the checksum precisely because it never survives a tick.
	local s = fresh()
	s.enemies = { enemyAt(1, 0, 200, 50) }
	Defense.hitTank(s, 100) -- sets the flag
	check(s.tank_hit_this_tick, "armed")
	Defense.spikes(s)
	check(not s.tank_hit_this_tick, "cleared even with spikes_damage == 0")
	eq(s.enemies[1].hp, 200, "no spikes stat → no retaliation")
end)

test("spikes_mult_scales_and_kills_award_bounty", function()
	local s = fresh()
	s.tank.spikes_damage = 100
	s.tank.spikes_mult = Fixed.fromRatio(3, 2) -- ×1.5 ⇒ 150
	s.enemies = { enemyAt(1, 0, 150, 50) }
	Defense.hitTank(s, 100)
	Defense.spikes(s)
	eq(#s.enemies, 0, "150 spikes killed the 150-hp enemy")
	eq(#s.pending_kills, 1, "kill recorded")
	eq(s.pending_kills[1], 0, "def recorded")
end)

test("spikes_damage_records_to_scoreboard_and_bloodmoney", function()
	local s = fresh()
	s.tank.spikes_damage = 200
	s.economy.gold_per_damage = Fixed.fromRatio(1, 4)
	local gold0 = s.economy.gold
	s.enemies = { enemyAt(1, 0, 1000, 50) }
	Defense.hitTank(s, 100)
	Defense.spikes(s)
	eq(s.total_damage_dealt, 200, "spikes counted as player damage")
	eq(s.economy.gold, gold0 + 50, "200 × 1/4 = 50 gold")
	eq(s.total_gold_earned, 50, "scoreboard credited")
end)

test("spikes_apply_poison_to_the_reflected_attacker", function()
	local s = fresh()
	s.tank.spikes_damage = 100
	s.tank.spikes_poison_dps = 2
	s.tank.spikes_poison_ticks = 90
	s.enemies = { enemyAt(1, 0, 1000, 100), enemyAt(2, 0, 1000, 1000) } -- near, far
	Defense.hitTank(s, 100)
	Defense.spikes(s)
	eq(s.enemies[1].hp, 900, "near enemy took 100 spikes")
	eq(s.enemies[1].status.poison_dps, 2, "near enemy poisoned")
	eq(s.enemies[1].status.poison_ticks, 90, "poison duration")
	eq(s.enemies[2].hp, 1000, "far enemy untouched")
	eq(s.enemies[2].status.poison_dps, 0, "far enemy not poisoned")
end)

test("stacking_spikes_accumulate_to_the_cap_then_respect_the_reset", function()
	-- per-stack 20, cap 3 → bonus grows 20,40,60 then holds at 60.
	local s = fresh()
	s.tank.spikes_damage = 0 -- isolate the stacking bonus
	s.tank.spikes_stack_per = 20
	s.tank.spikes_stacks_max = 3
	s.enemies = { enemyAt(1, 0, 1000000, 100) }

	local function hitAndSpike(): number
		Defense.hitTank(s, 100)
		local before = s.enemies[1].hp
		Defense.spikes(s)
		return before - s.enemies[1].hp
	end
	eq(hitAndSpike(), 20, "stack 1 ⇒ 20 spikes")
	eq(s.tank.spikes_stacks, 1, "one stack")
	eq(hitAndSpike(), 40, "stack 2 ⇒ 40 spikes")
	eq(hitAndSpike(), 60, "stack 3 ⇒ 60 spikes (cap)")
	eq(hitAndSpike(), 60, "stays at cap, no further growth")
	eq(s.tank.spikes_stacks, 3, "stacks capped at max")

	-- Reset (the round boundary clears it): a fresh stack starts at 20 again.
	s.tank.spikes_stacks = 0
	eq(hitAndSpike(), 20, "after reset, stack 1 ⇒ 20 again")
end)

test("spikes_skip_the_boss", function()
	local s = fresh()
	s.tank.spikes_damage = 100
	s.enemies = { enemyAt(1, BOSS, 1000, 50) }
	Defense.hitTank(s, 100)
	Defense.spikes(s)
	eq(s.enemies[1].hp, 1000, "boss takes no spikes damage")
end)

-- ============================ Defense: shield-break stun ============================

test("shield_break_stun_fires_on_the_down_edge_and_stuns_only_in_range", function()
	local s = fresh()
	s.tank.mana_shield = 300
	s.tank.mana_shield_max = 300
	s.tank.shieldbreak_stun_range = 500
	s.tank.shieldbreak_stun_ticks = 15
	s.enemies = { enemyAt(1, 0, 1000, 400), enemyAt(2, 0, 1000, 1000) }

	-- A hit that does NOT deplete the shield → no break, no stun.
	Defense.hitTank(s, 100)
	check(not s.shield_broke_this_tick, "shield still up → no break edge")
	Defense.shieldBreakStun(s)
	eq(s.enemies[1].status.stun_ticks, 0, "no stun while shield holds")

	-- A hit that depletes the shield → the >0→0 down-edge fires the pulse.
	Defense.hitTank(s, 9999)
	check(s.shield_broke_this_tick, "shield broke this tick")
	Defense.shieldBreakStun(s)
	check(not s.shield_broke_this_tick, "break flag consumed")
	eq(s.enemies[1].status.stun_ticks, 15, "in-range enemy stunned")
	eq(s.enemies[2].status.stun_ticks, 0, "out-of-range enemy not stunned")
end)

test("shield_break_stun_does_not_fire_when_shield_already_zero", function()
	local s = fresh()
	s.tank.mana_shield = 0
	s.tank.mana_shield_max = 300
	s.tank.shieldbreak_stun_range = 500
	s.tank.shieldbreak_stun_ticks = 15
	s.enemies = { enemyAt(1, 0, 1000, 100) }
	Defense.hitTank(s, 500)
	check(not s.shield_broke_this_tick, "no shield to break ⇒ no edge")
	Defense.shieldBreakStun(s)
	eq(s.enemies[1].status.stun_ticks, 0, "no stun when shield was already 0")
end)

test("shield_break_flag_needs_the_pulse_armed", function()
	-- `shieldbreak_stun_range == 0` ⇒ the flag is never set, so the pulse is a
	-- no-op and the flag is still false at the tick boundary.
	local s = fresh()
	s.tank.mana_shield = 100
	s.tank.mana_shield_max = 100
	s.enemies = { enemyAt(1, 0, 1000, 100) }
	Defense.hitTank(s, 9999)
	eq(s.tank.mana_shield, 0, "shield drained")
	check(not s.shield_broke_this_tick, "pulse not armed ⇒ flag stays false")
	Defense.shieldBreakStun(s)
	eq(s.enemies[1].status.stun_ticks, 0, "no stun")
end)

test("shield_break_stun_is_deterministic_across_two_runs", function()
	local function build()
		local s = fresh()
		s.tank.mana_shield = 100
		s.tank.mana_shield_max = 100
		s.tank.shieldbreak_stun_range = 600
		s.tank.shieldbreak_stun_ticks = 15
		s.enemies = { enemyAt(3, 0, 1000, 200), enemyAt(1, 0, 1000, 300), enemyAt(2, 0, 1000, 5000) }
		return s
	end
	local a, b = build(), build()
	Defense.hitTank(a, 9999)
	Defense.hitTank(b, 9999)
	Defense.shieldBreakStun(a)
	Defense.shieldBreakStun(b)
	for i = 1, #a.enemies do
		eq(a.enemies[i].status.stun_ticks, b.enemies[i].status.stun_ticks, "enemy " .. i .. " stun matches")
	end
	eq(a.enemies[1].status.stun_ticks, 15, "in-range stunned")
	eq(a.enemies[3].status.stun_ticks, 0, "far enemy untouched")
end)

test("shield_break_stun_takes_the_longer_remaining", function()
	local s = fresh()
	s.tank.mana_shield = 100
	s.tank.mana_shield_max = 100
	s.tank.shieldbreak_stun_range = 600
	s.tank.shieldbreak_stun_ticks = 5
	local e = enemyAt(1, 0, 1000, 100)
	e.status.stun_ticks = 40
	s.enemies = { e }
	Defense.hitTank(s, 9999)
	Defense.shieldBreakStun(s)
	eq(s.enemies[1].status.stun_ticks, 40, "longer existing stun preserved")
end)

-- ============================ Defense: regen ============================

test("regen_refills_up_to_max", function()
	local s = fresh()
	s.tank.mana_shield = 0
	s.tank.mana_shield_max = 100
	s.tank.mana_regen_per_tick = 30
	s.tank.hp = s.tank.max_hp - 50
	s.tank.hp_regen_per_tick = 20
	for _ = 1, 10 do
		Defense.regen(s)
	end
	eq(s.tank.mana_shield, 100, "shield capped at max")
	eq(s.tank.hp, s.tank.max_hp, "hp capped at max")
end)

test("healing_mult_scales_hp_regen", function()
	local s = fresh()
	s.tank.hp = s.tank.max_hp - 1000
	s.tank.hp_regen_per_tick = 100
	s.tank.healing_mult = Fixed.fromInt(2)
	Defense.regen(s)
	eq(s.tank.hp, s.tank.max_hp - 1000 + 200, "regen doubled by healing mult")
end)

test("missing_hp_heal_pulses_once_per_second", function()
	local s = fresh()
	s.tank.max_hp = 10000
	s.tank.hp = 0
	s.tank.missing_hp_heal_pct = Fixed.fromRatio(1, 4)

	-- Mid-second → no pulse.
	s.tick = 15
	Defense.regen(s)
	eq(s.tank.hp, 0, "only pulses on a second boundary")

	-- On the boundary → heals 25% of the 10000 deficit.
	s.tick = 30
	Defense.regen(s)
	eq(s.tank.hp, 2500, "25% of missing")
end)

test("negative_regen_drains_hp_each_tick", function()
	local s = fresh()
	s.tank.hp = 1000
	s.tank.max_hp = 24000
	s.tank.hp_regen_per_tick = -100
	Defense.regen(s)
	eq(s.tank.hp, 900, "drain subtracts magnitude each tick")
	Defense.regen(s)
	eq(s.tank.hp, 800, "and again")
	-- The drain bypasses healing_mult and can push HP below zero (death path).
	s.tank.healing_mult = Fixed.fromInt(5)
	s.tank.hp = 50
	Defense.regen(s)
	eq(s.tank.hp, -50, "drain unaffected by healing_mult, can go fatal")
end)

-- ============================ summary ============================

print(("status_test: %d passed, %d failed"):format(passed, failed))
if failed > 0 then
	error("status_test: " .. failed .. " failure(s)", 0)
end
