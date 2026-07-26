--[[
	client_test.luau — headless oracle for `src/client/**` (R4, `docs/10` F1/F2/F4).

	WHAT THIS COVERS AND WHY THAT IS THE INTERESTING PART
	------------------------------------------------------
	The client is written as a pure core plus a thin Roblox drawing shell, so
	almost everything that can be *wrong* is reachable from a bare interpreter:

	  * `Hud`      — every number→string conversion the player reads, plus the
	                 round clock and boss countdown derived from a raw tick count.
	  * `ShopUi`   — the shop-slot → input-code mapping. `SIM-SPEC` §S6 records
	                 that two agents once picked opposite byte orders and cost a
	                 diverge-at-tick-0, so this file asserts the packing against
	                 BOTH literal integers and `sim/Input.luau`'s own decode, in
	                 both directions, including the u8 boundary.
	  * `RunFlow`  — the F2 instant-retry machine. The load-bearing assertion is
	                 that with ZERO player input a retry is requested inside
	                 `RunFlow.MAX_DEAD_TIME` of the death, and that no reachable
	                 phase waits on another player.
	  * `Render`   — interpolation (id matching, the index-match jump guard,
	                 out-of-order drops) and the world→screen layout, plus the
	                 pooling arithmetic that backs the "no `Instance` per enemy"
	                 claim.

	It also runs the four modules against a `RenderView` produced by the REAL sim
	(`State.new` + `Arena.step` + `View.snapshot`), not only against hand-built
	fixtures, so contract drift in `View.luau` fails here rather than in Studio.

	And it asserts the P5 property directly: none of the client modules mutates
	the view it is handed.

	HOW TO RUN
	----------
	No input data; from `roblox/`:

	    luau test/client_test.luau

	Exits non-zero on failure.
]]

local Render = require("../src/client/Render")
local Hud = require("../src/client/Hud")
local ShopUi = require("../src/client/ShopUi")
local RunFlow = require("../src/client/RunFlow")
local Env = require("../src/client/RobloxEnv")

local Input = require("../src/shared/sim/Input")
local State = require("../src/shared/sim/State")
local Arena = require("../src/shared/sim/Arena")
local ViewMod = require("../src/shared/sim/View")
local Content = require("../src/shared/Content")

--=============================================================================
-- harness (same shape as the other suites in this directory)
--=============================================================================

local failures = 0
local checks = 0
local group = "?"

local function G(name)
	group = name
end

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

local function near(what, got, want, tol)
	checks += 1
	if type(got) ~= "number" or math.abs(got - want) > (tol or 1e-6) then
		fail(what, got, want)
	end
end

local function truthy(what, got)
	checks += 1
	if not got then
		fail(what, tostring(got), "truthy")
	end
end

local function falsy(what, got)
	checks += 1
	if got then
		fail(what, tostring(got), "falsy")
	end
end

local function raises(what, fn, ...)
	checks += 1
	local ok = pcall(fn, ...)
	if ok then
		fail(what, "no error", "error")
	end
end

--- Structural deep compare, used to prove the client never writes to a view.
local function deepEqual(a, b): boolean
	if type(a) ~= type(b) then
		return false
	end
	if type(a) ~= "table" then
		return a == b
	end
	for k, v in a do
		if not deepEqual(v, b[k]) then
			return false
		end
	end
	for k in b do
		if a[k] == nil then
			return false
		end
	end
	return true
end

local function deepCopy(v)
	if type(v) ~= "table" then
		return v
	end
	local out = {}
	for k, x in v do
		out[k] = deepCopy(x)
	end
	return out
end

--=============================================================================
-- synthetic RenderView fixture
--=============================================================================

