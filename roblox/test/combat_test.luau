--[[
	combat_test.luau — unit tests for `src/shared/sim/Combat.luau`, the
	transcription of `sim-core/crates/sim/src/combat.rs`.

	SCOPE. These cover the individually testable pieces: the armor/damage matrix
	application, target selection under a fixed RNG state (and the exact number
	of draws each attack type consumes), projectile arrival / splash / stale
	targets, the bounce and barrage target SETS, contact damage and the boss
	cadence, and the hazard / aura / minion / enemy-ranged phases.

	They are NOT the done-condition for R3. The oracle is the per-tick checksum
	trace diff against the Rust (`SIM-SPEC.md` §S6), which needs every module.
	These exist so that a divergence has somewhere cheap to be caught first.

	HOW TO RUN (standalone Luau, no file I/O needed — the catalog comes from
	`Content.luau`, not from argv):

	    cd roblox/test && luau combat_test.luau

	Expected values are derived from `combat.rs` and the shipped `content.json`,
	never hand-waved: where a number comes from the catalog it is read back out
	of `Content` in the test rather than pasted in.
]]

local Fixed = require("../src/shared/Fixed")
local Rng = require("../src/shared/Rng")
local Content = require("../src/shared/Content")
local State = require("../src/shared/sim/State")
local Combat = require("../src/shared/sim/Combat")

local Vec2 = State.Vec2

-- ============================ tiny test harness ============================

local passed, failed = 0, 0
local currentTest = "<none>"

local function fail(msg: string)
	failed += 1
	print(string.format("  FAIL [%s] %s", currentTest, msg))
end

local function check(cond: boolean, msg: string)
	if cond then
		passed += 1
	else
		fail(msg)
	end
end

local function eq(got, want, msg: string)
	check(got == want, string.format("%s: got %s, want %s", msg, tostring(got), tostring(want)))
end

local function feq(got, want, msg: string)
	check(
		Fixed.eq(got, want),
		string.format("%s: got %s, want %s", msg, Fixed.toDecimal(got), Fixed.toDecimal(want))
	)
end

local tests: { { name: string, fn: () -> () } } = {}
local function test(name: string, fn: () -> ())
	table.insert(tests, { name = name, fn = fn })
end

-- ============================ state fixtures ============================
-- The state table is built here rather than through a `State` constructor so the
-- test depends only on the field names, which SIM-SPEC §S3 pins as byte-identical
-- to `state.rs`.

local function zeroFixedArray(n: number)
	local t = {}
	for i = 1, n do
		t[i] = Fixed.ZERO
	end
	return t
end

local function newModifiers()
	return {
		add_global = Fixed.ZERO,
		add_by_type = zeroFixedArray(5),
		add_by_scope = zeroFixedArray(12),
		mul_global = Fixed.ONE,
		attack_speed = Fixed.ZERO,
		vs_stunned = Fixed.ZERO,
		vs_poisoned = Fixed.ZERO,
		poison_dmg_mult = Fixed.ONE,
		stun_dur_mult = Fixed.ONE,
		weapon_count_scaling = {},
		dmg_per_maxhp_rate = Fixed.ZERO,
		dmg_per_bounty_rate = Fixed.ZERO,
		shield_active_dmg = Fixed.ZERO,
	}
end

local PURPOSE_SPAWN, PURPOSE_TARGETING = 0, 1
local PURPOSE_SHOP, PURPOSE_REROLL, PURPOSE_PROC = 2, 3, 4

--- A fresh arena with NO weapons (tests push their own) and no enemies.
local function blankState()
	local seed = 0xABCDEF
	local player = 0
	return {
		tick = 0,
		round = 4294967295,
		master_seed = seed,
		player_id = player,
		tank = {
			hp = 24000,
			max_hp = 24000,
			pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
			clear_cooldown_end = 0,
			armor = 0,
			dodge_num = 0,
			dodge_den = 100,
			mana_shield = 0,
			mana_shield_max = 0,
			mana_regen_per_tick = 0,
			hp_regen_per_tick = 0,
			spikes_damage = 0,
			spikes_mult = Fixed.ONE,
			shield_active_dr = Fixed.ZERO,
			heal_on_damaged = 0,
			heal_on_kill = 0,
			mana_on_kill = 0,
			heal_on_poison = 0,
			healing_mult = Fixed.ONE,
			missing_hp_heal_pct = Fixed.ZERO,
			revives = 0,
			revive_bonus_hp = 0,
			shieldbreak_stun_range = 0,
			shieldbreak_stun_ticks = 0,
			spikes_poison_dps = 0,
			spikes_poison_ticks = 0,
			spikes_stack_per = 0,
			spikes_stacks = 0,
			spikes_stacks_max = 0,
			aura_range = 0,
			aura_cadence = 0,
			aura_damage = 0,
			aura_poison_dps = 0,
			aura_poison_ticks = 0,
			aura_tick = 0,
		},
		weapons = {},
		enemies = {},
		projectiles = {},
		hazards = {},
		minions = {},
		economy = {
			gold = 500,
			income_per_tick = 20,
			income_mult = Fixed.ONE,
			income_regen_pct = Fixed.ZERO,
			bounty_mult = Fixed.ONE,
			bounty_proc_chance_pct = 0,
			bounty_proc_bonus = Fixed.ZERO,
			gold_per_damage = Fixed.ZERO,
			income_shield_pct = Fixed.ZERO,
			rerolls_remaining = 5,
			reroll_cost = 100,
		},
		shop = { offers = {}, shop_seq = 0 },
		modifiers = newModifiers(),
		ramps = {},
		vuln_pulses = {},
		pending_perk = nil,
		tank_hit_this_tick = false,
		shield_broke_this_tick = false,
		next_entity_id = 1,
		dead = false,
		death_tick = nil,
		pending_kills = {},
		total_damage_dealt = 0,
		total_gold_earned = 0,
		bought_attack_mask = 0,
		weapons_bought = 0,
		economy_purchases = 0,
		rng_spawn = Rng.derive(seed, player, PURPOSE_SPAWN, 0),
		rng_targeting = Rng.derive(seed, player, PURPOSE_TARGETING, 0),
		rng_shop = Rng.derive(seed, player, PURPOSE_SHOP, 0),
		rng_reroll = Rng.derive(seed, player, PURPOSE_REROLL, 0),
		rng_proc = Rng.derive(seed, player, PURPOSE_PROC, 0),
	}
