--!nonstrict
--[[
	meta_test.luau — `src/server/Profile.luau` + `src/server/Meta.luau`
	(`docs/10` F3, R6).

	HOW TO RUN. From `roblox/test`:

	    luau meta_test.luau

	No arguments, no file I/O: both modules are pure (no Roblox globals), which is
	the whole point of the adapter seam — the DataStore never enters this process.

	WHAT IT CHECKS
	  1. XP: the run->XP arithmetic, its cap, and `runResultFromState` against a
	     real `ArenaState` (including the `U32_MAX` pre-first-round sentinel).
	  2. Unlock thresholds: the schedule covers the catalog exactly once, the
	     starter set is free, crossing a threshold grants exactly the ids at that
	     threshold and nothing else, and the pool is monotone in XP.
	  3. Pool restriction, fresh vs mature vs maxed.
	  4. THE LOAD-BEARING ONE: a restricted pool never yields a locked id. Driven
	     through the exact draw the documented `Shop.setPool` hook performs, over
	     many seeds and rerolls.
	  5. That the same draw with an UNRESTRICTED pool reproduces the real
	     `Shop.generateOffers` offer-for-offer AND leaves `rng_shop` on the same
	     cursor — which is what makes the wiring instruction safe to apply to
	     verified sim code.
	  6. Chassis application against a real `State.new` arena: weapon replaced,
	     passive folded in, mid-run application refused.
	  7. Serialization: round-trip equality, DataStore-safety of the payload,
	     tolerance of corrupt blobs, refusal of a newer schema, and the
	     conflict-free `merge`.
	  8. Cosmetics: ownership, equip gating, loadout defaults, and — an assertion
	     about the DESIGN, not the code — that no chassis carries a price.
]]

local Content = require("../src/shared/Content")
local Rng = require("../src/shared/Rng")
local State = require("../src/shared/sim/State")
local Shop = require("../src/shared/sim/Shop")
local Profile = require("../src/server/Profile")
local Meta = require("../src/server/Meta")

local checks, failures = 0, 0

local function fail(msg: string)
	failures += 1
	print("FAIL  " .. msg)
end

local function ok(cond: boolean, msg: string)
	checks += 1
	if not cond then
		fail(msg)
	end
end

local function eq(got: any, want: any, msg: string)
	checks += 1
	if got ~= want then
		fail(msg .. ": got " .. tostring(got) .. ", want " .. tostring(want))
	end
end

local function G(name: string)
	print("-- " .. name)
end

--- Deep structural equality, for the serialization round-trip.
local function deepEq(a: any, b: any, path: string): (boolean, string?)
	if type(a) ~= type(b) then
		return false, path .. ": type " .. type(a) .. " vs " .. type(b)
	end
	if type(a) ~= "table" then
		if a ~= b then
			return false, path .. ": " .. tostring(a) .. " vs " .. tostring(b)
		end
		return true, nil
	end
	for k, v in a do
		local good, why = deepEq(v, b[k], path .. "." .. tostring(k))
		if not good then
			return false, why
		end
	end
	for k in b do
		if a[k] == nil then
			return false, path .. "." .. tostring(k) .. ": missing on the left"
		end
	end
	return true, nil
end

local WEAPONS, MODIFIERS = "weapons", "modifiers"
local NW, NM = #Content.WEAPONS, #Content.MODIFIERS

--=============================================================================
G("1. XP from a run")
--=============================================================================

local X = Meta.XP

eq(Meta.xpForRun({ rounds = 0, ticks = 0, damage = 0, gold = 0 }), X.completion, "an instant death is worth the completion award only")

eq(
	Meta.xpForRun({ rounds = 3, ticks = 2700, damage = 250000, gold = 40000 }),
	X.completion + 3 * X.per_round + 30 + 10 + 20,
	"a mid-length run"
)

eq(
	Meta.xpForRun({ rounds = 3, ticks = 2700, damage = 250000, gold = 40000, last_stand = true })
		- Meta.xpForRun({ rounds = 3, ticks = 2700, damage = 250000, gold = 40000 }),
	X.last_stand,
	"Last Stand is worth exactly its award"
)