--- A hand-built `RenderView` with every field `View.luau` declares. Built by
--- hand (rather than only from the sim) so a test can put a specific number in a
--- specific field and assert the exact string the player sees.
local function fixture(over)
	local v = {
		tick = 1234,
		round = 3,
		dead = false,
		tank = { x = 0, y = 0, hp = 1240, max_hp = 2000, revives = 1 },
		enemies = {
			{ id = 7, x = 100, y = 0, hp = 50, base_hp = 100, kind = 0, boss = false, name = "Squeakzilla" },
			{ id = 9, x = -400, y = 200, hp = 900, base_hp = 900, kind = 6, boss = false, name = "Bonk" },
			{
				id = 11,
				x = 0,
				y = 1700,
				hp = 3e7,
				base_hp = 3.3e7,
				kind = 2,
				boss = true,
				name = "The Hippocrate",
			},
		},
		projectiles = { { x = 10, y = 10 }, { x = 0, y = 0 } },
		minions = { { x = 40, y = -40, kind = 0 }, { x = -40, y = 40, kind = 1 } },
		economy = { gold = 12345, income_per_tick = 4, rerolls_remaining = 5, reroll_cost = 100 },
		shop = {
			{ slot = 0, name = "Ol' Reliable", cost = 500, is_weapon = true, affordable = true, rarity = 0 },
			{ slot = 1, name = "Boomstick", cost = 1500, is_weapon = true, affordable = true, rarity = 1 },
			{ slot = 2, name = "Death Engine", cost = 90000, is_weapon = true, affordable = false, rarity = 3 },
			{ slot = 3, name = "Sidearm", cost = 500, is_weapon = true, affordable = true, rarity = 0 },
			{ slot = 4, name = "Big Wallet", cost = 750, is_weapon = false, affordable = true, rarity = 0 },
			{ slot = 5, name = "Spiked Hull", cost = 2000, is_weapon = false, affordable = true, rarity = 2 },
			{ slot = 6, name = "Thick Plate", cost = 500, is_weapon = false, affordable = true, rarity = 0 },
			{ slot = 7, name = "Overclock", cost = 40000, is_weapon = false, affordable = false, rarity = 3 },
		},
		arsenal = { { name = "Boomstick", count = 3 }, { name = "Ol' Reliable", count = 1 } },
		stats = {
			damage_dealt = 33500000,
			gold_earned = 84210,
			bought_attack_mask = 5,
			weapons_bought = 4,
			economy_purchases = 2,
		},
	}
	if over then
		for k, x in over do
			v[k] = x
		end
	end
	return v
end

--- The fork's timeline (F1). Used explicitly so the assertions below do not
--- depend on which timeline `Hud` defaults to.
local ROBLOX_TL = Content.TIMELINE.roblox
local STEAM_TL = Content.TIMELINE.steam

--=============================================================================

print("client_test: starting")

--=============================================================================
G("env: the client tree loads with no Roblox host")
--=============================================================================

falsy("RobloxEnv reports no host under a bare interpreter", Env.available)
eq("Render.new returns nil off-Roblox", Render.new(nil), nil)
eq("Hud.new returns nil off-Roblox", Hud.new(nil), nil)
eq("ShopUi.new returns nil off-Roblox", ShopUi.new(nil, function() end), nil)
eq("RunFlow.newScreen returns nil off-Roblox", RunFlow.newScreen(nil, function() end), nil)

--=============================================================================
G("hud: integer grouping")
--=============================================================================

eq("0", Hud.formatInt(0), "0")
eq("7", Hud.formatInt(7), "7")
eq("999", Hud.formatInt(999), "999")
eq("1000", Hud.formatInt(1000), "1,000")
eq("12345", Hud.formatInt(12345), "12,345")
eq("100000", Hud.formatInt(100000), "100,000")
eq("1234567", Hud.formatInt(1234567), "1,234,567")
eq("negative", Hud.formatInt(-4500), "-4,500")

--=============================================================================
G("hud: compact magnitudes")
--=============================================================================

eq("below 1k is exact", Hud.formatCompact(999), "999")
eq("exactly 1k", Hud.formatCompact(1000), "1.0k")
eq("1234 -> 1.2k", Hud.formatCompact(1234), "1.2k")
eq("truncates, never rounds up past the unit", Hud.formatCompact(1999), "1.9k")
eq("99999 -> 99.9k", Hud.formatCompact(99999), "99.9k")
eq("100k drops the decimal", Hud.formatCompact(100000), "100k")
eq("33.5M", Hud.formatCompact(33500000), "33.5M")
eq("billions", Hud.formatCompact(2400000000), "2.4B")
eq("zero", Hud.formatCompact(0), "0")

--=============================================================================
G("hud: clock")
--=============================================================================

eq("tick 0", Hud.formatClock(0, 30), "0:00")
eq("29 ticks is still 0:00", Hud.formatClock(29, 30), "0:00")
eq("30 ticks is one second", Hud.formatClock(30, 30), "0:01")
eq("one minute", Hud.formatClock(1800, 30), "1:00")
eq("the F1 boss tick is five minutes", Hud.formatClock(ROBLOX_TL.boss_spawn_tick, 30), "5:00")
eq("the Steam boss tick is thirty minutes", Hud.formatClock(STEAM_TL.boss_spawn_tick, 30), "30:00")
eq("past an hour", Hud.formatClock(30 * 3661, 30), "1:01:01")
eq("negative clamps to zero", Hud.formatClock(-90, 30), "0:00")