end

local function allocId(s: any): number
	local id = s.next_entity_id
	s.next_entity_id += 1
	return id
end

local function newStatus()
	return {
		poison_dps = 0,
		poison_ticks = 0,
		frost_stacks = 0,
		frost_ticks = 0,
		fire_stacks = 0,
		vuln_stacks = 0,
		stun_ticks = 0,
		freeze_ticks = 0,
	}
end

local function mkEnemy(s: any, def: number, hp: number, pos: any): number
	local id = allocId(s)
	table.insert(s.enemies, { id = id, def = def, hp = hp, pos = pos, status = newStatus() })
	return id
end

local function mkWeapon(s: any, def: number, nextFireTick: number): number
	local id = allocId(s)
	table.insert(s.weapons, { instance_id = id, def = def, next_fire_tick = nextFireTick })
	return id
end

local function at(x: number, y: number)
	return Vec2.new(Fixed.fromInt(x), Fixed.fromInt(y))
end

local NO_ON_HIT = {
	poison_dps = 0,
	poison_ticks = 0,
	frost_stacks = 0,
	fire_stacks = 0,
	stun_ticks = 0,
}
local NO_ABILITY = { tag = 0, a = 0, b = 0, c = 0 }

local function mkProjectile(s: any, p: any): number
	local id = allocId(s)
	p.id = id
	if p.on_hit == nil then
		p.on_hit = NO_ON_HIT
	end
	if p.ability == nil then
		p.ability = NO_ABILITY
	end
	table.insert(s.projectiles, p)
	return id
end

-- Catalog handles used by the tests, resolved by attack/ability shape so a
-- catalog reorder fails loudly rather than silently testing the wrong weapon.
local function findWeapon(pred: (any, number) -> boolean): number
	for i = 1, #Content.WEAPONS do
		if pred(Content.WEAPONS[i], i - 1) then
			return i - 1
		end
	end
	error("no weapon matched", 2)
end

local BOW = 0 -- Content.STARTING_WEAPON; single-target, piercing, range 900
local W_SPLASH = findWeapon(function(w)
	return w.attack.tag == 1
end)
local W_BARRAGE = findWeapon(function(w)
	return w.attack.tag == 2
end)
local W_AREA = findWeapon(function(w)
	return w.attack.tag == 3 and w.ability.tag == 0
end)
local W_WAVE = findWeapon(function(w)
	return w.attack.tag == 4 and w.ability.tag == 0
end)
local W_BOUNCE = findWeapon(function(w)
	return w.attack.tag == 5
end)
local W_KNOCKBACK = findWeapon(function(w)
	return w.ability.tag == 3
end)
local W_HAZARD = findWeapon(function(w)
	return w.ability.tag == 6
end)
local W_SUMMON = findWeapon(function(w)
	return w.ability.tag == 7 and w.attack.tag == 3
end)
local W_LIFEDRAIN = findWeapon(function(w)
	return w.ability.tag == 1 and w.attack.tag == 4
end)

local SQUEAK = 0 -- Squeakzilla: Light armor, move 8, contact 500
local BOSS_DEF = Content.BOSS
local BONK = 6 -- Fortified armor
-- Croak: RangedAttack { range 700, cooldown 30, damage 180 }
local CROAK = 8

-- ============================ damage matrix ============================

test("damage matrix multiplies by (damage_type, armor_class)", function()
	-- Piercing (1) vs Light (0) = 2x; vs Medium (1) = 1x; vs Fortified (2) = 0.35x.
	feq(Content.damageMultiplier(1, 0), Fixed.fromRatio(2, 1), "piercing vs light")
	feq(Content.damageMultiplier(3, 1), Fixed.fromRatio(1, 2), "siege vs medium")
	feq(Content.damageMultiplier(3, 2), Fixed.fromRatio(3, 2), "siege vs fortified")

	-- A projectile impact is the cleanest damage site: mod_mult is ONE there
	-- (the weapon multiplier was baked at fire time).
	local s = blankState()
	local pos = at(10, 0)
	local eid = mkEnemy(s, SQUEAK, 100000, pos)
	mkProjectile(s, {
		pos = at(0, 0),
		target = eid,
		last_target_pos = pos,
		damage = 100,
		damage_type = 1, -- piercing
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(1000),
	})
	Combat.advanceProjectiles(s)
	eq(s.enemies[1].hp, 100000 - 200, "piercing 100 vs Light doubles to 200")
	eq(s.total_damage_dealt, 200, "scoreboard records the post-matrix damage")

	-- Fortified: piercing 0.35 -> floor(1000 * 0.35) via Fixed rounding.
	local s2 = blankState()
	local p2 = at(10, 0)
	local eid2 = mkEnemy(s2, BONK, 100000, p2)
	mkProjectile(s2, {
		pos = at(0, 0),
		target = eid2,
		last_target_pos = p2,
		damage = 1000,
		damage_type = 1,
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(1000),
	})
	Combat.advanceProjectiles(s2)
	local expected = Fixed.toInt(Fixed.scaleI64(Content.damageMultiplier(1, 2), 1000))
	eq(s2.enemies[1].hp, 100000 - expected, "piercing vs Fortified uses the matrix entry")
end)