eq(
	Meta.xpForRun({ rounds = 1e9, ticks = 1e9, damage = 1e12, gold = 1e12 }),
	X.max_per_run,
	"a single run is capped"
)

eq(Meta.xpForRun({ rounds = -5, ticks = -5, damage = -5, gold = -5 }), X.completion, "negative inputs floor at 0")

-- Integer division, not rounding: 89 ticks is worth nothing.
eq(
	Meta.xpForRun({ rounds = 0, ticks = X.ticks_per_xp - 1, damage = 0, gold = 0 }),
	X.completion,
	"partial progress toward a tick award is not paid out"
)

do
	local s = State.new(12345, 1)
	local fresh = Meta.runResultFromState(s)
	eq(fresh.rounds, 0, "pre-first-round (U32_MAX) reads as 0 rounds completed")
	eq(fresh.ticks, 0, "a fresh arena survived 0 ticks")
	s.round = 7
	s.tick = 6300
	s.total_damage_dealt = 900000
	s.total_gold_earned = 12000
	local r = Meta.runResultFromState(s, true)
	eq(r.rounds, 7, "rounds completed = the round index")
	eq(r.damage, 900000, "damage is read straight off the arena")
	eq(r.gold, 12000, "gold is read straight off the arena")
	eq(r.last_stand, true, "Last Stand comes from the director, not the arena")
	-- Read-only: the arena is the sim's, not ours.
	eq(s.tick, 6300, "runResultFromState does not mutate the arena")
end

--=============================================================================
G("2. the unlock schedule")
--=============================================================================

do
	local good, problems = Meta.validate()
	ok(good, "Meta.validate: " .. table.concat(problems, "; "))
end

do
	local zero = { weapons = 0, modifiers = 0 }
	for _, kind in { WEAPONS, MODIFIERS } do
		local n = 0
		local defs = if kind == WEAPONS then Content.WEAPONS else Content.MODIFIERS
		for i = 1, #defs do
			local need = Meta.unlockXp(kind, defs[i].id)
			checks += 1
			if need == nil then
				fail(kind .. " id " .. defs[i].id .. " is not on the unlock schedule")
			elseif need == 0 then
				zero[kind] += 1
			end
		end
		n = #Meta.entitledIds(kind, 0)
		eq(n, zero[kind], kind .. ": entitledIds(0) matches the count of free ids")
	end
	eq(zero.weapons, Meta.STARTER.weapons, "the free weapon count is the configured starter size")
	eq(zero.modifiers, Meta.STARTER.modifiers, "the free modifier count is the configured starter size")
end

eq(Meta.unlockXp(WEAPONS, Content.constants.starting_weapon), 0, "the starting weapon is free")
eq(Meta.unlockXp(WEAPONS, NW + 100), nil, "an id outside the catalog is not on the schedule")

-- Rarity is unlock ORDER: nothing from a band is offerable before that band opens.
do
	local worstOpen = Meta.BANDS[3].open
	for _, kind in { WEAPONS, MODIFIERS } do
		local defs = if kind == WEAPONS then Content.WEAPONS else Content.MODIFIERS
		local early = 0
		for i = 1, #defs do
			local d = defs[i]
			if d.rarity >= 3 and (Meta.unlockXp(kind, d.id) :: number) < worstOpen then
				early += 1
			end
		end
		eq(early, 0, kind .. ": no Epic is offerable before the Epic band opens")
	end
end

-- Crossing a threshold grants exactly the ids AT that threshold.
do
	local firstStep = nil
	for i = 1, NW do
		local need = Meta.unlockXp(WEAPONS, Content.WEAPONS[i].id) :: number
		if need > 0 and (firstStep == nil or need < firstStep) then
			firstStep = need
		end
	end
	ok(firstStep ~= nil, "there is a first paid weapon unlock")
	local before = #Meta.entitledIds(WEAPONS, firstStep - 1)
	local after = #Meta.entitledIds(WEAPONS, firstStep)
	eq(before, Meta.STARTER.weapons, "one XP short of the first step, only the starter set is entitled")
	ok(after > before, "crossing the first step widens the weapon pool")