--=============================================================================
G("hud: score is Godot's log compression, verbatim")
--=============================================================================

eq("no damage", Hud.score(0), 0)
eq("sub-1 damage short-circuits", Hud.score(0.4), 0)
eq("1 damage", Hud.score(1), math.round(30 * math.log(2, 10)))
eq("1e6 damage", Hud.score(1000000), math.round(30 * math.log(1000001, 10)))
truthy("score is monotonic in damage", Hud.score(1e7) > Hud.score(1e6))
truthy("score stays a two-digit-ish figure at end-of-run damage", Hud.score(3.35e7) < 250)

--=============================================================================
G("hud: model")
--=============================================================================

local hm = Hud.model(fixture(), { timeline = ROBLOX_TL })

eq("hp text", hm.hp.text, "1,240 / 2,000")
near("hp fraction", hm.hp.frac, 0.62)
eq("gold text", hm.gold.text, "12,345g")
eq("income is per-tick scaled by tick_hz", hm.income.per_second, 4 * 30)
eq("income text", hm.income.text, "+120g/s")
eq("round text", hm.round.text, "ROUND 3")
eq("revives text", hm.revives.text, "REVIVES 1")
eq("reroll text", hm.rerolls.text, "REROLL 5 left · 100g")
truthy("reroll affordable at 12345 gold", hm.rerolls.affordable)
eq("elapsed", hm.timer.elapsed_text, Hud.formatClock(1234, 30))
-- tick 1234 with 600-tick rounds: 1234 % 600 = 34 into the round, 566 left.
eq("round remaining ticks (F1 600-tick round)", hm.timer.round_remaining_ticks, 566)
eq("boss countdown ticks", hm.timer.boss_in_ticks, ROBLOX_TL.boss_spawn_tick - 1234)
falsy("boss not yet active", hm.timer.boss_active)
eq("arsenal row with a count", hm.arsenal[1].text, "Boomstick ×3")
eq("arsenal row with one copy drops the multiplier", hm.arsenal[2].text, "Ol' Reliable")
eq("damage text", hm.stats.damage_text, "33.5M")
eq("earned text", hm.stats.gold_earned_text, "84.2k")
eq("population", hm.population.text, "3 enemies")

-- The SAME tick reads differently under the Steam timeline: 900-tick rounds.
local hmSteam = Hud.model(fixture(), { timeline = STEAM_TL })
eq("round remaining under the Steam timeline", hmSteam.timer.round_remaining_ticks, 900 - (1234 % 900))
truthy("boss countdown differs between timelines", hmSteam.timer.boss_in_ticks ~= hm.timer.boss_in_ticks)

-- Boss-active and dead paths.
local hmBoss = Hud.model(fixture({ tick = ROBLOX_TL.boss_spawn_tick + 10 }), { timeline = ROBLOX_TL })
truthy("boss active past the spawn tick", hmBoss.timer.boss_active)
eq("boss label", hmBoss.timer.boss_in_text, "BOSS")
eq("boss countdown clamps at zero", hmBoss.timer.boss_in_ticks, 0)

local hmDead = Hud.model(
	fixture({ dead = true, tank = { x = 0, y = 0, hp = -30, max_hp = 2000, revives = 0 } }),
	{ timeline = ROBLOX_TL }
)
truthy("dead flag carried", hmDead.dead)
eq("negative hp clamps to zero", hmDead.hp.current, 0)
near("dead hp fraction", hmDead.hp.frac, 0)
eq("no revives means no revive line", hmDead.revives.text, "")

--=============================================================================
G("shop: the SIM-SPEC S6 input encoding")
--=============================================================================

-- Literal expectations. `kind` is in the HIGH byte; this is the pinned order.
eq("Noop code", ShopUi.NOOP_CODE, 0)
eq("Reroll code", ShopUi.REROLL_CODE, 512)
eq("Clear code", ShopUi.CLEAR_CODE, 768)
eq("buy slot 0", ShopUi.buyCode(0), 256)
eq("buy slot 1", ShopUi.buyCode(1), 257)
eq("buy slot 3", ShopUi.buyCode(3), 259)
eq("buy slot 7", ShopUi.buyCode(7), 263)
eq("buy the u8 boundary slot", ShopUi.buyCode(255), 511)

