--!nonstrict
--[[
	wire_test.luau — proves `src/shared/net/Wire.luau` round-trips a
	`RenderView` losslessly, that its delta encoding handles spawn / move /
	despawn and keyframe rebasing, and that the encoded size actually fits the
	transport budget in `PHASE2-CONTRACTS.md` P4.

	HOW TO RUN (no arguments needed — everything is self-contained):

	    cd roblox/test && luau wire_test.luau            # tests + size report
	    cd roblox/test && luau wire_test.luau -a size    # size report only
	    cd roblox/test && luau wire_test.luau -a test    # tests only

	THE SIZE REPORT IS MEASURED, NOT QUOTED. `sizeReport()` below builds a
	synthetic arena at the population `docs/10 §F8` measured off the real Rust
	sim (67 enemies / 84 projectiles / 20 weapons), walks it for 200 frames at
	the 15 Hz replication rate, and prints the actual `buffer.len` of every
	message. Re-run it after touching the codec; do not copy its numbers
	forward by hand.

	P0: this file and `src/shared/net/Wire.luau` are the only two files this
	task owns. Nothing under `src/shared/sim/` is written to; `View.luau` is
	required read-only, and only to type-check that `Wire`'s mirrored type
	declarations still match the ones the sim actually produces.
]]

local Wire = require("../src/shared/net/Wire")
local View = require("../src/shared/sim/View")

-- Type-level drift guard. `Wire` re-declares `View`'s render types so it can
-- stay dependency-free; these two assignments make `luau-analyze` fail if the
-- two definitions stop being structurally identical in either direction.
local _viewToWire: Wire.RenderView = (nil :: any) :: View.RenderView
local _wireToView: View.RenderView = (nil :: any) :: Wire.RenderView

local mode = (...) or "all"

-- ============================ harness ============================

local failures = 0
local checks = 0

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

local function eq(got, want, msg: string)
	checks += 1
	if got ~= want then
		fail(`{msg}: got {tostring(got)}, want {tostring(want)}`)
	end
end

--- Assert `fn` raises, and that the message mentions `needle`.
local function raises(fn, needle: string, msg: string)
	checks += 1
	local okCall, err = pcall(fn)
	if okCall then
		fail(`{msg}: expected an error, but the call succeeded`)
	elseif needle ~= "" and not string.find(tostring(err), needle, 1, true) then
		fail(`{msg}: error did not mention "{needle}" — got {tostring(err)}`)
	end
end

-- ============================ deep equality ============================

--- Structural equality with a path, so a field-for-field failure names the
--- field. Type-strict: `false` and `nil` are different, `1` and `"1"` are
--- different. That strictness is the point — the round-trip claim is
--- field-for-field.
local function deepDiff(a, b, path: string): string?
	local ta, tb = type(a), type(b)
	if ta ~= tb then
		return `{path}: type {ta} vs {tb}`
	end
	if ta ~= "table" then
		if a ~= b then
			return `{path}: {tostring(a)} vs {tostring(b)}`
		end
		return nil
	end
	for k, va in a do
		local d = deepDiff(va, b[k], `{path}.{tostring(k)}`)
		if d then
			return d
		end
	end
	for k in b do
		if a[k] == nil then
			return `{path}.{tostring(k)}: missing on the left`
		end
	end
	return nil
end

local function same(a, b, msg: string)
	checks += 1
	local d = deepDiff(a, b, "view")
	if d then
		fail(`{msg}: {d}`)
	end
end

-- ============================ view fixtures ============================

--- Deterministic LCG so every number in this file is reproducible. Not the
--- sim's `Rng` — nothing here feeds a checksum, this is test scaffolding.
local function lcg(seed: number)
	local s = seed % 2147483647
	if s <= 0 then
		s += 2147483646
	end
	return function(n: number): number
		s = (s * 48271) % 2147483647
		return s % n
	end
end

local ENEMY_NAMES = {
	"Squeakzilla",
	"Fanged Death",
	"The Hippocrate",
	"Doomduck",
	"Bacon",
	"Honk",
	"Grunt",
	"Peon",
}
local WEAPON_NAMES = {
	"Bow",
	"Mortar Launcher",
	"Frost Bow",
	"Poison Bow",
	"Flamecaster",
	"Storm Hammer",
	"Death Engine",
	"Siege Ballista",
}

local function emptyish(tick: number): Wire.RenderView
	return {
		tick = tick,
		round = 0,
		dead = false,
		tank = { x = 0, y = 0, hp = 0, max_hp = 0, revives = 0 },
		enemies = {},
		projectiles = {},
		minions = {},
		economy = { gold = 0, income_per_tick = 0, rerolls_remaining = 0, reroll_cost = 0 },
		shop = {},
		arsenal = {},
		stats = {
			damage_dealt = 0,
			gold_earned = 0,
			bought_attack_mask = 0,
			weapons_bought = 0,
			economy_purchases = 0,
		},
	}