end

-- Monotone: XP never narrows the pool.
do
	local prevW, prevM = -1, -1
	local regressions = 0
	for xp = 0, 62000, 250 do
		local w, m = #Meta.entitledIds(WEAPONS, xp), #Meta.entitledIds(MODIFIERS, xp)
		if w < prevW or m < prevM then
			regressions += 1
		end
		prevW, prevM = w, m
	end
	eq(regressions, 0, "the entitled set is monotone non-decreasing in XP")
	eq(prevW, NW, "the whole weapon catalog is reachable")
	eq(prevM, NM, "the whole modifier catalog is reachable")
	ok(Meta.maxUnlockXp(WEAPONS) <= 62000, "maxUnlockXp(weapons) is inside the swept range")
	ok(Meta.maxUnlockXp(MODIFIERS) <= 62000, "maxUnlockXp(modifiers) is inside the swept range")
end

--=============================================================================
G("3. pool restriction: fresh vs mature")
--=============================================================================

local fresh = Profile.new("100")
local mature = Profile.new("200")
Profile.addXp(mature, 20000)
Meta.syncProfile(mature)
local maxed = Profile.new("300")
Profile.addXp(maxed, math.max(Meta.maxUnlockXp(WEAPONS), Meta.maxUnlockXp(MODIFIERS)))

local poolFresh = Meta.pool(fresh)
local poolMature = Meta.pool(mature)
local poolMaxed = Meta.pool(maxed)