-- Against `sim/Input.luau`, which owns the packing.
for slot = 0, 255 do
	if ShopUi.buyCode(slot) ~= Input.buyOffer(slot) then
		fail("buyCode agrees with Input.buyOffer", slot, "agreement")
		break
	end
end
checks += 1

-- Round-trip through the decode, for every kind.
local function roundTrip(what, code, wantKind, wantSlot)
	local k, s = ShopUi.decode(code)
	eq(what .. " kind", k, wantKind)
	eq(what .. " slot", s, wantSlot)
end
roundTrip("Noop", ShopUi.NOOP_CODE, Input.Kind.Noop, 0)
roundTrip("Reroll", ShopUi.REROLL_CODE, Input.Kind.Reroll, 0)
roundTrip("Clear", ShopUi.CLEAR_CODE, Input.Kind.Clear, 0)
roundTrip("BuyOffer(0)", ShopUi.buyCode(0), Input.Kind.BuyOffer, 0)
roundTrip("BuyOffer(7)", ShopUi.buyCode(7), Input.Kind.BuyOffer, 7)
roundTrip("BuyOffer(255)", ShopUi.buyCode(255), Input.Kind.BuyOffer, 255)

-- The failure mode the low-byte convention would produce: a slot number that
-- collides with another action's code. Under the pinned order it cannot.
truthy("no buy code collides with Reroll", ShopUi.buyCode(0) ~= ShopUi.REROLL_CODE)
truthy("no buy code collides with Clear", ShopUi.buyCode(255) ~= ShopUi.CLEAR_CODE)
truthy("every buy code is above every payload-free code", ShopUi.buyCode(0) > ShopUi.NOOP_CODE)

-- Out-of-range slots must not silently produce a valid code for another action.
raises("slot 256 is rejected", ShopUi.buyCode, 256)
raises("negative slot is rejected", ShopUi.buyCode, -1)
raises("fractional slot is rejected", ShopUi.buyCode, 1.5)

--=============================================================================
G("shop: model and the slot -> code mapping through the offer grid")
--=============================================================================

local sm = ShopUi.model(fixture())