end

--- A full arena at an arbitrary population. `spread` is the per-frame motion
--- magnitude used by `advance`, not by the builder itself.
local function makeView(cfg): Wire.RenderView
	local rnd = lcg(cfg.seed or 12345)
	local v = emptyish(cfg.tick or 0)
	v.round = cfg.round or 3
	v.dead = cfg.dead or false
	v.tank = {
		x = 0,
		y = 0,
		hp = 4200,
		max_hp = 5000,
		revives = 1,
	}

	local nEnemies = cfg.enemies or 0
	for i = 1, nEnemies do
		local kind = rnd(#ENEMY_NAMES)
		local boss = kind == 2
		v.enemies[i] = {
			id = (cfg.firstId or 1) + i - 1,
			x = rnd(3600) - 1800,
			y = rnd(3600) - 1800,
			hp = boss and (8000000 + rnd(1000000)) or (400 + rnd(9000)),
			base_hp = boss and 33000000 or (120 + kind * 300),
			kind = kind,
			boss = boss,
			name = ENEMY_NAMES[kind + 1],
		}
	end

	for i = 1, (cfg.projectiles or 0) do
		v.projectiles[i] = { x = rnd(3600) - 1800, y = rnd(3600) - 1800 }
	end

	for i = 1, (cfg.minions or 0) do
		v.minions[i] = { x = rnd(3600) - 1800, y = rnd(3600) - 1800, kind = rnd(4) }
	end

	v.economy = {
		gold = 15230,
		income_per_tick = 47,
		rerolls_remaining = 2,
		reroll_cost = 300,
	}

	for i = 1, (cfg.offers or 5) do
		local w = rnd(#WEAPON_NAMES)
		v.shop[i] = {
			slot = i - 1,
			name = WEAPON_NAMES[w + 1],
			cost = 250 + w * 75,
			is_weapon = w % 2 == 0,
			affordable = (250 + w * 75) <= v.economy.gold,
			rarity = w % 4,
		}
	end

	for i = 1, (cfg.weapons or 0) do
		v.arsenal[i] = { name = WEAPON_NAMES[(i - 1) % #WEAPON_NAMES + 1] .. " " .. tostring(i), count = 1 + rnd(4) }
	end

	v.stats = {
		damage_dealt = 918273645,
		gold_earned = 152300,
		bought_attack_mask = 11,
		weapons_bought = cfg.weapons or 0,
		economy_purchases = 4,
	}
	return v
end

--- Advance a view by one 15 Hz replication frame (two 30 Hz sim ticks):
--- everything drifts, a few enemies die, a few spawn, projectile count breathes.
--- `opts.step` sets the per-frame motion magnitude.
local function advance(prev: Wire.RenderView, rnd, opts): (Wire.RenderView, number)
	opts = opts or {}
	local step = opts.step or 4
	local churn = opts.churn or 2

	local v = emptyish(prev.tick + 2)
	v.round = prev.round
	v.dead = prev.dead
	v.tank = {
		x = prev.tank.x,
		y = prev.tank.y,
		hp = math.max(0, prev.tank.hp - rnd(3)),
		max_hp = prev.tank.max_hp,
		revives = prev.tank.revives,
	}

	local maxId = 0
	for _, e in prev.enemies do
		if e.id > maxId then
			maxId = e.id
		end
	end

	-- Move survivors toward the tank; kill `churn` of them.
	local killFrom = churn > 0 and (rnd(math.max(1, #prev.enemies)) + 1) or math.huge
	local n = 0
	for i, e in prev.enemies do
		if i >= killFrom and i < killFrom + churn then
			continue
		end
		n += 1
		local dx = e.x > 0 and -rnd(step + 1) or rnd(step + 1)
		local dy = e.y > 0 and -rnd(step + 1) or rnd(step + 1)
		v.enemies[n] = {
			id = e.id,
			x = e.x + dx,
			y = e.y + dy,
			hp = math.max(1, e.hp - rnd(220)),
			base_hp = e.base_hp,
			kind = e.kind,
			boss = e.boss,
			name = e.name,
		}
	end

	-- Respawn to hold the population steady.
	local spawned = 0
	while n < #prev.enemies do
		n += 1
		spawned += 1
		maxId += 1
		local kind = rnd(#ENEMY_NAMES)
		v.enemies[n] = {
			id = maxId,
			x = rnd(2) == 0 and 1800 or -1800,
			y = rnd(3600) - 1800,
			hp = 400 + rnd(9000),
			base_hp = 120 + kind * 300,
			kind = kind,
			boss = false,
			name = ENEMY_NAMES[kind + 1],
		}
	end

	-- Projectiles drift and the array shrinks/grows a little, which is what
	-- forces some index slots onto the absolute encoding.
	local pcount = math.max(0, #prev.projectiles - rnd(4) + rnd(4))
	for i = 1, pcount do
		local p = prev.projectiles[i]
		if p then
			v.projectiles[i] = { x = p.x + rnd(2 * step + 1) - step, y = p.y + rnd(2 * step + 1) - step }
		else
			v.projectiles[i] = { x = rnd(3600) - 1800, y = rnd(3600) - 1800 }
		end
	end

	for i, m in prev.minions do
		v.minions[i] = { x = m.x + rnd(3) - 1, y = m.y + rnd(3) - 1, kind = m.kind }
	end

	v.economy = {
		gold = prev.economy.gold + prev.economy.income_per_tick * 2,
		income_per_tick = prev.economy.income_per_tick,
		rerolls_remaining = prev.economy.rerolls_remaining,
		reroll_cost = prev.economy.reroll_cost,
	}
	v.shop = prev.shop
	v.arsenal = prev.arsenal
	v.stats = {
		damage_dealt = prev.stats.damage_dealt + rnd(4000),
		gold_earned = prev.stats.gold_earned + 90,
		bought_attack_mask = prev.stats.bought_attack_mask,
		weapons_bought = prev.stats.weapons_bought,
		economy_purchases = prev.stats.economy_purchases,
	}
	return v, spawned
end

-- ============================ 1. keyframe round-trip ============================

local function testKeyframeRoundTrip()
	local cases = {
		{ name = "empty", cfg = { enemies = 0, projectiles = 0, minions = 0, offers = 0, weapons = 0 } },
		{ name = "single enemy", cfg = { enemies = 1, projectiles = 0, minions = 0, offers = 1, weapons = 1 } },
		{ name = "single projectile", cfg = { enemies = 0, projectiles = 1, minions = 1, offers = 0, weapons = 0 } },
		{ name = "real peak 67/84/20", cfg = { enemies = 67, projectiles = 84, minions = 3, offers = 5, weapons = 20 } },
		{ name = "worst case 300 enemies", cfg = { enemies = 300, projectiles = 300, minions = 20, offers = 5, weapons = 20 } },
	}
	for _, c in cases do
		c.cfg.tick = 51234
		local v = makeView(c.cfg)
		local buf = Wire.packView(v)
		local back = Wire.unpackView(buf)
		same(back, v, `keyframe round-trip [{c.name}]`)
		ok(Wire.isKeyframe(buf), `isKeyframe [{c.name}]`)
		eq(Wire.baseTick(buf), nil, `keyframe has no baseTick [{c.name}]`)
	end

	-- Degenerate scalars: tick 0, negative-capable value fields, dead flag.
	local v = emptyish(0)
	v.dead = true
	v.round = 0
	v.tank = { x = -1800, y = 1799, hp = 0, max_hp = 5000, revives = 0 }
	v.economy = { gold = 0, income_per_tick = -12, rerolls_remaining = 0, reroll_cost = 0 }
	v.stats.damage_dealt = 0
	same(Wire.unpackView(Wire.packView(v)), v, "keyframe round-trip [degenerate scalars]")

	-- Large i64-ish magnitudes: boss hp and lifetime damage.
	local big = makeView({ enemies = 3, projectiles = 1, tick = 55200 })
	big.enemies[1].hp = 33000000
	big.enemies[1].base_hp = 33000000
	big.enemies[1].boss = true
	big.stats.damage_dealt = 4503599627370495
	big.stats.gold_earned = -4503599627370495
	same(Wire.unpackView(Wire.packView(big)), big, "keyframe round-trip [extreme magnitudes]")

	-- Empty and unicode-ish names must survive byte-for-byte.
	local names = makeView({ enemies = 2, offers = 2, weapons = 2 })
	names.enemies[1].name = ""
	names.enemies[2].name = "Ünïcødé \0 name"
	names.shop[1].name = ""
	names.arsenal[1].name = string.rep("x", 300)
	same(Wire.unpackView(Wire.packView(names)), names, "keyframe round-trip [awkward strings]")
end

-- ============================ 2. delta: spawn / move / despawn ============================

local function testDeltaBasics()
	local a = makeView({ enemies = 4, projectiles = 3, minions = 2, tick = 100, firstId = 10 })

	-- (a) pure move
	local b = makeView({ enemies = 4, projectiles = 3, minions = 2, tick = 102, firstId = 10 })
	for i, e in b.enemies do
		e.x = a.enemies[i].x + 3
		e.y = a.enemies[i].y - 2
		e.hp = a.enemies[i].hp - 17
		e.kind, e.name, e.base_hp, e.boss =
			a.enemies[i].kind, a.enemies[i].name, a.enemies[i].base_hp, a.enemies[i].boss
	end
	b.shop, b.arsenal, b.economy, b.stats, b.tank = a.shop, a.arsenal, a.economy, a.stats, a.tank
	local d = Wire.packDelta(a, b)
	eq(Wire.isKeyframe(d), false, "delta is not a keyframe")
	eq(Wire.baseTick(d), 100, "delta records its base tick")
	same(Wire.applyDelta(a, d), b, "delta [move only]")

	-- (b) despawn the middle two
	local c = { table.unpack(b.enemies) }
	table.remove(c, 2)
	table.remove(c, 2)
	local v3 = makeView({ tick = 104 })
	v3.enemies = c
	v3.projectiles, v3.minions = b.projectiles, b.minions
	v3.shop, v3.arsenal, v3.economy, v3.stats, v3.tank = b.shop, b.arsenal, b.economy, b.stats, b.tank
	v3.round, v3.dead = b.round, b.dead
	same(Wire.applyDelta(b, Wire.packDelta(b, v3)), v3, "delta [despawn]")

	-- (c) spawn two with fresh ids, one of them a boss with a new archetype
	local v4 = makeView({ tick = 106 })
	v4.enemies = { table.unpack(v3.enemies) }
	table.insert(v4.enemies, {
		id = 900,
		x = 1800,
		y = -12,
		hp = 5000,
		base_hp = 4200,
		kind = 5,
		boss = false,
		name = "Honk",
	})
	table.insert(v4.enemies, {
		id = 901,
		x = -1800,
		y = 40,
		hp = 33000000,
		base_hp = 33000000,
		kind = 2,
		boss = true,
		name = "The Hippocrate",
	})
	v4.projectiles, v4.minions = v3.projectiles, v3.minions
	v4.shop, v4.arsenal, v4.economy, v4.stats, v4.tank = v3.shop, v3.arsenal, v3.economy, v3.stats, v3.tank
	v4.round, v4.dead = v3.round, v3.dead
	same(Wire.applyDelta(v3, Wire.packDelta(v3, v4)), v4, "delta [spawn]")

	-- (d) spawn + despawn + move in one frame, with non-monotonic id order
	local v5 = makeView({ tick = 108 })
	v5.enemies = {
		{ id = 901, x = -1790, y = 44, hp = 32000000, base_hp = 33000000, kind = 2, boss = true, name = "The Hippocrate" },
		{ id = 12, x = v4.enemies[2].x + 1, y = v4.enemies[2].y, hp = 3, base_hp = v4.enemies[2].base_hp, kind = v4.enemies[2].kind, boss = v4.enemies[2].boss, name = v4.enemies[2].name },
		{ id = 3000, x = 5, y = 6, hp = 77, base_hp = 120, kind = 0, boss = false, name = "Squeakzilla" },
	}
	v5.enemies[2].id = v4.enemies[2].id
	v5.projectiles, v5.minions = v4.projectiles, v4.minions
	v5.shop, v5.arsenal, v5.economy, v5.stats, v5.tank = v4.shop, v4.arsenal, v4.economy, v4.stats, v4.tank
	v5.round, v5.dead = v4.round, v4.dead
	same(Wire.applyDelta(v4, Wire.packDelta(v4, v5)), v5, "delta [mixed, descending ids]")

	-- (e) projectiles: grow, shrink, empty
	local base = makeView({ enemies = 1, projectiles = 5, tick = 200 })
	for _, count in { 0, 1, 5, 12, 3 } do
		local nxt = makeView({ enemies = 1, projectiles = count, tick = base.tick + 2, seed = count + 7 })
		nxt.enemies = base.enemies
		nxt.minions, nxt.shop, nxt.arsenal, nxt.economy, nxt.stats, nxt.tank =
			base.minions, base.shop, base.arsenal, base.economy, base.stats, base.tank
		nxt.round, nxt.dead = base.round, base.dead
		same(Wire.applyDelta(base, Wire.packDelta(base, nxt)), nxt, `delta [projectiles -> {count}]`)
		base = nxt
	end

	-- (f) minions: kind change and count change
	local m0 = makeView({ minions = 3, tick = 300 })
	local m1 = makeView({ minions = 3, tick = 302, seed = 99 })
	m1.enemies, m1.projectiles, m1.shop, m1.arsenal, m1.economy, m1.stats, m1.tank =
		m0.enemies, m0.projectiles, m0.shop, m0.arsenal, m0.economy, m0.stats, m0.tank
	m1.round, m1.dead = m0.round, m0.dead
	m1.minions[1].kind = m0.minions[1].kind + 1
	m1.minions[2] = { x = m0.minions[2].x, y = m0.minions[2].y, kind = m0.minions[2].kind }
	same(Wire.applyDelta(m0, Wire.packDelta(m0, m1)), m1, "delta [minion kind change]")

	-- (g) an exactly-identical follow-up frame still round-trips
	local idle = makeView({ enemies = 6, projectiles = 4, minions = 1, tick = 400 })
	local idle2 = makeView({ enemies = 6, projectiles = 4, minions = 1, tick = 400 })
	same(Wire.applyDelta(idle, Wire.packDelta(idle, idle2)), idle2, "delta [nothing changed]")

	-- (h) low-rate sections change one at a time
	local s0 = makeView({ enemies = 2, weapons = 3, tick = 500 })
	local variants = {
		{ "shop", function(v) v.shop[1].affordable = not v.shop[1].affordable end },
		{ "arsenal", function(v) v.arsenal[2].count += 1 end },
		{ "economy", function(v) v.economy.rerolls_remaining -= 1 end },
		{ "stats", function(v) v.stats.economy_purchases += 1 end },
		{ "tank", function(v) v.tank.revives += 1 end },
		{ "round", function(v) v.round += 1 end },
		{ "dead", function(v) v.dead = true end },
	}
	for _, variant in variants do
		local s1 = makeView({ enemies = 2, weapons = 3, tick = 502 })
		variant[2](s1)
		same(Wire.applyDelta(s0, Wire.packDelta(s0, s1)), s1, `delta [{variant[1]} changed]`)
	end
end

-- ============================ 3. long delta chain ============================

local function testDeltaChain()
	local rnd = lcg(4242)
	local truth = makeView({ enemies = 67, projectiles = 84, minions = 2, weapons = 20, tick = 0 })
	local client = Wire.unpackView(Wire.packView(truth))
	same(client, truth, "chain [initial keyframe]")

	for frame = 1, 120 do
		local nxt = advance(truth, rnd, { step = 4, churn = 2 })
		if frame % 37 == 0 then
			-- shop reroll: a low-rate section actually changing mid-chain
			nxt.shop = {}
			for i = 1, 5 do
				nxt.shop[i] = {
					slot = i - 1,
					name = WEAPON_NAMES[(frame + i) % #WEAPON_NAMES + 1],
					cost = 200 + i * 50,
					is_weapon = i % 2 == 1,
					affordable = true,
					rarity = i % 4,
				}
			end
			nxt.arsenal = { table.unpack(truth.arsenal) }
			nxt.arsenal[1] = { name = truth.arsenal[1].name, count = truth.arsenal[1].count + 1 }
		end
		local d = Wire.packDelta(truth, nxt)
		client = Wire.applyDelta(client, d)
		same(client, nxt, `chain [frame {frame}]`)
		truth = nxt
	end

	-- The client that walked 120 deltas is byte-identical to a fresh keyframe.
	same(Wire.unpackView(Wire.packView(truth)), client, "chain [keyframe agrees with delta-walked view]")
end

-- ============================ 4. keyframe rebase ============================

local function testRebase()
	local rnd = lcg(777)
	local truth = makeView({ enemies = 40, projectiles = 50, weapons = 12, tick = 1000 })
	local client = Wire.unpackView(Wire.packView(truth))

	local f1 = advance(truth, rnd, {})
	local f2 = advance(f1, rnd, {})
	local f3 = advance(f2, rnd, {})

	-- The delta for f2 is lost. Applying f3's delta must be REFUSED, not
	-- silently applied on top of the stale view.
	local d3 = Wire.packDelta(f2, f3)
	raises(function()
		return Wire.applyDelta(client, d3)
	end, "does not match held view tick", "rebase [stale base is rejected]")
	eq(Wire.baseTick(d3), f2.tick, "rebase [baseTick identifies the missing frame]")

	-- `Wire.unpack` with no base at all is the joining-client case.
	raises(function()
		return Wire.unpack(nil, d3)
	end, "request a keyframe", "rebase [delta with no base]")

	-- A keyframe re-bases without any history, and the chain resumes.
	client = Wire.unpack(nil, Wire.packView(f3))
	same(client, f3, "rebase [keyframe restores exactly]")
	local f4 = advance(f3, rnd, {})
	client = Wire.unpack(client, Wire.packDelta(f3, f4))
	same(client, f4, "rebase [chain resumes after rebase]")

	-- A joining client mid-run gets the full arena from one keyframe.
	local joiner = Wire.unpackView(Wire.packView(f4))
	same(joiner, f4, "rebase [late joiner]")
end

-- ============================ 5. chunking ============================

local function testChunking()
	local v = makeView({ enemies = 300, projectiles = 300, minions = 20, weapons = 20, tick = 7 })
	local buf = Wire.packView(v)
	local chunks = Wire.chunk(buf, 1)
	ok(#chunks > 1, "chunking [a 300-entity keyframe needs more than one chunk]")
	for i, c in chunks do
		ok(
			buffer.len(c) <= Wire.MAX_CHUNK_BYTES,
			`chunking [chunk {i} is {buffer.len(c)} bytes, over the {Wire.MAX_CHUNK_BYTES} cap]`
		)
		ok(buffer.len(c) < Wire.HARD_CHUNK_LIMIT, `chunking [chunk {i} under the transport drop limit]`)
	end

	-- in-order reassembly
	local r = Wire.newReassembler()
	local out = nil
	for _, c in chunks do
		out = r:push(c) or out
	end
	ok(out ~= nil, "chunking [in-order reassembly completes]")
	same(Wire.unpackView(out), v, "chunking [reassembled message decodes to the same view]")
	eq(r.dropped, 0, "chunking [no drops on a clean stream]")

	-- out-of-order reassembly
	local r2 = Wire.newReassembler()
	local order = {}
	for i = #chunks, 1, -1 do
		order[#order + 1] = chunks[i]
	end
	local out2 = nil
	for _, c in order do
		out2 = r2:push(c) or out2
	end
	same(Wire.unpackView(out2), v, "chunking [reverse-order reassembly]")

	-- duplicate chunks are swallowed, not double-counted
	local r3 = Wire.newReassembler()
	local out3 = nil
	for _, c in chunks do
		out3 = r3:push(c) or out3
		r3:push(c)
	end
	ok(out3 ~= nil, "chunking [duplicates do not prevent completion]")
	ok(r3.stale > 0, "chunking [duplicates are counted as stale]")

	-- a missing chunk is DETECTED: the message never completes, and the next
	-- message's arrival retires the partial and increments `dropped`.
	local r4 = Wire.newReassembler({ window = 1 })
	for i, c in chunks do
		if i ~= 2 then
			eq(r4:push(c), nil, `chunking [incomplete message must not complete (chunk {i})]`)
		end
	end
	eq(r4:pending(), 1, "chunking [the incomplete message is still pending]")
	local chunks2 = Wire.chunk(Wire.packView(makeView({ enemies = 300, projectiles = 300, tick = 9 })), 2)
	local done = nil
	for _, c in chunks2 do
		done = r4:push(c) or done
	end
	ok(done ~= nil, "chunking [the following message completes]")
	eq(r4.dropped, 1, "chunking [the message with the missing chunk is counted as dropped]")

	-- single-chunk messages take the fast path and never occupy the window
	local small = Wire.packView(makeView({ enemies = 2, projectiles = 2, weapons = 1 }))
	ok(buffer.len(small) <= Wire.MAX_CHUNK_BYTES - Wire.CHUNK_HEADER_BYTES, "chunking [small view is one chunk]")
	local one = Wire.chunk(small, 3)
	eq(#one, 1, "chunking [one chunk for a small message]")
	local r5 = Wire.newReassembler()
	local got = r5:push(one[1])
	ok(got ~= nil, "chunking [single-chunk message completes immediately]")
	eq(r5:pending(), 0, "chunking [single-chunk message leaves nothing pending]")
	eq(buffer.len(got), buffer.len(small), "chunking [single-chunk payload length preserved]")

	-- malformed frames are reported, never accepted
	local r6 = Wire.newReassembler()
	local bad = buffer.create(3)
	eq(r6:push(bad), nil, "chunking [short frame rejected]")
	ok(r6.lastError ~= nil, "chunking [short frame sets lastError]")
	local badMagic = buffer.create(10)
	buffer.writeu8(badMagic, 0, 0x00)
	eq(r6:push(badMagic), nil, "chunking [bad magic rejected]")
	local badVer = buffer.create(10)
	buffer.writeu8(badVer, 0, Wire.CHUNK_MAGIC)
	buffer.writeu8(badVer, 1, 99)
	eq(r6:push(badVer), nil, "chunking [bad version rejected]")
	local badIdx = buffer.create(10)
	buffer.writeu8(badIdx, 0, Wire.CHUNK_MAGIC)
	buffer.writeu8(badIdx, 1, Wire.VERSION)
	buffer.writeu8(badIdx, 4, 5)
	buffer.writeu8(badIdx, 5, 2)
	eq(r6:push(badIdx), nil, "chunking [out-of-range index rejected]")

	-- interleaved messages within the window both complete
	local r7 = Wire.newReassembler({ window = 4 })
	local ca = Wire.chunk(Wire.packView(makeView({ enemies = 200, projectiles = 200, tick = 11 })), 10)
	local cb = Wire.chunk(Wire.packView(makeView({ enemies = 200, projectiles = 200, tick = 13, seed = 5 })), 11)
	local completed = 0
	for i = 1, math.max(#ca, #cb) do
		if ca[i] and r7:push(ca[i]) then
			completed += 1
		end
		if cb[i] and r7:push(cb[i]) then
			completed += 1
		end
	end
	eq(completed, 2, "chunking [interleaved messages both complete]")
	eq(r7.dropped, 0, "chunking [interleaving is not loss]")

	-- the API refuses a chunk size the transport would drop
	raises(function()
		return Wire.chunk(buf, 1, { maxChunkBytes = 1200 })
	end, "transport limit", "chunking [oversized maxChunkBytes rejected]")

	-- packChunked is the two calls in one
	local pc = Wire.packChunked(nil, v, 42)
	eq(#pc, #chunks, "chunking [packChunked matches chunk(pack(...))]")
end

-- ============================ 6. hostile / malformed input ============================

local function testMalformed()
	local v = makeView({ enemies = 5, projectiles = 5, weapons = 3, tick = 60 })
	local buf = Wire.packView(v)

	raises(function()
		return Wire.unpackView(buffer.create(0))
	end, "truncated", "malformed [empty buffer]")

	local badMagic = buffer.create(buffer.len(buf))
	buffer.copy(badMagic, 0, buf, 0, buffer.len(buf))
	buffer.writeu8(badMagic, 0, 0x00)
	raises(function()
		return Wire.unpackView(badMagic)
	end, "bad magic", "malformed [bad magic]")

	local badVer = buffer.create(buffer.len(buf))
	buffer.copy(badVer, 0, buf, 0, buffer.len(buf))
	buffer.writeu8(badVer, 1, 200)
	raises(function()
		return Wire.unpackView(badVer)
	end, "unsupported wire version", "malformed [bad version]")

	local truncated = buffer.create(buffer.len(buf) - 5)
	buffer.copy(truncated, 0, buf, 0, buffer.len(buf) - 5)
	raises(function()
		return Wire.unpackView(truncated)
	end, "", "malformed [truncated message]")

	local trailing = buffer.create(buffer.len(buf) + 3)
	buffer.copy(trailing, 0, buf, 0, buffer.len(buf))
	raises(function()
		return Wire.unpackView(trailing)
	end, "trailing", "malformed [trailing bytes]")

	-- a delta decoded against the wrong-length base view
	local a = makeView({ enemies = 5, tick = 10 })
	local b = makeView({ enemies = 5, tick = 12, seed = 3 })
	b.enemies = a.enemies
	local d = Wire.packDelta(a, b)
	local wrongBase = makeView({ enemies = 5, tick = 10, firstId = 5000 })
	raises(function()
		return Wire.applyDelta(wrongBase, d)
	end, "unknown enemy id", "malformed [delta against a mismatched base]")

	-- calling the wrong decoder
	raises(function()
		return Wire.unpackView(d)
	end, "delta message", "malformed [unpackView on a delta]")
	raises(function()
		return Wire.applyDelta(a, buf)
	end, "keyframe message", "malformed [applyDelta on a keyframe]")

	-- non-integer input is refused at encode time rather than silently rounded
	local frac = makeView({ enemies = 1 })
	frac.enemies[1].x = 12.5
	raises(function()
		return Wire.packView(frac)
	end, "finite integer", "malformed [non-integer coordinate]")
	local nan = makeView({ enemies = 1 })
	nan.tank.hp = 0 / 0
	raises(function()
		return Wire.packView(nan)
	end, "finite integer", "malformed [NaN]")
	local neg = makeView({ enemies = 1 })
	neg.tick = -1
	raises(function()
		return Wire.packView(neg)
	end, "non-negative", "malformed [negative tick]")
end

-- ============================ 7. size report ============================

local function stats(list: { number })
	local mn, mx, sum = math.huge, -math.huge, 0
	for _, v in list do
		mn = math.min(mn, v)
		mx = math.max(mx, v)
		sum += v
	end
	return mn, sum / #list, mx
end

local function pad(s: string, n: number): string
	if #s >= n then
		return s
	end
	return s .. string.rep(" ", n - #s)
end

local function rpad(s: string, n: number): string
	if #s >= n then
		return s
	end
	return string.rep(" ", n - #s) .. s
end

local HZ = 15

local function measure(label: string, cfg, frames: number, motion)
	local rnd = lcg(cfg.seed or 20260726)
	local v = makeView(cfg)
	local keyframe = buffer.len(Wire.packView(v))
	local keyChunks = #Wire.chunk(Wire.packView(v), 0)

	local sizes = {}
	local overCap = 0
	local spawnTotal = 0
	for _ = 1, frames do
		local nxt, spawned = advance(v, rnd, motion)
		spawnTotal += spawned
		local d = Wire.packDelta(v, nxt)
		local n = buffer.len(d)
		sizes[#sizes + 1] = n
		if n > Wire.MAX_CHUNK_BYTES then
			overCap += 1
		end
		v = nxt
	end
	local mn, avg, mx = stats(sizes)
	return {
		label = label,
		entities = #v.enemies + #v.projectiles + #v.minions,
		keyframe = keyframe,
		keyChunks = keyChunks,
		min = mn,
		avg = avg,
		max = mx,
		overCap = overCap,
		frames = frames,
		spawnsPerFrame = spawnTotal / frames,
		kbps = avg * HZ / 1024,
		maxChunks = math.ceil(mx / (Wire.MAX_CHUNK_BYTES - Wire.CHUNK_HEADER_BYTES)),
	}
end

local function sizeReport()
	print("")
	print("================ Wire size report ================")
	print(`replication rate {HZ} Hz · payload cap {Wire.MAX_CHUNK_BYTES} B · transport drop {Wire.HARD_CHUNK_LIMIT} B`)
	print("population figures from docs/10 F8 (measured off the Rust sim, not assumed)")
	print("")

	local rows = {
		measure(
			"real peak 67E/84P/20W",
			{ enemies = 67, projectiles = 84, minions = 0, weapons = 20, offers = 5 },
			200,
			{ step = 4, churn = 2 }
		),
		measure(
			"real peak, fast movers",
			{ enemies = 67, projectiles = 84, minions = 0, weapons = 20, offers = 5, seed = 99 },
			200,
			{ step = 30, churn = 2 }
		),
		measure(
			"idle / early round 8E/4P",
			{ enemies = 8, projectiles = 4, minions = 0, weapons = 3, offers = 5 },
			200,
			{ step = 4, churn = 1 }
		),
		measure(
			"300 entities total 216E/84P",
			{ enemies = 216, projectiles = 84, minions = 0, weapons = 20, offers = 5 },
			200,
			{ step = 4, churn = 4 }
		),
		measure(
			"stress 300E/300P/20M",
			{ enemies = 300, projectiles = 300, minions = 20, weapons = 20, offers = 5 },
			200,
			{ step = 4, churn = 6 }
		),
		measure(
			"stress 300E, fast movers",
			{ enemies = 300, projectiles = 300, minions = 20, weapons = 20, offers = 5, seed = 31 },
			200,
			{ step = 30, churn = 6 }
		),
	}

	print(
		pad("scenario", 26)
			.. rpad("ents", 6)
			.. rpad("keyfrm", 8)
			.. rpad("kchk", 6)
			.. rpad("dmin", 7)
			.. rpad("davg", 7)
			.. rpad("dmax", 7)
			.. rpad("B/ent", 7)
			.. rpad("chunks", 8)
			.. rpad("KB/s", 8)
	)
	print(string.rep("-", 90))
	for _, r in rows do
		print(
			pad(r.label, 26)
				.. rpad(tostring(r.entities), 6)
				.. rpad(tostring(r.keyframe), 8)
				.. rpad(tostring(r.keyChunks), 6)
				.. rpad(tostring(r.min), 7)
				.. rpad(string.format("%.0f", r.avg), 7)
				.. rpad(tostring(r.max), 7)
				.. rpad(string.format("%.2f", r.avg / r.entities), 7)
				.. rpad(tostring(r.maxChunks), 8)
				.. rpad(string.format("%.1f", r.kbps), 8)
		)
	end
	print("")
	print("keyfrm = full keyframe bytes · kchk = chunks a keyframe needs")
	print("dmin/davg/dmax = delta bytes per 15 Hz frame · B/ent = davg / live entities")
	print("chunks = chunks the WORST delta frame needs · KB/s = davg x 15 Hz, one client")
	print("")

	for _, r in rows do
		local fits = r.max <= Wire.MAX_CHUNK_BYTES
		print(
			`  {pad(r.label, 26)} worst delta {rpad(tostring(r.max), 5)} B — `
				.. (fits and "fits one 900 B payload" or `needs {r.maxChunks} chunks`)
				.. ` · {string.format("%.1f", r.kbps)} KB/s vs the ~22 KB/s budget: `
				.. (r.kbps <= 22 and "OK" or "OVER")
		)
	end
	print("")
	print(
		`  server-side for 8 arenas at the real peak: {string.format("%.1f", rows[1].kbps * 8)} KB/s `
			.. `(docs/10 F8 projects ~176 KB/s)`
	)
	print("==================================================")
	print("")
end

-- ============================ run ============================

if mode ~= "size" then
	testKeyframeRoundTrip()
	testDeltaBasics()
	testDeltaChain()
	testRebase()
	testChunking()
	testMalformed()
	print(`wire_test: {checks} checks, {failures} failure(s)`)
end

if mode ~= "test" then
	sizeReport()
end

if failures > 0 then
	error(`wire_test: {failures} failure(s)`, 0)
end