eq(#poolFresh.weapons, Meta.STARTER.weapons, "a fresh profile is offered only the starter weapons")
eq(#poolFresh.modifiers, Meta.STARTER.modifiers, "a fresh profile is offered only the starter modifiers")
ok(#poolMature.weapons > #poolFresh.weapons, "a mature profile is offered more weapons")
ok(#poolMature.modifiers > #poolFresh.modifiers, "a mature profile is offered more modifiers")
ok(#poolMature.weapons < NW, "a mature profile still has weapons left to unlock")
eq(#poolMaxed.weapons, NW, "a maxed profile is offered the whole weapon catalog")
eq(#poolMaxed.modifiers, NM, "a maxed profile is offered the whole modifier catalog")
ok(Meta.isFullPool(poolMaxed), "a maxed pool IS the full pool")
ok(not Meta.isFullPool(poolFresh), "a fresh pool is not the full pool")
ok(table.find(poolFresh.weapons, Content.constants.starting_weapon) ~= nil, "the starting weapon is always offerable")

do -- ascending, no duplicates: `pool[kind][r + 1]` must be a stable draw table
	local sorted = true
	local dupes = 0
	local seen = {}
	for i = 2, #poolMature.weapons do
		if poolMature.weapons[i] <= poolMature.weapons[i - 1] then
			sorted = false
		end
	end
	for _, id in poolMature.weapons do
		if seen[id] then
			dupes += 1
		end
		seen[id] = true
	end
	ok(sorted, "the pool is ascending")
	eq(dupes, 0, "the pool has no duplicates")
end

do -- `isOffered` is the O(1) form of pool membership; they must agree
	local disagreements = 0
	for id = 0, NW - 1 do
		local inPool = table.find(poolMature.weapons, id) ~= nil
		if inPool ~= Meta.isOffered(mature, WEAPONS, id) then
			disagreements += 1
		end
	end
	eq(disagreements, 0, "isOffered agrees with pool membership for every weapon id")
	ok(not Meta.isOffered(mature, WEAPONS, NW + 5), "an out-of-catalog id is never offered")
end

do -- a stored unlock survives even if the schedule would not grant it
	local granted = Profile.new("400")
	local locked = nil
	for id = 0, NW - 1 do
		if (Meta.unlockXp(WEAPONS, id) :: number) > 0 then
			locked = id
			break
		end
	end
	ok(locked ~= nil, "there is a weapon locked at 0 XP")
	ok(not Meta.isOffered(granted, WEAPONS, locked), "it is not offered before the grant")
	Profile.unlock(granted, WEAPONS, locked)
	ok(Meta.isOffered(granted, WEAPONS, locked), "a stored unlock is offered regardless of XP")
	eq(#Meta.pool(granted).weapons, Meta.STARTER.weapons + 1, "a stored unlock widens the pool by exactly one")
end

--=============================================================================
G("4/5. the draw: restricted never yields a locked id; full is oracle-identical")
--=============================================================================

-- EXACTLY the draw the documented `Shop.setPool` hook performs (see
-- `Meta.bindShop`). `Rng.below(rng_shop, n)` with `n = #pool[kind]`, then the
-- draw index mapped through the pool. Eight draws, weapons first — the same
-- slot layout and the same draw COUNT as `Shop.generateOffers`.
local function drawWithPool(s, pool)
	local offers = table.create(Shop.OFFER_SLOTS)
	for slot = 0, Shop.OFFER_SLOTS - 1 do
		local kind = if slot < Shop.WEAPON_SLOTS then WEAPONS else MODIFIERS
		local ids = pool[kind]
		local r = Rng.below(s.rng_shop, #ids)
		local def = Meta.defForDraw(pool, kind, r)
		local catalogT = if kind == WEAPONS then Content.WEAPONS else Content.MODIFIERS
		offers[slot + 1] = {
			kind = if kind == WEAPONS then State.OfferKind.Weapon else State.OfferKind.Modifier,
			def = def,
			cost = catalogT[def + 1].cost,
		}
	end
	s.shop.offers = offers
	s.shop.shop_seq += 1
	return offers
end

do -- 5: unrestricted == the real Shop, offer for offer and cursor for cursor
	local mismatches = 0
	for seed = 0, 40 do
		local a = State.new(seed, 1)
		local b = State.new(seed, 1)
		local full = Meta.fullPool()
		for _ = 1, 4 do -- the round-0 shop plus three rerolls
			Shop.generateOffers(a)
			drawWithPool(b, full)
			for i = 1, Shop.OFFER_SLOTS do
				local x, y = a.shop.offers[i], b.shop.offers[i]
				if x.kind ~= y.kind or x.def ~= y.def or x.cost ~= y.cost then
					mismatches += 1
				end
			end
			local ahi, alo = Rng.state(a.rng_shop)
			local bhi, blo = Rng.state(b.rng_shop)
			if ahi ~= bhi or alo ~= blo then
				mismatches += 1
			end
		end
		if a.shop.shop_seq ~= b.shop.shop_seq then
			mismatches += 1
		end
	end
	eq(mismatches, 0, "a full-pool draw reproduces Shop.generateOffers exactly (offers AND rng_shop cursor)")
end

do -- 4: the restriction actually holds
	local violations, drawn = 0, 0
	local pools = {
		{ p = fresh, pool = poolFresh, name = "fresh" },
		{ p = mature, pool = poolMature, name = "mature" },
	}
	for _, case in pools do
		local unlockedW, unlockedM = {}, {}
		for _, id in case.pool.weapons do
			unlockedW[id] = true
		end
		for _, id in case.pool.modifiers do
			unlockedM[id] = true
		end
		for seed = 0, 120 do
			local s = State.new(seed, 1)
			for _ = 1, 6 do
				local offers = drawWithPool(s, case.pool)
				for i = 1, Shop.OFFER_SLOTS do
					local o = offers[i]
					drawn += 1
					local isWeapon = o.kind == State.OfferKind.Weapon
					local allowed = if isWeapon then unlockedW[o.def] else unlockedM[o.def]
					if not allowed then
						violations += 1
					end
					if not Meta.isOffered(case.p, if isWeapon then WEAPONS else MODIFIERS, o.def) then
						violations += 1
					end
					-- The cost must come from the REAL catalog entry, not from the
					-- pool position.
					local catalogT = if isWeapon then Content.WEAPONS else Content.MODIFIERS
					if o.cost ~= catalogT[o.def + 1].cost then
						violations += 1
					end
				end
			end
		end
	end
	ok(drawn >= 11000, "the restriction sweep actually drew a large sample (" .. drawn .. " offers)")
	eq(violations, 0, "a restricted pool NEVER yields a locked id")
end

do -- 4b: and the restricted draw really is narrower than the full one
	local s = State.new(99, 1)
	local seen = {}
	for _ = 1, 200 do
		for _, o in drawWithPool(s, poolFresh) do
			if o.kind == State.OfferKind.Weapon then
				seen[o.def] = true
			end
		end
	end
	local n = 0
	for _ in seen do
		n += 1
	end
	eq(n, #poolFresh.weapons, "200 fresh-pool draws cover the starter pool and nothing outside it")
end

do -- `bindShop`: the wiring seam, feature-detected
	local captured = nil
	local fakeShop = {
		setPool = function(p)
			captured = p
		end,
	}
	Meta.bindShop(fakeShop, poolFresh)
	eq(captured, poolFresh, "bindShop hands the pool to a pool-aware Shop")

	-- Against today's real Shop (no `setPool`): a full pool is a no-op...
	local okFull = pcall(Meta.bindShop, Shop, Meta.fullPool())
	ok(okFull, "bindShop accepts an unrestricted pool against the stock Shop")
	-- ...and a restricted one must fail LOUDLY rather than silently not restrict.
	local okRestricted = pcall(Meta.bindShop, Shop, poolFresh)
	ok(not okRestricted, "bindShop refuses to pretend it restricted a stock Shop")
end

--=============================================================================
G("6. chassis application")
--=============================================================================

do
	local base = State.new(7, 1)
	eq(base.weapons[1].def, Content.constants.starting_weapon, "a stock arena starts with the default weapon")
	eq(base.tank.max_hp, 24000, "a stock arena starts at 24000 max HP")

	local s = State.new(7, 1)
	Meta.applyChassis(s, "bulwark")
	eq(s.weapons[1].def, 12, "Bulwark replaces the starting weapon with Boulder")
	eq(#s.weapons, 1, "the chassis weapon REPLACES rather than adds")
	eq(s.weapons[1].instance_id, base.weapons[1].instance_id, "entity ids are untouched")
	eq(s.next_entity_id, base.next_entity_id, "no entity id was consumed")
	eq(s.tank.max_hp, 24500, "Bulwark's Improved Masonry passive is folded in (+500 max HP)")
	eq(s.economy.gold, base.economy.gold, "a chassis passive is free — no gold is spent")

	local p = State.new(7, 1)
	Meta.applyChassis(p, "prospector")
	eq(p.weapons[1].def, 1, "Prospector starts with the Mortar Launcher")
	eq(p.economy.income_per_tick, base.economy.income_per_tick + 10, "Gold Mine's flat income is applied")

	local r = State.new(7, 1)
	Meta.applyChassis(r, "ranger")
	eq(r.weapons[1].def, 0, "Ranger starts with the Bow")
	eq(r.tank.max_hp, 24000, "Ranger's passive is not a stat passive")

	-- Deterministic: the same chassis on the same seed is the same state.
	local a, b = State.new(7, 2), State.new(7, 2)
	Meta.applyChassis(a, "alchemist")
	Meta.applyChassis(b, "alchemist")
	local same = deepEq(a.tank, b.tank, "tank")
	ok(same, "chassis application is deterministic")
	local ahi, alo = Rng.state(a.rng_shop)
	local bhi, blo = Rng.state(b.rng_shop)
	ok(ahi == bhi and alo == blo, "chassis application draws no RNG")
	local chi, clo = Rng.state(State.new(7, 2).rng_shop)
	ok(ahi == chi and alo == clo, "...and leaves rng_shop exactly where State.new did")

	ok(not pcall(Meta.applyChassis, State.new(7, 1), "no_such_chassis"), "an unknown chassis is refused")
	local mid = State.new(7, 1)
	mid.tick = 30
	ok(not pcall(Meta.applyChassis, mid, "bulwark"), "mid-run application is refused (it would fork the checksum)")
	local shopped = State.new(7, 1)
	shopped.shop.shop_seq = 1
	ok(not pcall(Meta.applyChassis, shopped, "bulwark"), "application after the first shop is refused")
end

do -- ownership gating
	local p = Profile.new("500")
	eq(Meta.selectedChassis(p).id, Meta.DEFAULT_CHASSIS, "a fresh profile runs the default chassis")
	ok(Meta.ownsChassis(p, Meta.DEFAULT_CHASSIS), "the default chassis is owned from 0 XP")
	ok(not Meta.ownsChassis(p, "arcanist"), "a locked chassis is not owned")
	ok(not Meta.selectChassis(p, "arcanist"), "selecting a locked chassis is refused")
	eq(Meta.selectedChassis(p).id, Meta.DEFAULT_CHASSIS, "...and does not take effect")
	ok(not Meta.selectChassis(p, "no_such_chassis"), "selecting an unknown chassis is refused")

	Profile.addXp(p, 30000)
	Meta.syncProfile(p)
	ok(Meta.ownsChassis(p, "arcanist"), "XP unlocks the chassis")
	ok(Profile.hasChassis(p, "arcanist"), "syncProfile materialized the ownership")
	ok(Meta.selectChassis(p, "arcanist"), "an owned chassis can be selected")
	eq(Meta.selectedChassis(p).id, "arcanist", "the selection sticks")

	local s = State.new(7, 1)
	eq(Meta.applyProfile(s, p), "arcanist", "applyProfile applies the selection and reports it")
	eq(s.weapons[1].def, 11, "...and the arena starts with the Arcanist's weapon")

	-- A selection that is no longer valid must fall back, not error.
	p.selected_chassis = "ghost_chassis"
	eq(Meta.selectedChassis(p).id, Meta.DEFAULT_CHASSIS, "a stale selection falls back to the default")

	eq(#Meta.ownedChassis(Profile.new("600")), 1, "a fresh account owns exactly one chassis")
	ok(#Meta.ownedChassis(p) > 1, "a mature account owns several")
end

--=============================================================================
G("7. serialization, storage and reconcile")
--=============================================================================

do
	local p = Profile.new("777")
	Profile.addXp(p, 21500)
	Meta.syncProfile(p)
	Meta.selectChassis(p, "bulwark")
	Meta.grantCosmetic(p, "skin_obsidian")
	Meta.grantCosmetic(p, "trail_ember")
	Meta.equipCosmetic(p, "skin_obsidian")
	Meta.equipCosmetic(p, "trail_ember")
	Profile.recordRun(p, { rounds = 9, ticks = 8100, damage = 4000000, gold = 90000, last_stand = true })
	Profile.unlock(p, WEAPONS, NW - 1)

	local raw = Profile.serialize(p)
	local safe, why = Profile.isDataStoreSafe(raw)
	ok(safe, "the serialized payload is DataStore-safe: " .. tostring(why))

	local q = Profile.deserialize(raw)
	local same, diff = deepEq(Profile.serialize(q), raw, "profile")
	ok(same, "serialize -> deserialize -> serialize is identity: " .. tostring(diff))
	eq(q.xp, p.xp, "XP round-trips")
	eq(q.selected_chassis, "bulwark", "the chassis selection round-trips")
	eq(q.stats.last_stands, 1, "lifetime stats round-trip")
	eq(q.stats.rounds_best, 9, "the best-round stat round-trips")
	ok(Profile.isUnlocked(q, WEAPONS, NW - 1), "a stored unlock round-trips")
	ok(Profile.hasCosmetic(q, "skin_obsidian"), "cosmetic ownership round-trips")
	eq(Profile.equipped(q, "skin"), "skin_obsidian", "the equipped cosmetic round-trips")
	local sameSame, diff2 = deepEq(Meta.pool(q), Meta.pool(p), "pool")
	ok(sameSame, "a round-tripped profile draws the same pool: " .. tostring(diff2))

	-- Arrays, not sets, on the wire: that is what JSON can carry.
	eq(type(raw.unlocked_weapons), "table", "unlocks serialize as a table")
	eq(#raw.unlocked_weapons, #Meta.entitledIds(WEAPONS, p.xp) + 1, "every unlocked id is on the wire")
	local ascending = true
	for i = 2, #raw.unlocked_weapons do
		if raw.unlocked_weapons[i] <= raw.unlocked_weapons[i - 1] then
			ascending = false
		end
	end
	ok(ascending, "the serialized id array is sorted (stable blobs are diffable blobs)")
end

do -- the unsafe shapes the checker exists to catch
	ok(not (Profile.isDataStoreSafe({ [1] = "a", x = 2 })), "mixed keys are rejected")
	ok(not (Profile.isDataStoreSafe({ [1] = "a", [3] = "b" })), "sparse arrays are rejected")
	ok(not (Profile.isDataStoreSafe({ x = 0 / 0 })), "NaN is rejected")
	ok(not (Profile.isDataStoreSafe({ x = math.huge })), "infinity is rejected")
	ok(not (Profile.isDataStoreSafe({ x = print })), "functions are rejected")
	local cyclic = {}
	cyclic.self = cyclic
	ok(not (Profile.isDataStoreSafe(cyclic)), "cycles are rejected")
	ok((Profile.isDataStoreSafe({ a = { 1, 2, 3 }, b = "x", c = true })), "a plain nested payload is accepted")
end

do -- tolerance and refusal
	local p = Profile.deserialize({ xp = "not a number", unlocked_weapons = "junk", stats = 5 }, "888")
	eq(p.xp, 0, "a corrupt XP field degrades to 0 rather than locking the account out")
	eq(p.user_id, "888", "the key the blob was loaded under wins")
	eq(#Meta.pool(p).weapons, Meta.STARTER.weapons, "a corrupt profile still gets a valid starter pool")

	local q = Profile.deserialize({ v = 1, xp = 500, selected_chassis = "bulwark", chassis = {} }, "889")
	eq(q.selected_chassis, "", "a selection the account does not own is dropped on load")

	local newer, err = Profile.tryDeserialize({ v = Profile.SCHEMA_VERSION + 1, xp = 10 })
	eq(newer, nil, "a newer schema is refused")
	ok(err ~= nil, "...with a reason")
	ok(not pcall(Profile.deserialize, { v = Profile.SCHEMA_VERSION + 1 }), "deserialize errors on it")
	ok(not pcall(Profile.deserialize, "not a table"), "a non-table blob is refused")
end

do -- merge: two servers, one account
	local a = Profile.new("900")
	Profile.addXp(a, 5000)
	Profile.unlock(a, WEAPONS, 40)
	Profile.recordRun(a, { rounds = 4, ticks = 3600, damage = 10, gold = 10 })
	local b = Profile.deserialize(Profile.serialize(a), "900")
	Profile.unlock(b, WEAPONS, 41)
	Profile.addXp(b, 900)
	Profile.recordRun(b, { rounds = 9, ticks = 8100, damage = 10, gold = 10 })

	local m = Profile.merge(a, b)
	ok(Profile.isUnlocked(m, WEAPONS, 40) and Profile.isUnlocked(m, WEAPONS, 41), "merge unions the unlock sets")
	eq(m.xp, 5900, "merge keeps the higher XP")
	eq(m.stats.rounds_best, 9, "merge keeps the better run")
	eq(m.stats.runs, 2, "monotone counters take the max, so a duplicated run counts once")
	eq(Profile.merge(b, a).xp, m.xp, "merge is symmetric in XP")
	eq(a.xp, 5000, "merge does not mutate its inputs")
end

do -- the adapter seam, exercised through the in-memory implementation
	local adapter = Profile.memoryAdapter()
	local repo = Profile.newRepo(adapter)
	local p = repo:load("42")
	eq(p.xp, 0, "an unknown account loads as a fresh profile")

	local award = Meta.awardRun(p, { rounds = 6, ticks = 5400, damage = 800000, gold = 60000, last_stand = true })
	ok(award.xp_gained > 0, "a run awards XP")
	eq(award.xp_total, p.xp, "the award reports the new total")
	ok(#award.unlocked.weapons >= Meta.STARTER.weapons, "the first award materializes the starter unlocks")
	eq(p.stats.runs, 1, "the run is recorded")
	repo:save(p)

	local reloaded = repo:load("42")
	eq(reloaded.xp, p.xp, "XP survives the store")
	eq(reloaded.stats.rounds_best, 6, "stats survive the store")

	-- Atomic read-modify-write: the mutate transform sees stored state.
	local mutated = repo:mutate("42", function(prof)
		Meta.awardRun(prof, { rounds = 2, ticks = 1800, damage = 0, gold = 0 })
	end)
	eq(mutated.stats.runs, 2, "mutate composes on top of what was stored")
	eq(repo:load("42").stats.runs, 2, "...and the result was written back")

	-- A save that would store a broken payload must fail at the call site rather
	-- than inside `UpdateAsync`. (`save` merges against what is stored first, and
	-- `merge` drops a NaN on the way through — so the guard is reached via
	-- `mutate`, which serializes the caller's profile directly.)
	ok(not pcall(function()
		repo:mutate("42", function(prof)
			prof.stats.xp_total = 0 / 0
		end)
	end), "mutate refuses to store an unsafe payload")

	local broken = repo:load("42")
	broken.stats.xp_total = 0 / 0
	local merged = repo:save(broken)
	ok(merged.stats.xp_total == merged.stats.xp_total, "merge sanitizes a NaN counter rather than storing it")
	ok((Profile.isDataStoreSafe(Profile.serialize(repo:load("42")))), "the stored blob is still safe afterwards")
end

--=============================================================================
G("8. cosmetics, and the monetization line")
--=============================================================================

do
	local p = Profile.new("1000")
	ok(Meta.ownsCosmetic(p, "skin_stock"), "the default skin is owned by everyone")
	ok(not Meta.ownsCosmetic(p, "skin_obsidian"), "a priced skin is not owned by default")
	ok(not Meta.equipCosmetic(p, "skin_obsidian"), "an unowned cosmetic cannot be equipped")
	ok(not Meta.grantCosmetic(p, "no_such_cosmetic"), "an unknown cosmetic cannot be granted")
	ok(Meta.grantCosmetic(p, "skin_obsidian"), "a completed purchase grants the cosmetic")
	ok(not Meta.grantCosmetic(p, "skin_obsidian"), "granting twice is a no-op (receipts can replay)")
	ok(Meta.equipCosmetic(p, "skin_obsidian"), "an owned cosmetic can be equipped")
	eq(Meta.loadout(p).skin, "skin_obsidian", "the loadout reports the equipped skin")
	eq(Meta.loadout(p).trail, "trail_none", "an empty slot falls back to its default")
	eq(Meta.loadout(p).arena, "arena_classic", "every slot is filled")
	ok(Meta.unequip(p, "skin"), "a slot can be cleared")
	eq(Meta.loadout(p).skin, "skin_stock", "...and falls back to the default")

	-- The F3 trade-off, asserted rather than assumed: cosmetics are the only
	-- priced content, and buying every one of them cannot widen the offer pool
	-- or change a chassis.
	local rich = Profile.new("1001")
	for _, c in Meta.COSMETICS do
		Profile.grantCosmetic(rich, c.id)
	end
	local poor = Profile.new("1002")
	local samePool, diff = deepEq(Meta.pool(rich), Meta.pool(poor), "pool")
	ok(samePool, "owning every cosmetic does not widen the offer pool: " .. tostring(diff))
	eq(#Meta.ownedChassis(rich), #Meta.ownedChassis(poor), "owning every cosmetic unlocks no chassis")
	local priced = 0
	for _, c in Meta.CHASSIS do
		if (c :: any).price ~= nil then
			priced += 1
		end
	end
	eq(priced, 0, "no chassis carries a price (F3: no paid power)")
end

--=============================================================================

print(string.format("meta_test: %d checks, %d failures", checks, failures))
if failures > 0 then
	error(string.format("meta_test FAILED (%d)", failures), 0)
end
print("meta_test: OK")