test("bosses are immune to weapon fire and to its abilities", function()
	local s = blankState()
	local pos = at(10, 0)
	local eid = mkEnemy(s, BOSS_DEF, 33000000, pos)
	mkProjectile(s, {
		pos = at(0, 0),
		target = eid,
		last_target_pos = pos,
		damage = 1000000,
		damage_type = 4, -- chaos: 1x against everything
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(1000),
		on_hit = { poison_dps = 50, poison_ticks = 60, frost_stacks = 0, fire_stacks = 0, stun_ticks = 30 },
		ability = { tag = 3, a = 300, b = 0, c = 0 }, -- knockback
	})
	Combat.advanceProjectiles(s)
	eq(s.enemies[1].hp, 33000000, "boss hp untouched")
	eq(s.enemies[1].status.poison_ticks, 0, "boss takes no on-hit status")
	eq(s.total_damage_dealt, 0, "no damage recorded against a boss")
	feq(s.enemies[1].pos.x, Fixed.fromInt(10), "boss not knocked back")
end)

test("fire/vuln stacks and target-conditional bonuses multiply in", function()
	local s = blankState()
	s.modifiers.vs_stunned = Fixed.fromRatio(1, 1) -- +100% vs stunned
	local pos = at(10, 0)
	local eid = mkEnemy(s, SQUEAK, 1000000, pos)
	local e = s.enemies[1]
	e.status.fire_stacks = 100 -- +50%
	e.status.vuln_stacks = 50 -- +50%
	e.status.stun_ticks = 5 -- conditional bonus applies
	mkProjectile(s, {
		pos = at(0, 0),
		target = eid,
		last_target_pos = pos,
		damage = 1000,
		damage_type = 4, -- chaos: armor matrix is 1x
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(1000),
	})
	Combat.advanceProjectiles(s)
	-- armor(1) * mod(1) * vuln(1 + 0.5 + 0.5) * cond(1 + 1) = 4x
	local armor = Content.damageMultiplier(4, 0)
	local vuln = Fixed.add(Fixed.add(Fixed.ONE, Fixed.fromRatio(500, 1000)), Fixed.fromRatio(50, 100))
	local cond = Fixed.add(Fixed.ONE, Fixed.fromRatio(1, 1))
	local want = Fixed.toInt(Fixed.scaleI64(Fixed.mul(Fixed.mul(Fixed.mul(armor, Fixed.ONE), vuln), cond), 1000))
	eq(s.enemies[1].hp, 1000000 - want, "stacked multipliers")
	eq(want, 4000, "…which is 4x here")
end)

-- ============================ fireWeapons ============================