eq("eight offers", #sm.offers, 8)
eq("gold text", sm.gold_text, "12,345g")
for i = 1, 8 do
	local o = sm.offers[i]
	eq(string.format("offer %d carries its 0-based slot", i), o.slot, i - 1)
	eq(string.format("offer %d code", i), o.code, 256 + (i - 1))
end
eq("first offer name", sm.offers[1].name, "Ol' Reliable")
eq("first offer cost text", sm.offers[1].cost_text, "500g")
eq("rarity name common", sm.offers[1].rarity_name, "COMMON")
eq("rarity name epic", sm.offers[3].rarity_name, "EPIC")
eq("weapon tag", sm.offers[1].kind_text, "WEAPON")
eq("modifier tag", sm.offers[5].kind_text, "MODIFIER")
truthy("affordable offer is enabled", sm.offers[1].enabled)
falsy("unaffordable offer is disabled", sm.offers[3].enabled)
eq("reroll label", sm.reroll.label, "REROLL  100g  (5 left)")
eq("clear code", sm.clear.code, 768)
falsy("clear cooldown is NOT knowable from a RenderView", sm.clear.cooldown_known)

-- `press` is the only producer of codes, and it produces integers only.
eq("press buy slot 0", ShopUi.press(sm, "buy", 0), 256)
eq("press buy slot 6", ShopUi.press(sm, "buy", 6), 262)
eq("press reroll", ShopUi.press(sm, "reroll"), 512)
eq("press clear", ShopUi.press(sm, "clear"), 768)
eq("press an unaffordable slot emits nothing", ShopUi.press(sm, "buy", 2), nil)
eq("press a slot that is not on the grid emits nothing", ShopUi.press(sm, "buy", 99), nil)
eq("press buy with no slot emits nothing", ShopUi.press(sm, "buy"), nil)
eq("press an unknown action emits nothing", ShopUi.press(sm, "nope" :: any), nil)
eq("press type", type(ShopUi.press(sm, "reroll")), "number")

-- Dead / broke arenas disable everything. Still only ever hints (P5).
local smDead = ShopUi.model(fixture({ dead = true }))
eq("dead arena disables buy", ShopUi.press(smDead, "buy", 0), nil)
eq("dead arena disables reroll", ShopUi.press(smDead, "reroll"), nil)
eq("dead arena disables clear", ShopUi.press(smDead, "clear"), nil)

local smNoRerolls = ShopUi.model(fixture({
	economy = { gold = 12345, income_per_tick = 4, rerolls_remaining = 0, reroll_cost = 100 },
}))
eq("no rerolls left disables reroll", ShopUi.press(smNoRerolls, "reroll"), nil)

local smBroke = ShopUi.model(fixture({
	economy = { gold = 10, income_per_tick = 4, rerolls_remaining = 5, reroll_cost = 100 },
}))
eq("cannot afford a reroll", ShopUi.press(smBroke, "reroll"), nil)

--=============================================================================
G("authority: no client module writes to the view (P5)")
--=============================================================================

local v = fixture()
local before = deepCopy(v)
Hud.model(v, { timeline = ROBLOX_TL })
ShopUi.model(v)
local probeInterp = Render.newInterp(0.1)
probeInterp:push(v, 0)
local probeSample = probeInterp:sample(0.1)
Render.layout(probeSample, { x = 0, y = 0, w = 800, h = 600 })
RunFlow.new():observe(v, 0)
truthy("the view is byte-identical after every client module has read it", deepEqual(v, before))

--=============================================================================
G("render: interpolation")
--=============================================================================

local a = fixture({ tick = 100 })
local b = fixture({ tick = 105 })
-- Enemy 7 walks 0 -> 100 on x; enemy 9 is unchanged; enemy 11 leaves and a new
-- enemy 13 arrives, so it has no previous position to lerp from.
a.enemies[1].x = 0
b.enemies[1].x = 100
b.enemies[3] = { id = 13, x = 500, y = 500, hp = 10, base_hp = 10, kind = 1, boss = false, name = "Fanged Death" }

local it = Render.newInterp(0.1)
eq("sampling an empty buffer yields nothing", it:sample(0), nil)

it:push(a, 0)
local s0 = it:sample(0)
truthy("one snapshot is enough to draw", s0 ~= nil)
eq("alpha is 1 with only one snapshot", s0.alpha, 1)
eq("single-snapshot position is verbatim", s0.enemies[1].x, 0)

it:push(b, 0.1)
-- render time = now - delay = 0.15 - 0.1 = 0.05, halfway between t=0 and t=0.1.
local s1 = it:sample(0.15)
near("alpha at the midpoint", s1.alpha, 0.5)
eq("enemies are matched by id, not by index", s1.enemies[1].id, 7)
near("matched enemy is halfway", s1.enemies[1].x, 50)
near("unmoved enemy stays put", s1.enemies[2].x, -400)
eq("the entity set comes from the NEWEST snapshot", #s1.enemies, 3)
eq("a newly-arrived enemy is present", s1.enemies[3].id, 13)
near("a newly-arrived enemy is not lerped from a stranger", s1.enemies[3].x, 500)
eq("discrete state is read from the newest snapshot, never blended", s1.view.tick, 105)

-- Clamping: before the first sample time and long after the last.
near("alpha clamps to 0 before the window", it:sample(0.05).alpha, 0)
near("alpha clamps to 1 after the window", it:sample(10).alpha, 1)

-- Out-of-order arrivals must not drag entities backwards.
local c = fixture({ tick = 102 })
c.enemies[1].x = -9999
it:push(c, 0.05)
eq("a late snapshot is dropped", it:sample(10).view.tick, 105)

-- The index-match jump guard for id-less projectiles.
local p0 = fixture()
local p1 = fixture()
p0.projectiles = { { x = 0, y = 0 }, { x = 0, y = 0 } }
p1.projectiles = { { x = 100, y = 0 }, { x = 5000, y = 0 } }
local it2 = Render.newInterp(0)
it2:push(p0, 0)
it2:push(p1, 1)
local sp = it2:sample(0.5)
near("a plausible index match is interpolated", sp.projectiles[1].x, 50)
near("an implausible index match snaps instead of lerping", sp.projectiles[2].x, 5000)

-- Minions additionally require the kind to match before interpolating.
local m0 = fixture()
local m1 = fixture()
m0.minions = { { x = 0, y = 0, kind = 0 } }
m1.minions = { { x = 100, y = 0, kind = 1 } }
local it3 = Render.newInterp(0)
it3:push(m0, 0)
it3:push(m1, 1)
near("a minion of a different kind is not interpolated", it3:sample(0.5).minions[1].x, 100)

-- Reset clears the buffer so a new run never lerps out of the old one.
it:reset()
eq("reset empties the buffer", it:sample(10), nil)

--=============================================================================
G("render: layout (world -> screen)")
--=============================================================================

local lv = fixture()
lv.enemies = {
	{ id = 1, x = 0, y = 0, hp = 1, base_hp = 1, kind = 0, boss = false, name = "a" },
	{ id = 2, x = Render.WORLD_REFERENCE_RADIUS, y = 0, hp = 1, base_hp = 1, kind = 0, boss = false, name = "b" },
	{ id = 3, x = 0, y = Render.WORLD_REFERENCE_RADIUS, hp = 1, base_hp = 1, kind = 0, boss = false, name = "c" },
	{ id = 4, x = 0, y = 0, hp = 1, base_hp = 1, kind = 2, boss = true, name = "boss" },
	{ id = 5, x = 0, y = 0, hp = 1, base_hp = 1, kind = 6, boss = false, name = "heavy" },
	{ id = 6, x = 500000, y = 0, hp = 1, base_hp = 1, kind = 0, boss = false, name = "offscreen" },
}
local li = Render.newInterp(0)
li:push(lv, 0)
local plan = Render.layout(li:sample(0), { x = 0, y = 0, w = 800, h = 600 })

local vpShort = 600
near("scale is Godot's min(w,h)*FIT/reference", plan.scale, vpShort * Render.FIT / Render.WORLD_REFERENCE_RADIUS)
near("centre x", plan.cx, 400)
near("centre y", plan.cy, 300)
near("the origin maps to the centre", plan.enemies[1].x, 400)
near("the origin maps to the centre (y)", plan.enemies[1].y, 300)
near("+x at the reference radius sits FIT of the short side out", plan.enemies[2].x, 400 + vpShort * Render.FIT)
near("+y is FLIPPED for the screen", plan.enemies[3].y, 300 - vpShort * Render.FIT)
eq("far-off-screen entities are culled", #plan.enemies, 5)
eq("but the pre-cull count is reported", plan.counts.enemies, 6)
truthy("the boss marker is the largest", plan.enemies[4].size > plan.enemies[1].size)
truthy("the heavy marker is bigger than a grunt", plan.enemies[5].size > plan.enemies[1].size)
truthy("the boss marker is bigger than the heavy", plan.enemies[4].size > plan.enemies[5].size)
near("hp bar fill matches the tank", plan.hpBar.fill, 1240 / 2000)
falsy("a live arena draws no dead veil", plan.dead)
eq("ui scale is clamped at the low end for a tiny viewport", Render.layout(li:sample(0), {
	x = 0,
	y = 0,
	w = 100,
	h = 80,
}).uiScale, Render.UI_SCALE_MIN)
eq("ui scale is clamped at the high end for a huge viewport", Render.layout(li:sample(0), {
	x = 0,
	y = 0,
	w = 4000,
	h = 4000,
}).uiScale, Render.UI_SCALE_MAX)

local deadPlan = Render.layout(
	(function()
		local di = Render.newInterp(0)
		di:push(fixture({ dead = true }), 0)
		return di:sample(0)
	end)(),
	{ x = 0, y = 0, w = 800, h = 600 }
)
truthy("a dead arena draws the veil", deadPlan.dead)

-- An offset viewport (the F4 spectator panel) shifts everything, nothing else.
local offset = Render.layout(li:sample(0), { x = 900, y = 40, w = 300, h = 300 })
near("an offset viewport re-centres", offset.cx, 1050)
near("an offset viewport re-centres (y)", offset.cy, 190)

--=============================================================================
G("render: pooling arithmetic — no Instance per enemy")
--=============================================================================

local LIMIT = Render.POOL_LIMITS.enemies
local poolSize = 0
local created = 0
local peak = 0
-- A run's worth of wildly varying enemy counts, including a spike well past the
-- ceiling and a long tail near zero.
for _, count in { 0, 5, 40, 12, 67, 3, 67, 0, 400, 66, 1, 67 } do
	local want, grow = Render.poolFit(count, poolSize, LIMIT)
	truthy("want never exceeds the pool ceiling", want <= LIMIT)
	truthy("want never exceeds demand", want <= math.max(count, 0))
	truthy("grow is never negative", grow >= 0)
	poolSize += grow
	created += grow
	peak = math.max(peak, count)
	truthy("the pool never shrinks below what is wanted", poolSize >= want)
	truthy("the pool never exceeds the ceiling", poolSize <= LIMIT)
end
eq("total instances created equals the high-water mark, not the sum of demand", created, LIMIT)
eq("a spike above the ceiling is capped, not obeyed", poolSize, LIMIT)
-- The property that matters: once warm, steady-state frames create nothing.
local _, growAgain = Render.poolFit(67, poolSize, LIMIT)
eq("a warm pool creates nothing for a typical frame", growAgain, 0)
local _, growPeak = Render.poolFit(LIMIT, poolSize, LIMIT)
eq("a warm pool creates nothing even at peak", growPeak, 0)

--=============================================================================
G("runflow: the F2 instant-retry machine")
--=============================================================================

local flow = RunFlow.new()
eq("starts booting", flow:phase(), "booting")
eq("no runs yet", flow:runs(), 0)

local ev = flow:observe(fixture({ tick = 0 }), 0)
eq("first live view starts a run", ev[1].kind, "run_started")
eq("phase running", flow:phase(), "running")
eq("run counted", flow:runs(), 1)

-- Alive ticks produce nothing.
eq("a live view is uneventful", #flow:observe(fixture({ tick = 30 }), 1), 0)
eq("timers are uneventful while alive", #flow:update(1), 0)
eq("deadFor is zero while alive", flow:deadFor(1), 0)
eq("countdown is zero while alive", flow:countdown(1), 0)
eq("retryNow is ignored while alive", #flow:retryNow(1), 0)

-- Death.
local deathView = fixture({ tick = 600, dead = true, round = 4 })
local dev = flow:observe(deathView, 10)
eq("death ends the run", dev[1].kind, "run_ended")
eq("phase result", flow:phase(), "result")
truthy("a result was captured", flow:result() ~= nil)
eq("result round", flow:result().round, 4)
eq("result ticks", flow:result().ticks, 600)
eq("result score matches the HUD's", flow:result().score, Hud.score(deathView.stats.damage_dealt))
eq("result damage text matches the HUD's", flow:result().damage_text, Hud.formatCompact(33500000))

-- THE F2 GUARANTEE: with zero player input, a retry is requested inside the
-- dead-time budget. This is the assertion that stops anyone quietly turning the
-- results screen into an elimination lobby.
truthy("the auto-retry fires before the F2 budget", RunFlow.DEFAULT_AUTO_RETRY < RunFlow.MAX_DEAD_TIME)
local emitted = nil
local t = 10
while t < 10 + RunFlow.MAX_DEAD_TIME do
	t += 1 / 60
	local evs = flow:update(t)
	if #evs > 0 then
		emitted = { t = t, ev = evs[1] }
		break
	end
end
truthy("a retry was requested with no player input at all", emitted ~= nil)
eq("and it is a retry request", emitted.ev.kind, "request_retry")
eq("first attempt", emitted.ev.attempt, 1)
truthy("within the F2 dead-time budget", emitted.t - 10 <= RunFlow.MAX_DEAD_TIME)
eq("phase retrying", flow:phase(), "retrying")
eq("countdown is zero once retrying", flow:countdown(emitted.t), 0)

-- Still dead views while retrying keep us retrying, and do NOT re-fire.
eq("a dead view during retry is uneventful", #flow:observe(fixture({ tick = 700, dead = true }), emitted.t), 0)
eq("still retrying", flow:phase(), "retrying")

-- The server hands back a fresh arena.
local startEv = flow:observe(fixture({ tick = 0 }), emitted.t + 0.4)
eq("a live arena starts the next run", startEv[1].kind, "run_started")
eq("phase running again", flow:phase(), "running")
eq("two runs now", flow:runs(), 2)
eq("the stale result is cleared", flow:result(), nil)
eq("attempts reset", flow:attempts(), 0)

--=============================================================================
G("runflow: manual retry, timeout re-request, and tick-regression detection")
--=============================================================================

local f2 = RunFlow.new({ autoRetrySeconds = 2.4, retryTimeoutSeconds = 2.0 })
f2:observe(fixture({ tick = 0 }), 0)
f2:observe(fixture({ tick = 900, dead = true }), 5)
eq("in results", f2:phase(), "result")
near("countdown starts at the auto-retry delay", f2:countdown(5), 2.4)
near("countdown decays", f2:countdown(6), 1.4)

local manual = f2:retryNow(5.2)
eq("a manual retry fires immediately", manual[1].kind, "request_retry")
eq("manual retry is attempt 1", manual[1].attempt, 1)
eq("phase retrying", f2:phase(), "retrying")
near("the player waited a fifth of a second, not 2.4", f2:deadFor(5.2), 0.2)

-- A dropped request must not strand the player: after the timeout, ask again.
eq("no re-request before the timeout", #f2:update(7.0), 0)
local again = f2:update(7.3)
eq("a re-request is emitted after the timeout", again[1].kind, "request_retry")
eq("attempt 2", again[1].attempt, 2)
local again3 = f2:update(9.4)
eq("and keeps re-requesting", again3[1].attempt, 3)

-- Tick regression is the second new-run signal (a replacement arena).
local f3 = RunFlow.new()
f3:observe(fixture({ tick = 5000 }), 0)
eq("running", f3:phase(), "running")
local regressed = f3:observe(fixture({ tick = 3 }), 1)
eq("a tick that went backwards is a new run", regressed[1].kind, "run_started")
eq("two runs", f3:runs(), 2)

-- Booting straight into a dead arena must show results, not hang.
local f4 = RunFlow.new()
local bootDead = f4:observe(fixture({ tick = 4000, dead = true }), 0)
eq("booting into a dead arena goes straight to results", bootDead[1].kind, "run_ended")
eq("phase result", f4:phase(), "result")
truthy("and still auto-retries", #f4:update(RunFlow.MAX_DEAD_TIME) > 0)

-- Reset returns to booting.
f4:reset()
eq("reset returns to booting", f4:phase(), "booting")
eq("reset clears the result", f4:result(), nil)

--=============================================================================
G("integration: the real sim's RenderView, not a fixture")
--=============================================================================

-- Drive the actual simulation and feed its own `View.snapshot` through every
-- client module. This is what catches drift in `View.luau` before Studio does.
local st = State.new("0xdeadbeefcafef00d", 0)
for _ = 1, 400 do
	Arena.step(st, Input.NOOP)
end
local realView = ViewMod.snapshot(st)

checks += 1
local okHud, hudModel = pcall(Hud.model, realView, { timeline = ROBLOX_TL })
if not okHud then
	fail("Hud.model on a real snapshot", tostring(hudModel), "no error")
else
	eq("real tick is reflected", hudModel.tick, realView.tick)
	eq("real round is reflected", hudModel.round.text, string.format("ROUND %d", realView.round))
	truthy("hp text is non-empty", #hudModel.hp.text > 0)
	truthy("gold text is non-empty", #hudModel.gold.text > 0)
end

checks += 1
local okShop, shopModel = pcall(ShopUi.model, realView)
if not okShop then
	fail("ShopUi.model on a real snapshot", tostring(shopModel), "no error")
else
	eq("the real shop has the 4+4 layout", #shopModel.offers, 8)
	for i = 1, #shopModel.offers do
		local o = shopModel.offers[i]
		eq(string.format("real offer %d slot is 0-based", i), o.slot, i - 1)
		eq(string.format("real offer %d code decodes to BuyOffer", i), (select(1, ShopUi.decode(o.code))), Input.Kind.BuyOffer)
		eq(string.format("real offer %d code decodes to its own slot", i), (select(2, ShopUi.decode(o.code))), o.slot)
		truthy(string.format("real offer %d has a name", i), #o.name > 0)
	end
end

checks += 1
local okRender, renderErr = pcall(function()
	local ri = Render.newInterp(0.1)
	ri:push(realView, 0)
	Arena.step(st, Input.NOOP)
	Arena.step(st, Input.NOOP)
	ri:push(ViewMod.snapshot(st), 0.1)
	local sampled = ri:sample(0.15)
	local p = Render.layout(sampled, { x = 0, y = 0, w = 1280, h = 720 })
	assert(p.counts.enemies == #sampled.enemies, "layout counts must match the sample")
	assert(#p.enemies <= p.counts.enemies, "culling can only remove")
end)
if not okRender then
	fail("Render on a real snapshot", tostring(renderErr), "no error")
end

checks += 1
local okFlow, flowErr = pcall(function()
	local rf = RunFlow.new()
	local evs = rf:observe(realView, 0)
	assert(evs[1] ~= nil and evs[1].kind == "run_started", "a real live view must start a run")
end)
if not okFlow then
	fail("RunFlow on a real snapshot", tostring(flowErr), "no error")
end

--=============================================================================

print(string.format("client_test: %d checks, %d failures", checks, failures))
if failures > 0 then
	error(string.format("client_test FAILED (%d)", failures), 0)
end
print("client_test: OK")