test("no enemy in range: does not fire AND does not advance cooldown", function()
	-- Load-bearing: `fire_weapons` deliberately leaves `next_fire_tick` alone
	-- when nothing is in range (an upstream inefficiency that is part of the
	-- oracle — do not "fix" it).
	local s = blankState()
	mkWeapon(s, BOW, 0)
	mkEnemy(s, SQUEAK, 200, at(1500, 0)) -- 1500 > Bow range 900
	local rngBefore = Rng.state(s.rng_targeting)
	Combat.fireWeapons(s)
	eq(#s.projectiles, 0, "should not fire at an out-of-range enemy")
	eq(s.weapons[1].next_fire_tick, 0, "cooldown must not advance")
	eq(Rng.state(s.rng_targeting), rngBefore, "no targeting draw when no candidates")

	mkEnemy(s, SQUEAK, 200, at(300, 0))
	Combat.fireWeapons(s)
	eq(#s.projectiles, 1, "should fire at an in-range enemy")
	local p = s.projectiles[1]
	eq(p.damage, Content.WEAPONS[BOW + 1].damage, "baked damage = base at 1x")
	eq(p.damage_type, Content.WEAPONS[BOW + 1].damage_type, "carries the weapon damage type")
	check(Fixed.eq(p.splash_radius, Fixed.ZERO), "Bow is single-target")
	eq(s.weapons[1].next_fire_tick, Content.WEAPONS[BOW + 1].cooldown_ticks, "cooldown advanced")
end)

test("weapon on cooldown does not fire", function()
	local s = blankState()
	mkWeapon(s, BOW, 100)
	mkEnemy(s, SQUEAK, 200, at(100, 0))
	s.tick = 50
	Combat.fireWeapons(s)
	eq(#s.projectiles, 0, "weapon on cooldown must not fire")
end)

test("attack-speed modifier shortens the cooldown (floor, min 1)", function()
	local s = blankState()
	mkWeapon(s, BOW, 0)
	mkEnemy(s, SQUEAK, 200, at(100, 0))
	s.modifiers.attack_speed = Fixed.fromRatio(1, 1) -- +100% => x2 => 30 -> 15
	Combat.fireWeapons(s)
	eq(s.weapons[1].next_fire_tick, 15, "cooldown halved")

	-- An absurd attack speed floors to 1, never 0.
	local s2 = blankState()
	mkWeapon(s2, BOW, 0)
	mkEnemy(s2, SQUEAK, 200, at(100, 0))
	s2.modifiers.attack_speed = Fixed.fromInt(1000)
	Combat.fireWeapons(s2)
	eq(s2.weapons[1].next_fire_tick, 1, "cooldown floors at 1 tick")
end)

test("the weapon multiplier is baked into projectile damage at fire time", function()
	local function fireBaked(addGlobal): number
		local s = blankState()
		s.modifiers.add_global = addGlobal
		mkWeapon(s, BOW, 0)
		mkEnemy(s, SQUEAK, 1000000, at(100, 0))
		Combat.fireWeapons(s)
		return s.projectiles[1].damage
	end
	local base = fireBaked(Fixed.ZERO)
	local doubled = fireBaked(Fixed.fromRatio(1, 1))
	eq(base, Content.WEAPONS[BOW + 1].damage, "1x = catalog damage")
	eq(doubled, base * 2, "+100% global doubles the baked damage")
end)

-- ---------------------------- target selection ----------------------------

test("single-target selection consumes exactly one below(#candidates)", function()
	local s = blankState()
	mkWeapon(s, BOW, 0)
	local ids = {}
	for i = 0, 4 do
		ids[i + 1] = mkEnemy(s, SQUEAK, 200, at(100 + i, i))
	end
	-- Predict the pick from a clone of the stream, then check both the target
	-- and that the live stream advanced by exactly that one draw.
	local oracle = Rng.clone(s.rng_targeting)
	local expectedIdx = Rng.below(oracle, 5) -- 0-based index into candidates
	Combat.fireWeapons(s)
	eq(#s.projectiles, 1, "one projectile")
	eq(s.projectiles[1].target, ids[expectedIdx + 1], "target matches the predicted draw")
	eq(Rng.state(s.rng_targeting), Rng.state(oracle), "exactly one draw consumed")
end)

test("splash selection draws once and carries the splash radius", function()
	local s = blankState()
	mkWeapon(s, W_SPLASH, 0)
	local wdef = Content.WEAPONS[W_SPLASH + 1]
	local ids = {}
	for i = 1, 3 do
		ids[i] = mkEnemy(s, SQUEAK, 200, at(50 * i, 0))
	end
	local oracle = Rng.clone(s.rng_targeting)
	local expectedIdx = Rng.below(oracle, 3)
	Combat.fireWeapons(s)
	eq(s.projectiles[1].target, ids[expectedIdx + 1], "splash target matches the draw")
	feq(s.projectiles[1].splash_radius, Fixed.fromInt(wdef.attack.a), "splash radius from the catalog")
	eq(Rng.state(s.rng_targeting), Rng.state(oracle), "exactly one draw consumed")
end)

test("barrage: partial Fisher-Yates, distinct targets, one draw per shot", function()
	local s = blankState()
	mkWeapon(s, W_BARRAGE, 0)
	local n = Content.WEAPONS[W_BARRAGE + 1].attack.a
	local ids = {}
	for i = 1, 6 do
		ids[i] = mkEnemy(s, SQUEAK, 200, at(40 * i, 0))
	end
	local shots = math.min(n, 6)

	-- Reproduce the Rust's partial shuffle over the candidate INDEX pool.
	local oracle = Rng.clone(s.rng_targeting)
	local pool = { 1, 2, 3, 4, 5, 6 }
	local expected = {}
	for k = 0, shots - 1 do
		local j = k + Rng.below(oracle, #pool - k)
		pool[k + 1], pool[j + 1] = pool[j + 1], pool[k + 1]
		expected[k + 1] = ids[pool[k + 1]]
	end

	Combat.fireWeapons(s)
	eq(#s.projectiles, shots, "one projectile per shot")
	eq(Rng.state(s.rng_targeting), Rng.state(oracle), "exactly one draw per shot (below(1) included)")
	local seen = {}
	for i = 1, shots do
		eq(s.projectiles[i].target, expected[i], "barrage shot " .. i)
		check(seen[s.projectiles[i].target] == nil, "barrage targets are distinct")
		seen[s.projectiles[i].target] = true
	end
end)

test("bounce: random first target, then the nearest others by (dist, id)", function()
	local s = blankState()
	mkWeapon(s, W_BOUNCE, 0)
	local wdef = Content.WEAPONS[W_BOUNCE + 1]
	local n = wdef.attack.a
	-- Five enemies in a line, all within the weapon's range of the tank.
	local r = wdef.range
	local xs = { 10, 20, 30, 40, 50 }
	local ids = {}
	for i, x in xs do
		check(x < r, "fixture inside weapon range")
		ids[i] = mkEnemy(s, SQUEAK, 1000000, at(x, 0))
	end

	local oracle = Rng.clone(s.rng_targeting)
	local firstIdx = Rng.below(oracle, 5) + 1 -- 1-based index into s.enemies

	Combat.fireWeapons(s)
	eq(Rng.state(s.rng_targeting), Rng.state(oracle), "bounce draws exactly once")

	-- Expected target set: `first` plus the n-1 nearest others to it.
	local others = {}
	for i = 1, 5 do
		if i ~= firstIdx then
			table.insert(others, i)
		end
	end
	table.sort(others, function(a, b)
		local da = math.abs(xs[a] - xs[firstIdx])
		local db = math.abs(xs[b] - xs[firstIdx])
		if da ~= db then
			return da < db
		end
		return ids[a] < ids[b] -- ties resolve to the lower id
	end)
	local hitSet = { [firstIdx] = true }
	for i = 1, math.min(n - 1, #others) do
		hitSet[others[i]] = true
	end

	local hitCount = 0
	for i = 1, 5 do
		local damaged = s.enemies[i].hp < 1000000
		if hitSet[i] then
			check(damaged, "bounce should have hit enemy " .. i)
			hitCount += 1
		else
			check(not damaged, "bounce should NOT have hit enemy " .. i)
		end
	end
	eq(hitCount, math.min(n, 5), "bounce hits exactly n targets")
	eq(#s.projectiles, 0, "bounce is an instant attack — no projectiles")
end)

test("area and wave are instant, hit by their own radius, and draw no RNG", function()
	local s = blankState()
	mkWeapon(s, W_AREA, 0)
	local wdef = Content.WEAPONS[W_AREA + 1]
	local radius = wdef.attack.a
	-- One inside the area radius (also inside weapon range, so `candidates` is
	-- non-empty and the weapon fires + advances cooldown), one outside it.
	check(radius <= wdef.range, "fixture assumption: area radius <= weapon range")
	local inside = mkEnemy(s, SQUEAK, 1000000, at(radius - 1, 0))
	local outside = mkEnemy(s, SQUEAK, 1000000, at(radius + 10, 0))
	local rngBefore = Rng.state(s.rng_targeting)
	Combat.fireWeapons(s)
	eq(Rng.state(s.rng_targeting), rngBefore, "Area draws no targeting words")
	eq(#s.projectiles, 0, "Area is instant")
	check(s.enemies[1].hp < 1000000, "enemy inside the area was hit")
	eq(s.enemies[2].hp, 1000000, "enemy outside the area was not hit")
	eq(s.weapons[1].next_fire_tick > 0, true, "cooldown advanced")
	eq(inside ~= outside, true, "distinct ids")

	local s2 = blankState()
	mkWeapon(s2, W_WAVE, 0)
	local w2 = Content.WEAPONS[W_WAVE + 1]
	local reach = w2.range + w2.attack.a
	mkEnemy(s2, SQUEAK, 1000000, at(w2.range, 0)) -- in range, inside reach
	mkEnemy(s2, SQUEAK, 1000000, at(reach + 10, 0)) -- outside reach
	local before2 = Rng.state(s2.rng_targeting)
	Combat.fireWeapons(s2)
	eq(Rng.state(s2.rng_targeting), before2, "Wave draws no targeting words")
	check(s2.enemies[1].hp < 1000000, "enemy inside the wave reach was hit")
	eq(s2.enemies[2].hp, 1000000, "enemy beyond reach was not hit")
end)

test("instant attacks reap kills in the same call (bounty recorded)", function()
	local s = blankState()
	mkWeapon(s, W_AREA, 0)
	local wdef = Content.WEAPONS[W_AREA + 1]
	mkEnemy(s, SQUEAK, 1, at(wdef.attack.a - 1, 0))
	Combat.fireWeapons(s)
	eq(#s.enemies, 0, "dead enemy reaped")
	eq(#s.pending_kills, 1, "kill recorded for economy")
	eq(s.pending_kills[1], SQUEAK, "the dead enemy's def")
end)

-- ---------------------------- weapon abilities ----------------------------

test("knockback shoves the damaged enemy away from the tank", function()
	local s = blankState()
	local wdef = Content.WEAPONS[W_KNOCKBACK + 1]
	local dist = wdef.ability.a
	local pos = at(100, 0)
	local eid = mkEnemy(s, SQUEAK, 1000000, pos)
	mkProjectile(s, {
		pos = at(0, 0),
		target = eid,
		last_target_pos = pos,
		damage = 10,
		damage_type = 4,
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(1000),
		ability = wdef.ability,
	})
	Combat.advanceProjectiles(s)
	feq(s.enemies[1].pos.x, Fixed.fromInt(100 + dist), "shoved directly away along +x")
end)

test("hazard ability drops one hazard at the first damaged enemy", function()
	local s = blankState()
	local wdef = Content.WEAPONS[W_HAZARD + 1]
	local ab = wdef.ability
	local impact = at(100, 0)
	local first = mkEnemy(s, SQUEAK, 1000000, impact)
	mkEnemy(s, SQUEAK, 1000000, at(120, 0))
	mkProjectile(s, {
		pos = impact,
		target = first,
		last_target_pos = impact,
		damage = 10,
		damage_type = 4,
		splash_radius = Fixed.fromInt(300),
		speed = Fixed.fromInt(1000),
		ability = ab,
	})
	Combat.advanceProjectiles(s)
	eq(#s.hazards, 1, "exactly one hazard per impact")
	feq(s.hazards[1].pos.x, Fixed.fromInt(100), "dropped at the FIRST damaged enemy")
	eq(s.hazards[1].dmg, ab.a, "hazard dmg")
	eq(s.hazards[1].radius, ab.b, "hazard radius")
	eq(s.hazards[1].ticks_left, ab.c, "hazard lifetime")
	eq(s.hazards[1].damage_type, 4, "hazard inherits the hitting damage type")
end)

test("summon raises an ally only from a killing blow, capped globally", function()
	local s = blankState()
	local wdef = Content.WEAPONS[W_SUMMON + 1]
	local ab = wdef.ability
	mkWeapon(s, W_SUMMON, 0)
	-- Twenty 1-hp enemies inside the area radius: 16 raise, the rest are capped.
	for i = 1, 20 do
		mkEnemy(s, SQUEAK, 1, at(1 + i, 0))
	end
	Combat.fireWeapons(s)
	eq(#s.minions, 16, "MAX_MINIONS caps the raise")
	eq(s.minions[1].kind, ab.a, "minion kind from the ability")
	eq(s.minions[1].hp, ab.b, "minion hp from the ability")
	eq(s.minions[1].damage, ab.c, "minion damage from the ability")
	eq(s.minions[1].damage_type, wdef.damage_type, "minion inherits the killing weapon's type")
	eq(s.minions[1].expire_tick, s.tick + 450, "MINION_LIFETIME")

	-- A non-lethal hit raises nothing.
	local s2 = blankState()
	mkWeapon(s2, W_SUMMON, 0)
	mkEnemy(s2, SQUEAK, 100000000, at(2, 0))
	Combat.fireWeapons(s2)
	eq(#s2.minions, 0, "no corpse, no ally")
end)

test("life drain heals the tank once per damaged enemy", function()
	local s = blankState()
	local wdef = Content.WEAPONS[W_LIFEDRAIN + 1]
	local perHit = wdef.ability.a
	mkWeapon(s, W_LIFEDRAIN, 0)
	s.tank.hp = 1000
	for i = 1, 3 do
		mkEnemy(s, SQUEAK, 100000000, at(10 * i, 0))
	end
	Combat.fireWeapons(s)
	eq(s.tank.hp, 1000 + perHit * 3, "one heal per enemy damaged, flushed once")
end)

-- ============================ advanceProjectiles ============================

test("projectile kills the target and records its def", function()
	local s = blankState()
	local pos = at(10, 0)
	local eid = mkEnemy(s, SQUEAK, 100, pos)
	mkProjectile(s, {
		pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		target = eid,
		last_target_pos = pos,
		damage = 200, -- piercing vs armor 0 -> 2x = 400, lethal
		damage_type = 1,
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(1000),
	})
	Combat.advanceProjectiles(s)
	eq(#s.enemies, 0, "enemy should be dead")
	eq(#s.pending_kills, 1, "one kill recorded")
	eq(s.pending_kills[1], SQUEAK, "dead enemy def pushed")
	eq(#s.projectiles, 0, "arrived projectile removed")
end)

test("in-flight projectile survives and moves by `speed`", function()
	local s = blankState()
	local pos = at(100, 0)
	local eid = mkEnemy(s, SQUEAK, 200, pos)
	mkProjectile(s, {
		pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		target = eid,
		last_target_pos = pos,
		damage = 75,
		damage_type = 1,
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(10),
	})
	Combat.advanceProjectiles(s)
	eq(#s.projectiles, 1, "still in flight")
	eq(#s.enemies, 1, "enemy unharmed yet")
	eq(s.enemies[1].hp, 200, "hp unchanged")
	feq(s.projectiles[1].pos.x, Fixed.fromInt(10), "moved 10 units along +x")
end)

test("splash hits every enemy within the radius of the impact point", function()
	local s = blankState()
	local impact = at(100, 0)
	local near1 = mkEnemy(s, SQUEAK, 100, impact) -- distance 0
	mkEnemy(s, SQUEAK, 100, at(250, 0)) -- 150 away
	mkEnemy(s, SQUEAK, 100, at(1000, 0)) -- 900 away
	mkProjectile(s, {
		pos = impact,
		target = near1,
		last_target_pos = impact,
		damage = 300, -- siege vs armor 0 -> 1x, lethal
		damage_type = 3,
		splash_radius = Fixed.fromInt(300),
		speed = Fixed.fromInt(1000),
	})
	Combat.advanceProjectiles(s)
	eq(#s.enemies, 1, "only the far enemy survives")
	eq(#s.pending_kills, 2, "two kills recorded")
end)

test("splash detonates at last_target_pos when the target is gone", function()
	local s = blankState()
	local impact = at(50, 0)
	mkEnemy(s, SQUEAK, 100, impact) -- a bystander at the last known position
	mkProjectile(s, {
		pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		target = 99999, -- nonexistent
		last_target_pos = impact,
		damage = 300,
		damage_type = 3,
		splash_radius = Fixed.fromInt(300),
		speed = Fixed.fromInt(10),
	})
	Combat.advanceProjectiles(s)
	eq(#s.enemies, 0, "bystander killed by the detonation")
	eq(s.pending_kills[1], SQUEAK, "kill recorded")
	eq(#s.projectiles, 0, "projectile removed")
end)

test("single-target projectile simply vanishes when the target is gone", function()
	local s = blankState()
	mkEnemy(s, SQUEAK, 100, Vec2.new(Fixed.ZERO, Fixed.ZERO))
	mkProjectile(s, {
		pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		target = 99999,
		last_target_pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		damage = 1000,
		damage_type = 1,
		splash_radius = Fixed.ZERO,
		speed = Fixed.fromInt(10),
	})
	Combat.advanceProjectiles(s)
	eq(#s.enemies, 1, "bystander untouched by a single-target miss")
	eq(#s.projectiles, 0, "stale projectile removed")
	eq(#s.pending_kills, 0, "no kill")
end)

test("a surviving projectile tracks its target's last position", function()
	local s = blankState()
	local pos = at(500, 0)
	local eid = mkEnemy(s, SQUEAK, 200, pos)
	mkProjectile(s, {
		pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		target = eid,
		last_target_pos = Vec2.new(Fixed.ZERO, Fixed.ZERO),
		damage = 10,
		damage_type = 1,
		splash_radius = Fixed.fromInt(100),
		speed = Fixed.fromInt(10),
	})
	Combat.advanceProjectiles(s)
	feq(s.projectiles[1].last_target_pos.x, Fixed.fromInt(500), "last_target_pos refreshed")
	-- And it must be a COPY: moving the enemy must not move the stored point.
	s.enemies[1].pos = at(900, 0)
	feq(s.projectiles[1].last_target_pos.x, Fixed.fromInt(500), "stored position does not alias the enemy")
end)

-- ============================ moveEnemies ============================

test("enemy reaching the tank deals contact damage and self-destructs", function()
	local s = blankState()
	local hp0 = s.tank.hp
	mkEnemy(s, SQUEAK, 200, at(3, 0)) -- move_speed 8 >= distance -> arrives
	Combat.moveEnemies(s)
	eq(#s.enemies, 0, "enemy removed on contact")
	eq(s.tank.hp, hp0 - Content.ENEMIES[SQUEAK + 1].contact_damage, "contact_damage applied")
	eq(#s.pending_kills, 0, "self-destruct grants no bounty")
end)

test("enemy marches toward the tank at move_speed", function()
	local s = blankState()
	local hp0 = s.tank.hp
	mkEnemy(s, SQUEAK, 200, at(100, 0))
	Combat.moveEnemies(s)
	eq(#s.enemies, 1, "still marching")
	feq(s.enemies[1].pos.x, Fixed.fromInt(92), "moved 8 units toward the origin")
	eq(s.tank.hp, hp0, "no contact yet")
end)

test("immobile enemies do not move", function()
	local s = blankState()
	mkEnemy(s, SQUEAK, 200, at(100, 0))
	s.enemies[1].status.stun_ticks = 5
	Combat.moveEnemies(s)
	feq(s.enemies[1].pos.x, Fixed.fromInt(100), "stunned enemy is pinned")

	local s2 = blankState()
	mkEnemy(s2, SQUEAK, 200, at(100, 0))
	s2.enemies[1].status.freeze_ticks = 5
	Combat.moveEnemies(s2)
	feq(s2.enemies[1].pos.x, Fixed.fromInt(100), "frozen enemy is pinned")
end)

test("contact damage scales with match time on the enemy_hp_mult curve", function()
	local base = Content.ENEMIES[SQUEAK + 1].contact_damage
	local late = blankState()
	late.tick = 27000 -- SCALE_STEP_2_TICK
	local mult = Content.enemyHpMult(late.tick)
	local expected = Fixed.toInt(Fixed.scaleI64(mult, base))
	check(expected > base, "scaling must increase contact damage")
	local hp0 = late.tank.hp
	mkEnemy(late, SQUEAK, 200, at(3, 0))
	Combat.moveEnemies(late)
	eq(late.tank.hp, hp0 - expected, "late contact damage scaled")
end)

test("boss plants on the tank and grinds only on its cadence", function()
	local cadence = Content.BOSS_CONTACT_CADENCE
	local raw = Content.ENEMIES[BOSS_DEF + 1].contact_damage

	-- Off-cadence tick: boss arrives, plants, deals nothing.
	local off = blankState()
	off.tick = 54001
	check(off.tick % cadence ~= 0, "fixture: off-cadence tick")
	local hpOff = off.tank.hp
	mkEnemy(off, BOSS_DEF, 33000000, at(2, 0))
	Combat.moveEnemies(off)
	eq(#off.enemies, 1, "boss persists on contact (no self-destruct)")
	check(Fixed.eq(off.enemies[1].pos.x, Fixed.ZERO) and Fixed.eq(off.enemies[1].pos.y, Fixed.ZERO), "planted")
	eq(off.tank.hp, hpOff, "no damage on an off-cadence tick")

	-- On-cadence tick: still planted, now lands one scaled hit.
	local on = blankState()
	on.tick = 54000
	check(on.tick % cadence == 0, "fixture: on-cadence tick")
	local expected = Fixed.toInt(Fixed.scaleI64(Content.enemyHpMult(on.tick), raw))
	local hpOn = on.tank.hp
	mkEnemy(on, BOSS_DEF, 33000000, at(2, 0))
	Combat.moveEnemies(on)
	eq(#on.enemies, 1, "boss still present after a contact hit")
	eq(on.tank.hp, hpOn - expected, "cadence tick deals one scaled boss hit")
	eq(#on.pending_kills, 0, "boss contact grants no bounty")
end)

-- ============================ tickHazards ============================

test("hazards pulse enemies in radius, decay, and expire", function()
	local s = blankState()
	local id = allocId(s)
	table.insert(s.hazards, {
		id = id,
		pos = at(0, 0),
		dmg = 100,
		damage_type = 4, -- chaos: 1x
		radius = 200,
		ticks_left = 2,
	})
	mkEnemy(s, SQUEAK, 1000, at(100, 0)) -- inside
	mkEnemy(s, SQUEAK, 1000, at(500, 0)) -- outside
	Combat.tickHazards(s)
	eq(s.enemies[1].hp, 900, "in-radius enemy pulsed")
	eq(s.enemies[2].hp, 1000, "out-of-radius enemy untouched")
	eq(s.hazards[1].ticks_left, 1, "lifetime decremented")
	eq(s.total_damage_dealt, 100, "hazard damage is a player source")

	Combat.tickHazards(s)
	eq(#s.hazards, 0, "hazard expired and was dropped")
	eq(s.enemies[1].hp, 800, "pulsed once more before expiring")
end)

test("hazard kills route through the shared death path", function()
	local s = blankState()
	local id = allocId(s)
	table.insert(s.hazards, {
		id = id,
		pos = at(0, 0),
		dmg = 10000,
		damage_type = 4,
		radius = 200,
		ticks_left = 5,
	})
	mkEnemy(s, SQUEAK, 100, at(50, 0))
	Combat.tickHazards(s)
	eq(#s.enemies, 0, "enemy reaped")
	eq(s.pending_kills[1], SQUEAK, "bounty recorded")
end)

-- ============================ tickAura ============================

test("aura is a no-op when unconfigured", function()
	local s = blankState()
	mkEnemy(s, SQUEAK, 1000, at(10, 0))
	Combat.tickAura(s)
	eq(s.enemies[1].hp, 1000, "no aura, no damage")
	eq(s.tank.aura_tick, 0, "counter untouched")
end)

test("aura fires on its integer cadence, resets, and applies its poison", function()
	local s = blankState()
	s.tank.aura_cadence = 3
	s.tank.aura_damage = 100
	s.tank.aura_range = 200
	s.tank.aura_poison_dps = 7
	s.tank.aura_poison_ticks = 60
	mkEnemy(s, SQUEAK, 100000, at(100, 0)) -- inside
	mkEnemy(s, SQUEAK, 100000, at(500, 0)) -- outside

	Combat.tickAura(s)
	eq(s.tank.aura_tick, 1, "counter advanced")
	eq(s.enemies[1].hp, 100000, "no fire before the cadence")
	Combat.tickAura(s)
	eq(s.tank.aura_tick, 2, "counter advanced")
	Combat.tickAura(s)
	eq(s.tank.aura_tick, 0, "counter reset on fire")
	-- Magic vs Light armor = 1x.
	local want = Fixed.toInt(Fixed.scaleI64(Content.damageMultiplier(Content.DMG_MAGIC, 0), 100))
	eq(s.enemies[1].hp, 100000 - want, "aura damage applied to the in-range enemy")
	eq(s.enemies[2].hp, 100000, "out-of-range enemy untouched")
	eq(s.enemies[1].status.poison_dps, 7, "aura poison applied")
	eq(s.enemies[1].status.poison_ticks, 60, "aura poison duration")
end)

-- ============================ tickMinions ============================

test("minions expire, chase, strike on cooldown, and route kills", function()
	local s = blankState()
	-- An expired minion is dropped before anything else happens.
	table.insert(s.minions, {
		id = allocId(s),
		pos = at(0, 0),
		kind = 0,
		hp = 100,
		damage = 50,
		damage_type = 4,
		next_attack_tick = 0,
		expire_tick = 0,
	})
	s.tick = 0
	Combat.tickMinions(s)
	eq(#s.minions, 0, "minion at expire_tick is removed")

	-- A live minion out of reach steps toward its nearest non-boss target.
	local s2 = blankState()
	table.insert(s2.minions, {
		id = allocId(s2),
		pos = at(0, 0),
		kind = 0,
		hp = 100,
		damage = 50,
		damage_type = 4,
		next_attack_tick = 0,
		expire_tick = 100,
	})
	mkEnemy(s2, BOSS_DEF, 33000000, at(10, 0)) -- boss: never targeted
	mkEnemy(s2, SQUEAK, 1000, at(500, 0)) -- the real target
	Combat.tickMinions(s2)
	feq(s2.minions[1].pos.x, Fixed.fromInt(18), "stepped MINION_SPEED toward the non-boss target")
	eq(s2.enemies[2].hp, 1000, "no strike out of reach")

	-- In reach: strikes on cooldown.
	local s3 = blankState()
	table.insert(s3.minions, {
		id = allocId(s3),
		pos = at(0, 0),
		kind = 0,
		hp = 100,
		damage = 50,
		damage_type = 4, -- chaos: 1x
		next_attack_tick = 0,
		expire_tick = 100,
	})
	mkEnemy(s3, SQUEAK, 1000, at(100, 0)) -- within MINION_REACH 160
	Combat.tickMinions(s3)
	eq(s3.enemies[1].hp, 950, "struck for its damage")
	eq(s3.minions[1].next_attack_tick, 30, "MINION_ATTACK_CD")
	Combat.tickMinions(s3)
	eq(s3.enemies[1].hp, 950, "on cooldown, no second strike")
end)

-- ============================ enemyRangedAttacks ============================

test("ranged enemies fire on their (tick + id) % cooldown phase", function()
	local ab = Content.ENEMIES[CROAK + 1].ability
	eq(ab.tag, 1, "fixture: Croak has RangedAttack")
	local range = ab.a
	local cooldown = ab.b % 4294967296
	local damage = ab.c

	-- In range and on phase -> one hit.
	local s = blankState()
	local eid = mkEnemy(s, CROAK, 300, at(range - 10, 0))
	s.tick = (cooldown - (eid % cooldown)) % cooldown
	local hp0 = s.tank.hp
	Combat.enemyRangedAttacks(s)
	local expected = Fixed.toInt(Fixed.scaleI64(Content.enemyHpMult(s.tick), damage))
	eq(s.tank.hp, hp0 - expected, "on-phase ranged hit lands (scaled)")

	-- Off phase -> nothing.
	local s2 = blankState()
	local eid2 = mkEnemy(s2, CROAK, 300, at(range - 10, 0))
	s2.tick = ((cooldown - (eid2 % cooldown)) % cooldown) + 1
	local hp2 = s2.tank.hp
	Combat.enemyRangedAttacks(s2)
	eq(s2.tank.hp, hp2, "off-phase tick does nothing")

	-- Out of standoff range -> nothing, even on phase.
	local s3 = blankState()
	local eid3 = mkEnemy(s3, CROAK, 300, at(range + 100, 0))
	s3.tick = (cooldown - (eid3 % cooldown)) % cooldown
	local hp3 = s3.tank.hp
	Combat.enemyRangedAttacks(s3)
	eq(s3.tank.hp, hp3, "out of range does nothing")

	-- Immobile -> nothing, even in range and on phase.
	local s4 = blankState()
	local eid4 = mkEnemy(s4, CROAK, 300, at(range - 10, 0))
	s4.enemies[1].status.stun_ticks = 5
	s4.tick = (cooldown - (eid4 % cooldown)) % cooldown
	local hp4 = s4.tank.hp
	Combat.enemyRangedAttacks(s4)
	eq(s4.tank.hp, hp4, "stunned enemy cannot attack")

	-- A plain melee enemy never fires.
	local s5 = blankState()
	mkEnemy(s5, SQUEAK, 200, at(10, 0))
	local hp5 = s5.tank.hp
	Combat.enemyRangedAttacks(s5)
	eq(s5.tank.hp, hp5, "no RangedAttack ability, no standoff fire")
end)

-- ============================ run ============================

print("combat_test: " .. #tests .. " tests")
for _, t in tests do
	currentTest = t.name
	local ok, err = pcall(t.fn)
	if not ok then
		failed += 1
		print(string.format("  ERROR [%s] %s", t.name, tostring(err)))
	end
end
print(string.format("combat_test: %d checks passed, %d failed", passed, failed))
if failed > 0 then
	error("combat_test FAILED", 0)
end
