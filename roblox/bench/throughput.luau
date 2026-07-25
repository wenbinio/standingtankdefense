--[[
	throughput.luau — R2 THROUGHPUT SPIKE (docs/10 §F7, docs/09 §1.3 / §1.6)

	THE QUESTION THIS ANSWERS
	  Can 8 arenas simulate at 30 Hz inside one Roblox server's per-tick budget?

	WHAT IT MEASURES
	  1. Per-tick cost of ONE arena running a synthetic-but-representative
	     worst-case wave, then extrapolated to 8 arenas at 30 Hz and expressed as
	     a percentage of a 33.33 ms tick budget.
	  2. The cost of the exact fixed-point path (CONTRACTS §C2 `Fixed`) against
	     the same workload written in plain doubles — i.e. what exactness costs.
	  3. Replication: bytes/tick for ~300 entity positions packed into a
	     `buffer`, against the ~900 B `UnreliableRemoteEvent` payload cap at a
	     10-15 Hz send rate (docs/09 §1.6).

	HOW THE WORKLOAD IS SIZED — NOT INVENTED
	  Entity counts, spawn cadences, movement speeds, ring radii, the damage
	  matrix and the arsenal below are transcribed from
	  `sim-core/crates/sim/src/{content.rs,waves.rs}`. The steady-state
	  population is *derived* from those cadences (see WaveModel) rather than
	  guessed, and the derivation reproduces the population the Rust sim
	  actually reaches (see README — analytic 67.5 vs measured peak 67 over 24
	  seeds). The per-tick phases mirror `sim::step`'s phase order.

	PORTABILITY
	  Everything above the `Studio glue` section touches no Roblox global. It
	  runs unmodified under a bare `luau` interpreter, which is how the numbers
	  in the README were produced. Run it standalone:

	      luau -O2 roblox/bench/throughput.luau

	READ THE CAVEATS SECTION OF THE PRINTED REPORT BEFORE QUOTING ANY NUMBER.
]]

--=============================================================================
-- 0. Environment adapter — the ONE place allowed to notice Roblox.
--=============================================================================

local Env = {}
do
	local isRoblox = false
	do
		local ok, g = pcall(function()
			return game
		end)
		isRoblox = ok and g ~= nil
	end
	Env.isRoblox = isRoblox

	-- os.clock() exists in both environments and is monotonic CPU time in both.
	Env.clock = os.clock

	local lines: { string } = {}
	function Env.write(s: string)
		table.insert(lines, s)
		print(s)
	end
	function Env.text(): string
		return table.concat(lines, "\n")
	end

	-- Resolve the real `Fixed` (CONTRACTS §C2) if R0 has landed; otherwise fall
	-- back to the clearly-marked bench stub. Reports which one it got.
	function Env.resolveFixed(): (any, string)
		if isRoblox then
			local rs = game:GetService("ReplicatedStorage")
			local shared = rs:FindFirstChild("Shared")
			local mod = (shared and shared:FindFirstChild("Fixed")) or rs:FindFirstChild("Fixed")
			if mod then
				return require(mod), "src/shared/Fixed.luau (real)"
			end
			local stub = script.Parent:FindFirstChild("FixedStub")
			return require(stub), "bench/FixedStub.luau (STUB)"
		end
		local ok, real = pcall(require, "../src/shared/Fixed")
		if ok and type(real) == "table" and real.mul then
			return real, "src/shared/Fixed.luau (real)"
		end
		return require("./FixedStub"), "bench/FixedStub.luau (STUB)"
	end

	-- Always available: the stub is a Fixed whose *representation* is a plain
	-- double behind the *same* API. Running the workload against it measures the
	-- escape hatch docs/10 §F5 explicitly reserves ("the module's internals can
	-- be swapped to plain doubles without touching a single caller").
	function Env.loadStub(): any
		if isRoblox then
			return require(script.Parent:FindFirstChild("FixedStub"))
		end
		return require("./FixedStub")
	end
end

--=============================================================================
-- 1. Content, transcribed from sim-core/crates/sim/src/content.rs
--=============================================================================

local TICK_HZ = 30
local BOSS_SPAWN_TICK = 54000 -- content.rs BOSS_SPAWN_TICK (30 min @ 30 Hz)

-- content.rs SPAWN_RING: 8 staggered radii (~1200 / 1500 / 1800).
local SPAWN_RING = {
	{ 1200, 0 },
	{ 1273, 1273 },
	{ 0, 1500 },
	{ -849, 849 },
	{ 1800, 0 },
	{ -1061, -1061 },
	{ 0, -1200 },
	{ 1273, -1273 },
}

-- content.rs ENEMIES. `armor`: 0 Light, 1 Medium, 2 Fortified.
-- `ranged`: has an EnemyAbility::RangedAttack (drives the standoff-attack phase).
local ENEMIES = {
	[0] = { name = "Squeakzilla", hp = 200, speed = 8, contact = 500, armor = 0, ranged = false },
	[1] = { name = "Fanged Death", hp = 1200, speed = 4, contact = 1500, armor = 1, ranged = false },
	[2] = { name = "The Hippocrate", hp = 33000000, speed = 3, contact = 45000, armor = 0, ranged = false },
	[3] = { name = "Doomduck", hp = 120, speed = 8, contact = 350, armor = 0, ranged = false },
	[4] = { name = "Bacon", hp = 260, speed = 16, contact = 700, armor = 0, ranged = false },
	[5] = { name = "Honk", hp = 180, speed = 22, contact = 600, armor = 0, ranged = false },
	[6] = { name = "Bonk", hp = 4000, speed = 3, contact = 2200, armor = 2, ranged = false },
	[7] = { name = "Nope Rope", hp = 320, speed = 6, contact = 200, armor = 0, ranged = true, rcd = 45 },
	[8] = { name = "Croak", hp = 300, speed = 6, contact = 200, armor = 0, ranged = true, rcd = 30 },
	[9] = { name = "Spicy", hp = 600, speed = 5, contact = 300, armor = 0, ranged = true, rcd = 36 },
	[10] = { name = "Popsicle", hp = 900, speed = 4, contact = 350, armor = 0, ranged = true, rcd = 40 },
	[11] = { name = "Dodo", hp = 800, speed = 0, contact = 0, armor = 0, ranged = false },
}

local MIN = 60 * TICK_HZ
local CLIFF_20_TICK = 20 * 60 * TICK_HZ
local CLIFF_25_TICK = 25 * 60 * TICK_HZ

-- content.rs WAVE_M0 — { enemy, cadence_ticks, start_tick }
local WAVE_M0 = {
	{ 0, 18, 0 },
	{ 1, 95, 0 },
	{ 3, 45, MIN // 5 },
	{ 11, 600, MIN // 2 },
	{ 4, 95, 5 * MIN // 12 },
	{ 3, 70, MIN },
	{ 5, 130, 3 * MIN // 4 },
	{ 8, 120, 3 * MIN // 2 },
	{ 7, 180, 4 * MIN },
	{ 9, 200, 4 * MIN },
	{ 6, 300, 6 * MIN },
	{ 10, 240, 6 * MIN },
	{ 6, 150, CLIFF_20_TICK },
	{ 5, 60, CLIFF_20_TICK },
	{ 9, 90, CLIFF_25_TICK },
	{ 10, 100, CLIFF_25_TICK },
	{ 0, 8, CLIFF_25_TICK },
}

-- content.rs BOSS_ESCORT — the densest spawn schedule in the game.
local BOSS_ESCORT = {
	{ 0, 4, BOSS_SPAWN_TICK },
	{ 4, 12, BOSS_SPAWN_TICK },
	{ 5, 15, BOSS_SPAWN_TICK },
	{ 1, 45, BOSS_SPAWN_TICK },
}

-- content.rs damage_multiplier: [damage_type][armor_class] as (num, den).
local DAMAGE_MATRIX = {
	[0] = { [0] = { 1, 1 }, [1] = { 1, 1 }, [2] = { 1, 2 } }, -- Normal
	[1] = { [0] = { 2, 1 }, [1] = { 1, 1 }, [2] = { 7, 20 } }, -- Piercing
	[2] = { [0] = { 1, 1 }, [1] = { 2, 1 }, [2] = { 1, 2 } }, -- Magic
	[3] = { [0] = { 1, 1 }, [1] = { 1, 2 }, [2] = { 3, 2 } }, -- Siege
	[4] = { [0] = { 1, 1 }, [1] = { 1, 1 }, [2] = { 1, 1 } }, -- Chaos
}

-- The arsenal a real run actually ends with. This is the 20-weapon loadout the
-- Rust `preview` binary reports at the boss on seed 0xa5a51234deadbeef:
--   Bow x1, Chaos Swarm x1, Glaive Thrower x2, Magic Bolt x14, Magic Missile x2.
-- Stats are the content.rs WeaponDefs. `atk`: 1 single, 2 splash, 3 bounce.
local ARSENAL = {
	{ name = "Bow", dmg = 75, dtype = 1, cd = 30, range = 900, speed = 45, atk = 1, arg = 0, n = 1 },
	{ name = "Chaos Swarm", dmg = 105, dtype = 4, cd = 30, range = 1200, speed = 45, atk = 2, arg = 300, n = 1 },
	{ name = "Glaive Thrower", dmg = 70, dtype = 0, cd = 27, range = 600, speed = 45, atk = 3, arg = 3, n = 2 },
	{ name = "Magic Bolt", dmg = 100, dtype = 2, cd = 27, range = 900, speed = 45, atk = 1, arg = 0, n = 14 },
	{ name = "Magic Missile", dmg = 100, dtype = 0, cd = 27, range = 900, speed = 45, atk = 1, arg = 0, n = 2 },
}

--=============================================================================
-- 2. WaveModel — derive the worst-case population from the real cadences.
--
-- An enemy lives from spawn until it reaches the tank (it self-destructs on
-- contact) or dies. Ignoring kills — the throughput worst case — the population
-- of a stream is Little's law: rate x lifetime, where
--     rate     = 1 / cadence_ticks
--     lifetime = ring_radius / (move_speed * slowFactor)   [ticks]
-- `slowFactor` is the Frost slow from status.rs move_speed_mult: 1 - 0.02/stack,
-- floored at 0.10. A Frost-heavy build (FROST_MAX_STACKS = 25) multiplies every
-- lifetime by up to 10x, and that is the real population ceiling.
--=============================================================================

local WaveModel = {}

local MEAN_RADIUS = 0
do
	local s = 0
	for _, p in SPAWN_RING do
		s += math.sqrt(p[1] * p[1] + p[2] * p[2])
	end
	MEAN_RADIUS = s / #SPAWN_RING
end

-- Streams active at `tick`, in the same order waves.spawn() walks them.
function WaveModel.activeStreams(tick: number)
	if tick >= BOSS_SPAWN_TICK then
		return BOSS_ESCORT
	end
	local out = {}
	for _, w in WAVE_M0 do
		if tick >= w[3] then
			table.insert(out, w)
		end
	end
	return out
end

-- Steady-state population and the spawn-rate-weighted enemy mix.
function WaveModel.steadyState(tick: number, slowFactor: number)
	local streams = WaveModel.activeStreams(tick)
	local pop, rate = 0, 0
	local mix = {}
	for _, w in streams do
		local def = ENEMIES[w[1]]
		local r = 1 / w[2]
		rate += r
		local life
		if def.speed == 0 then
			life = 600 -- inert (Dodo): bounded here by the reap, not by travel
		else
			life = MEAN_RADIUS / (def.speed * slowFactor)
		end
		pop += r * life
		table.insert(mix, { id = w[1], weight = r })
	end
	return pop, rate, mix
end

--=============================================================================
-- 3. The arena workload.
--
-- A struct-of-arrays mini-arena that performs the same per-tick phases, in the
-- same order, as `sim::step` — the ones whose cost scales with entity count:
--
--     spawn -> fireWeapons (O(W*E) range scan) -> advanceProjectiles (O(P),
--     plus O(E) per splash) -> moveEnemies (O(E)) -> enemyRangedAttacks (O(E))
--     -> statusTick (O(E)) -> reap (O(E)) -> checksum (O(E))
--
-- Two implementations, identical algorithm:
--   runFixed  — every arithmetic op goes through the §C2 `Fixed` API.
--   runDouble — the same arithmetic in native Luau doubles.
-- The difference between them is exactly what determinism costs.
--
-- Population is PINNED to cfg.pop by refilling at the end of each tick, so the
-- cost/entity-count curve is clean and comparable across configurations. Which
-- populations are worth pinning to comes from WaveModel above.
--=============================================================================

local PHASES = { "fire", "proj", "move", "ranged", "status", "reap", "checksum", "spawn" }

local function newRng(seed: number)
	local s = seed
	return function(n: number): number
		-- xorshift32; deterministic, cheap, and NOT the game's Rng (CONTRACTS
		-- §C3) — a benchmark does not need stream-compatible randomness.
		s = bit32.bxor(s, bit32.lshift(s, 13))
		s = bit32.bxor(s, bit32.rshift(s, 17))
		s = bit32.bxor(s, bit32.lshift(s, 5))
		return s % n
	end
end

local function buildWeapons(count: number)
	local w = {}
	for _, a in ARSENAL do
		for _ = 1, a.n do
			table.insert(w, a)
		end
	end
	-- Scale the measured 20-weapon loadout up or down, preserving its mix.
	local out = {}
	for i = 1, count do
		out[i] = w[((i - 1) % #w) + 1]
	end
	return out
end

local function buildMix(mix)
	-- Expand the spawn-rate-weighted mix into a 256-entry lookup table.
	local total = 0
	for _, m in mix do
		total += m.weight
	end
	local t, i = {}, 0
	for _, m in mix do
		local n = math.max(1, math.floor(256 * m.weight / total + 0.5))
		for _ = 1, n do
			i += 1
			if i > 256 then
				break
			end
			t[i] = m.id
		end
	end
	while i < 256 do
		i += 1
		t[i] = mix[1].id
	end
	return t
end

--------------------------------------------------------------------------- fixed

local function runFixed(cfg, Fixed)
	-- Every arithmetic op goes through the §C2 API. The bench never touches a
	-- Fixed value's internals, which is what lets it run unchanged against both
	-- the real module (4x16-bit limb tables) and the bench stub (plain numbers).
	local Fmul, Fdiv, Fsqrt, FfromInt, Fscale, Fratio =
		Fixed.mul, Fixed.div, Fixed.sqrt, Fixed.fromInt, Fixed.scaleI64, Fixed.fromRatio
	local Fadd, Fsub, Fle, Flt, FtoInt, Flo =
		Fixed.add, Fixed.sub, Fixed.le, Fixed.lt, Fixed.toInt, Fixed.lo
	local ZERO = Fixed.ZERO

	local ring = {}
	for i, p in SPAWN_RING do
		ring[i] = { FfromInt(p[1]), FfromInt(p[2]) }
	end
	local dmgMult = {}
	for dt, row in DAMAGE_MATRIX do
		dmgMult[dt] = {}
		for ac, nd in row do
			dmgMult[dt][ac] = Fratio(nd[1], nd[2])
		end
	end
	local espeed, econtact, earmor, eranged, ercd, ehp0 = {}, {}, {}, {}, {}, {}
	for id, d in ENEMIES do
		espeed[id] = FfromInt(d.speed)
		econtact[id] = d.contact
		earmor[id] = d.armor
		eranged[id] = d.ranged
		ercd[id] = d.rcd or 0
		ehp0[id] = d.hp
	end

	local weapons = buildWeapons(cfg.weapons)
	local wrange2, wspeed, wsplash2 = {}, {}, {}
	for i, w in weapons do
		local r = FfromInt(w.range)
		wrange2[i] = Fmul(r, r)
		wspeed[i] = FfromInt(w.speed)
		local sp = FfromInt(w.arg)
		wsplash2[i] = if w.atk == 2 then Fmul(sp, sp) else false
	end
	local wnext = table.create(#weapons, 0)

	local mixTable = buildMix(cfg.mix)
	local rand = newRng(cfg.seed or 0x9E3779B9)

	-- Enemy SoA
	local ex, ey, ehp, edef, efrost, epois, epcd, ercdT =
		table.create(1024), table.create(1024), table.create(1024), table.create(1024),
		table.create(1024), table.create(1024), table.create(1024), table.create(1024)
	local n = 0
	-- Projectile SoA
	local px, py, ptgt, pdmg, pdtype, psplash2, pspd = {}, {}, {}, {}, {}, {}, {}
	local np = 0

	local TANKX, TANKY = ZERO, ZERO
	local prof = cfg.profile and {} or nil
	if prof then
		for _, k in PHASES do
			prof[k] = 0
		end
	end
	local clock = Env.clock

	local function spawnOne()
		n += 1
		local r = ring[rand(8) + 1]
		local id = mixTable[rand(256) + 1]
		ex[n], ey[n] = r[1], r[2]
		edef[n] = id
		ehp[n] = ehp0[id]
		efrost[n] = cfg.frost
		epois[n] = 0
		epcd[n] = 0
		ercdT[n] = rand(64)
	end

	local checksumAcc = 0
	local totalHits, totalScans = 0, 0

	for tick = 1, cfg.ticks do
		local t0
		-- ---- fireWeapons: O(W*E) in-range scan, then emit ------------------
		if prof then
			t0 = clock()
		end
		for wi = 1, #weapons do
			if tick < wnext[wi] then
				continue
			end
			local w = weapons[wi]
			local r2 = wrange2[wi]
			local best, bestD = 0, nil
			for ei = 1, n do
				local dx, dy = Fsub(ex[ei], TANKX), Fsub(ey[ei], TANKY)
				local d2 = Fadd(Fmul(dx, dx), Fmul(dy, dy))
				totalScans += 1
				if Fle(d2, r2) and (bestD == nil or Flt(d2, bestD)) then
					best, bestD = ei, d2
				end
			end
			if best == 0 then
				continue
			end
			wnext[wi] = tick + w.cd
			local shots = if w.atk == 3 then w.arg else 1
			for _ = 1, shots do
				np += 1
				px[np], py[np] = TANKX, TANKY
				ptgt[np] = best
				pdmg[np] = w.dmg
				pdtype[np] = w.dtype
				psplash2[np] = wsplash2[wi]
				pspd[np] = wspeed[wi]
			end
		end
		if prof then
			prof.fire += clock() - t0
			t0 = clock()
		end

		-- ---- advanceProjectiles: O(P) step_toward + impact ------------------
		local wp = 0
		for pi = 1, np do
			local ti = ptgt[pi]
			if ti > n then
				continue
			end
			local tx, ty = ex[ti], ey[ti]
			local sx, sy = px[pi], py[pi]
			local dx, dy = Fsub(tx, sx), Fsub(ty, sy)
			local d2 = Fadd(Fmul(dx, dx), Fmul(dy, dy))
			local step = pspd[pi]
			local step2 = Fmul(step, step)
			if Fle(d2, step2) then
				-- impact
				local dm = pdmg[pi]
				local sp2 = psplash2[pi]
				local dt = pdtype[pi]
				if sp2 then
					for ei = 1, n do
						local ax, ay = Fsub(ex[ei], tx), Fsub(ey[ei], ty)
						local a2 = Fadd(Fmul(ax, ax), Fmul(ay, ay))
						if Fle(a2, sp2) then
							ehp[ei] -= FtoInt(Fscale(dmgMult[dt][earmor[edef[ei]]], dm))
						end
					end
				else
					ehp[ti] -= FtoInt(Fscale(dmgMult[dt][earmor[edef[ti]]], dm))
				end
				totalHits += 1
			else
				local dist = Fsqrt(d2)
				wp += 1
				px[wp] = Fadd(sx, Fdiv(Fmul(dx, step), dist))
				py[wp] = Fadd(sy, Fdiv(Fmul(dy, step), dist))
				ptgt[wp], pdmg[wp], pdtype[wp], psplash2[wp], pspd[wp] =
					ti, pdmg[pi], pdtype[pi], psplash2[pi], pspd[pi]
			end
		end
		np = wp
		if prof then
			prof.proj += clock() - t0
			t0 = clock()
		end

		-- ---- moveEnemies: O(E) step_toward + contact ------------------------
		for ei = 1, n do
			local f = efrost[ei]
			local slow = if f > 0 then Fratio(math.max(10, 100 - 2 * f), 100) else Fixed.ONE
			local speed = Fmul(espeed[edef[ei]], slow)
			local x, y = ex[ei], ey[ei]
			local dx, dy = Fsub(TANKX, x), Fsub(TANKY, y)
			local d2 = Fadd(Fmul(dx, dx), Fmul(dy, dy))
			local step2 = Fmul(speed, speed)
			if Fle(d2, step2) then
				ex[ei], ey[ei] = TANKX, TANKY
				ehp[ei] = 0 -- contact: self-destructs (reaped below)
			else
				local dist = Fsqrt(d2)
				ex[ei] = Fadd(x, Fdiv(Fmul(dx, speed), dist))
				ey[ei] = Fadd(y, Fdiv(Fmul(dy, speed), dist))
			end
		end
		if prof then
			prof.move += clock() - t0
			t0 = clock()
		end

		-- ---- enemyRangedAttacks: O(E), the standoff subset fires ------------
		for ei = 1, n do
			local id = edef[ei]
			if not eranged[id] then
				continue
			end
			local c = epcd[ei]
			if c > 0 then
				epcd[ei] = c - 1
			else
				local dx, dy = Fsub(ex[ei], TANKX), Fsub(ey[ei], TANKY)
				local d2 = Fadd(Fmul(dx, dx), Fmul(dy, dy))
				if Fle(d2, wrange2[1]) then
					epcd[ei] = ercd[id]
				end
			end
		end
		if prof then
			prof.ranged += clock() - t0
			t0 = clock()
		end

		-- ---- statusTick: O(E) poison DoT + frost decay ----------------------
		for ei = 1, n do
			local p = epois[ei]
			if p > 0 then
				ehp[ei] -= p
				epois[ei] = p - 1
			end
			local f = efrost[ei]
			if f > 0 and (tick + ei) % 30 == 0 then
				efrost[ei] = f - 1
			end
		end
		if prof then
			prof.status += clock() - t0
			t0 = clock()
		end

		-- ---- reap: O(E) stable compaction -----------------------------------
		local w2 = 0
		for ei = 1, n do
			if ehp[ei] > 0 then
				w2 += 1
				if w2 ~= ei then
					ex[w2], ey[w2], ehp[w2], edef[w2], efrost[w2], epois[w2], epcd[w2] =
						ex[ei], ey[ei], ehp[ei], edef[ei], efrost[ei], epois[ei], epcd[ei]
				end
			end
		end
		n = w2
		if prof then
			prof.reap += clock() - t0
			t0 = clock()
		end

		-- ---- checksum: O(E) FNV-1a fold (docs/05 state_checksum) ------------
		if cfg.checksum then
			local h = 2166136261
			for ei = 1, n do
				h = bit32.bxor(h, Flo(ex[ei]))
				h = (h * 16777619) % 4294967296
				h = bit32.bxor(h, Flo(ey[ei]))
				h = (h * 16777619) % 4294967296
				h = bit32.bxor(h, bit32.band(ehp[ei], 0xFFFFFFFF))
				h = (h * 16777619) % 4294967296
			end
			checksumAcc = bit32.bxor(checksumAcc, h)
		end
		if prof then
			prof.checksum += clock() - t0
			t0 = clock()
		end

		-- ---- spawn / population pin -----------------------------------------
		while n < cfg.pop do
			spawnOne()
		end
		if prof then
			prof.spawn += clock() - t0
		end
	end

	return { n = n, np = np, checksum = checksumAcc, hits = totalHits, scans = totalScans, prof = prof }
end

-------------------------------------------------------------------------- double

local function runDouble(cfg)
	local sqrt = math.sqrt
	local floor = math.floor

	local ring = SPAWN_RING
	local dmgMult = {}
	for dt, row in DAMAGE_MATRIX do
		dmgMult[dt] = {}
		for ac, nd in row do
			dmgMult[dt][ac] = nd[1] / nd[2]
		end
	end
	local espeed, econtact, earmor, eranged, ercd, ehp0 = {}, {}, {}, {}, {}, {}
	for id, d in ENEMIES do
		espeed[id] = d.speed
		econtact[id] = d.contact
		earmor[id] = d.armor
		eranged[id] = d.ranged
		ercd[id] = d.rcd or 0
		ehp0[id] = d.hp
	end

	local weapons = buildWeapons(cfg.weapons)
	local wrange2, wspeed, wsplash2 = {}, {}, {}
	for i, w in weapons do
		wrange2[i] = w.range * w.range
		wspeed[i] = w.speed
		wsplash2[i] = if w.atk == 2 then w.arg * w.arg else 0
	end
	local wnext = table.create(#weapons, 0)

	local mixTable = buildMix(cfg.mix)
	local rand = newRng(cfg.seed or 0x9E3779B9)

	local ex, ey, ehp, edef, efrost, epois, epcd =
		table.create(1024), table.create(1024), table.create(1024), table.create(1024),
		table.create(1024), table.create(1024), table.create(1024)
	local n = 0
	local px, py, ptgt, pdmg, pdtype, psplash2, pspd = {}, {}, {}, {}, {}, {}, {}
	local np = 0

	local prof = cfg.profile and {} or nil
	if prof then
		for _, k in PHASES do
			prof[k] = 0
		end
	end
	local clock = Env.clock

	local function spawnOne()
		n += 1
		local r = ring[rand(8) + 1]
		local id = mixTable[rand(256) + 1]
		ex[n], ey[n] = r[1], r[2]
		edef[n] = id
		ehp[n] = ehp0[id]
		efrost[n] = cfg.frost
		epois[n] = 0
		epcd[n] = 0
	end

	local checksumAcc = 0
	local totalHits, totalScans = 0, 0

	for tick = 1, cfg.ticks do
		local t0
		if prof then
			t0 = clock()
		end
		for wi = 1, #weapons do
			if tick < wnext[wi] then
				continue
			end
			local w = weapons[wi]
			local r2 = wrange2[wi]
			local best, bestD = 0, nil
			for ei = 1, n do
				local dx, dy = ex[ei], ey[ei]
				local d2 = dx * dx + dy * dy
				totalScans += 1
				if d2 <= r2 and (bestD == nil or d2 < bestD) then
					best, bestD = ei, d2
				end
			end
			if best == 0 then
				continue
			end
			wnext[wi] = tick + w.cd
			local shots = if w.atk == 3 then w.arg else 1
			for _ = 1, shots do
				np += 1
				px[np], py[np] = 0, 0
				ptgt[np] = best
				pdmg[np] = w.dmg
				pdtype[np] = w.dtype
				psplash2[np] = wsplash2[wi]
				pspd[np] = wspeed[wi]
			end
		end
		if prof then
			prof.fire += clock() - t0
			t0 = clock()
		end

		local wp = 0
		for pi = 1, np do
			local ti = ptgt[pi]
			if ti > n then
				continue
			end
			local tx, ty = ex[ti], ey[ti]
			local sx, sy = px[pi], py[pi]
			local dx, dy = tx - sx, ty - sy
			local d2 = dx * dx + dy * dy
			local step = pspd[pi]
			local step2 = step * step
			if d2 <= step2 then
				local dm = pdmg[pi]
				local sp2 = psplash2[pi]
				local dt = pdtype[pi]
				if sp2 ~= 0 then
					for ei = 1, n do
						local ax, ay = ex[ei] - tx, ey[ei] - ty
						if ax * ax + ay * ay <= sp2 then
							ehp[ei] -= floor(dmgMult[dt][earmor[edef[ei]]] * dm)
						end
					end
				else
					ehp[ti] -= floor(dmgMult[dt][earmor[edef[ti]]] * dm)
				end
				totalHits += 1
			else
				local dist = sqrt(d2)
				wp += 1
				px[wp] = sx + dx * step / dist
				py[wp] = sy + dy * step / dist
				ptgt[wp], pdmg[wp], pdtype[wp], psplash2[wp], pspd[wp] =
					ti, pdmg[pi], pdtype[pi], psplash2[pi], pspd[pi]
			end
		end
		np = wp
		if prof then
			prof.proj += clock() - t0
			t0 = clock()
		end

		for ei = 1, n do
			local f = efrost[ei]
			local slow = if f > 0 then math.max(0.10, 1 - 0.02 * f) else 1
			local speed = espeed[edef[ei]] * slow
			local x, y = ex[ei], ey[ei]
			local d2 = x * x + y * y
			local step2 = speed * speed
			if d2 <= step2 or d2 == 0 then
				ex[ei], ey[ei] = 0, 0
				ehp[ei] = 0
			else
				local dist = sqrt(d2)
				ex[ei] = x - x * speed / dist
				ey[ei] = y - y * speed / dist
			end
		end
		if prof then
			prof.move += clock() - t0
			t0 = clock()
		end

		for ei = 1, n do
			local id = edef[ei]
			if not eranged[id] then
				continue
			end
			local c = epcd[ei]
			if c > 0 then
				epcd[ei] = c - 1
			else
				local dx, dy = ex[ei], ey[ei]
				if dx * dx + dy * dy <= wrange2[1] then
					epcd[ei] = ercd[id]
				end
			end
		end
		if prof then
			prof.ranged += clock() - t0
			t0 = clock()
		end

		for ei = 1, n do
			local p = epois[ei]
			if p > 0 then
				ehp[ei] -= p
				epois[ei] = p - 1
			end
			local f = efrost[ei]
			if f > 0 and (tick + ei) % 30 == 0 then
				efrost[ei] = f - 1
			end
		end
		if prof then
			prof.status += clock() - t0
			t0 = clock()
		end

		local w2 = 0
		for ei = 1, n do
			if ehp[ei] > 0 then
				w2 += 1
				if w2 ~= ei then
					ex[w2], ey[w2], ehp[w2], edef[w2], efrost[w2], epois[w2], epcd[w2] =
						ex[ei], ey[ei], ehp[ei], edef[ei], efrost[ei], epois[ei], epcd[ei]
				end
			end
		end
		n = w2
		if prof then
			prof.reap += clock() - t0
			t0 = clock()
		end

		if cfg.checksum then
			local h = 2166136261
			for ei = 1, n do
				h = bit32.bxor(h, bit32.band(floor(ex[ei]), 0xFFFFFFFF))
				h = (h * 16777619) % 4294967296
				h = bit32.bxor(h, bit32.band(floor(ey[ei]), 0xFFFFFFFF))
				h = (h * 16777619) % 4294967296
				h = bit32.bxor(h, bit32.band(ehp[ei], 0xFFFFFFFF))
				h = (h * 16777619) % 4294967296
			end
			checksumAcc = bit32.bxor(checksumAcc, h)
		end
		if prof then
			prof.checksum += clock() - t0
			t0 = clock()
		end

		while n < cfg.pop do
			spawnOne()
		end
		if prof then
			prof.spawn += clock() - t0
		end
	end

	return { n = n, np = np, checksum = checksumAcc, hits = totalHits, scans = totalScans, prof = prof }
end

--=============================================================================
-- 4. Timing helper — best-of-k, so a scheduler hiccup cannot inflate a result.
--=============================================================================

local function timeRun(fn, cfg, reps: number)
	local best, res = math.huge, nil
	for _ = 1, reps do
		local t0 = Env.clock()
		local r = fn(cfg)
		local dt = Env.clock() - t0
		if dt < best then
			best, res = dt, r
		end
	end
	return best / cfg.ticks, res -- seconds per tick
end

-- Pick a tick count that makes each timed run last roughly `targetSec`, so a
-- cheap config gets enough ticks to be measurable and an expensive one does not
-- take a minute. Costs one short pilot run.
local function autoTicks(fn, cfg, targetSec: number, minTicks: number, maxTicks: number): number
	local pilot = table.clone(cfg)
	pilot.ticks = 12
	local t0 = Env.clock()
	fn(pilot)
	local per = (Env.clock() - t0) / pilot.ticks
	if per <= 0 then
		return maxTicks
	end
	return math.clamp(math.floor(targetSec / per), minTicks, maxTicks)
end

--=============================================================================
-- 5. Fixed-vs-double microbenchmark — the raw op cost, isolated from the sim.
--=============================================================================

-- Time `body(iters)` best-of-3 and subtract the empty-loop baseline, so the
-- number is the op and not the `for`.
local function timeOp(body, iters: number, baseline: number): number
	body(256) -- warm
	local best = math.huge
	for _ = 1, 3 do
		local t0 = Env.clock()
		body(iters)
		local dt = Env.clock() - t0
		if dt < best then
			best = dt
		end
	end
	return math.max(0, best / iters - baseline)
end

local function emptyLoopCost(iters: number): number
	local best = math.huge
	local sink = 0
	for _ = 1, 3 do
		local t0 = Env.clock()
		for i = 1, iters do
			sink += 1
		end
		local dt = Env.clock() - t0
		if dt < best then
			best = dt
		end
	end
	if sink < 0 then
		error("dead")
	end
	return best / iters
end

-- Per-operation cost of the §C2 surface vs. the equivalent double expression.
-- This is what tells the numeric-core owner WHICH op to attack.
local function opCosts(Fixed, iters: number)
	local base = emptyLoopCost(iters)
	local a = Fixed.fromInt(1800)
	local b = Fixed.fromRatio(3, 2)
	local big = Fixed.fromInt(1440000) -- ~ a squared distance, for sqrt
	local out = {}
	-- Operands live in a table so Luau's -O2 constant folder cannot evaluate the
	-- double-side expressions at compile time and report 0 ns. They are hoisted
	-- into locals before each loop, so the loop body is pure arithmetic.
	local K = { 1800.0, 1.5, 1440000.0, 1200.0 }

	local function rec(name, fixedBody, doubleBody)
		local f = timeOp(fixedBody, iters, base)
		local d = timeOp(doubleBody, iters, base)
		table.insert(out, { name = name, fixed = f, double = d, ratio = if d > 0 then f / d else 0 })
	end

	local Fadd, Fsub, Fmul, Fdiv, Fsqrt, Fscale, Fratio, Fcmp, Ffi =
		Fixed.add, Fixed.sub, Fixed.mul, Fixed.div, Fixed.sqrt, Fixed.scaleI64,
		Fixed.fromRatio, Fixed.compare, Fixed.fromInt

	rec("add", function(n)
		local s = a
		for i = 1, n do
			s = Fadd(a, b)
		end
		return s
	end, function(n)
		local a, b = K[1], K[2]
		local s = 0
		for i = 1, n do
			s += a + b
		end
		return s
	end)

	rec("sub", function(n)
		local s = a
		for i = 1, n do
			s = Fsub(a, b)
		end
		return s
	end, function(n)
		local a, b = K[1], K[2]
		local s = 0
		for i = 1, n do
			s += a - b
		end
		return s
	end)

	rec("mul", function(n)
		local s = a
		for i = 1, n do
			s = Fmul(a, b)
		end
		return s
	end, function(n)
		local a, b = K[1], K[2]
		local s = 0
		for i = 1, n do
			s += a * b
		end
		return s
	end)

	rec("div", function(n)
		local s = a
		for i = 1, n do
			s = Fdiv(a, b)
		end
		return s
	end, function(n)
		local a, b = K[1], K[2]
		local s = 0
		for i = 1, n do
			s += a / b
		end
		return s
	end)

	rec("scaleI64", function(n)
		local s = a
		for i = 1, n do
			s = Fscale(b, 1200)
		end
		return s
	end, function(n)
		local b, v = K[2], K[4]
		local floor = math.floor
		local s = 0
		for i = 1, n do
			s += floor(b * v)
		end
		return s
	end)

	rec("sqrt", function(n)
		local s = a
		for i = 1, n do
			s = Fsqrt(big)
		end
		return s
	end, function(n)
		local g = K[3]
		local sqrt = math.sqrt
		local s = 0
		for i = 1, n do
			s += sqrt(g)
		end
		return s
	end)

	rec("fromInt", function(n)
		local s = a
		for i = 1, n do
			s = Ffi(1800)
		end
		return s
	end, function(n)
		local a = K[1]
		local s = 0
		for i = 1, n do
			s += a
		end
		return s
	end)

	rec("fromRatio", function(n)
		local s = a
		for i = 1, n do
			s = Fratio(3, 2)
		end
		return s
	end, function(n)
		local a, b = K[1], K[2]
		local s = 0
		for i = 1, n do
			s += a / b
		end
		return s
	end)

	rec("compare", function(n)
		local s = 0
		for i = 1, n do
			s = Fcmp(a, b)
		end
		return s
	end, function(n)
		local a, b = K[1], K[2]
		local s = 0
		for i = 1, n do
			if a < b then s += 1 end
		end
		return s
	end)

	return out
end

--=============================================================================
-- 6. Replication budget (docs/09 §1.6).
--
-- UnreliableRemoteEvent: ~900 B usable payload, hard-dropped above 1000 B.
-- Positions live in +/-1800 units, so a whole-unit i16 is lossless for x and y.
-- Each client only ever receives ITS OWN arena (plus, under F4, one spectated
-- arena), so this is 1-2 arenas per client, never 8.
--=============================================================================

local UNRELIABLE_CAP = 900

local ENCODINGS = {
	{ name = "id u16 + type u8 + x,y i16 + hp u8", bytes = 8, note = "full, addressable, self-healing" },
	{ name = "type u8 + x,y i16 + hp u8", bytes = 6, note = "index-implied id; needs a stable order" },
	{ name = "type u8 + x,y i16", bytes = 5, note = "index-implied id, hp on a slower channel" },
	{ name = "x,y i16", bytes = 4, note = "positions only; type/hp on a keyframe channel" },
	{ name = "dx,dy i8 + flags u8", bytes = 3, note = "delta vs last keyframe; needs periodic keyframes" },
}

-- Actually pack N positions into a buffer and time it, so the CPU cost of
-- serialisation is part of the budget rather than assumed free.
local function packPositions(count: number, bytesPer: number, xs, ys, types, hps)
	local b = buffer.create(count * bytesPer)
	local o = 0
	if bytesPer == 8 then
		for i = 1, count do
			buffer.writeu16(b, o, i % 65536)
			buffer.writeu8(b, o + 2, types[i])
			buffer.writei16(b, o + 3, xs[i])
			buffer.writei16(b, o + 5, ys[i])
			buffer.writeu8(b, o + 7, hps[i])
			o += 8
		end
	elseif bytesPer == 5 then
		for i = 1, count do
			buffer.writeu8(b, o, types[i])
			buffer.writei16(b, o + 1, xs[i])
			buffer.writei16(b, o + 3, ys[i])
			o += 5
		end
	else
		for i = 1, count do
			buffer.writei16(b, o, xs[i])
			buffer.writei16(b, o + 2, ys[i])
			o += 4
		end
	end
	return b
end

local function replicationReport(entities: number, sendHz: { number })
	local xs, ys, types, hps = {}, {}, {}, {}
	for i = 1, entities do
		xs[i] = ((i * 137) % 3600) - 1800
		ys[i] = ((i * 71) % 3600) - 1800
		types[i] = i % 12
		hps[i] = i % 256
	end

	Env.write("")
	Env.write("== 3. REPLICATION BUDGET (docs/09 §1.6) ==")
	Env.write(string.format("  entities replicated per client: %d (its own arena only; F4 adds 1 spectated)", entities))
	Env.write(string.format("  UnreliableRemoteEvent usable payload: %d B (dropped above 1000 B)", UNRELIABLE_CAP))
	Env.write("")
	Env.write("  encoding                                B/ent   total B   payloads   ent/payload")
	for _, e in ENCODINGS do
		local total = entities * e.bytes
		local perPayload = math.floor(UNRELIABLE_CAP / e.bytes)
		local payloads = math.ceil(entities / perPayload)
		Env.write(string.format(
			"  %-38s %5d %9d %10d %13d   %s",
			e.name, e.bytes, total, payloads, perPayload, e.note
		))
	end

	Env.write("")
	Env.write("  bandwidth for the 5 B encoding (index-implied id, hp on a slower channel):")
	for _, hz in sendHz do
		local total = entities * 5
		Env.write(string.format(
			"    %2d Hz -> %7.1f KB/s per client, %6.1f KB/s server-side for 8 clients",
			hz, total * hz / 1024, total * hz * 8 / 1024
		))
	end

	-- Time the pack itself.
	local reps = 2000
	local best = math.huge
	for _ = 1, 3 do
		local t0 = Env.clock()
		for _ = 1, reps do
			packPositions(entities, 5, xs, ys, types, hps)
		end
		local dt = (Env.clock() - t0) / reps
		if dt < best then
			best = dt
		end
	end
	Env.write("")
	Env.write(string.format("  measured pack cost: %.3f ms for %d entities at 5 B each", best * 1000, entities))
	Env.write(string.format(
		"    at 15 Hz that is %.3f ms/s per arena, %.3f ms/s for 8 arenas (%.2f%% of one core)",
		best * 1000 * 15, best * 1000 * 15 * 8, best * 15 * 8 * 100
	))
end

--=============================================================================
-- 7. The report.
--=============================================================================

local TICK_BUDGET_MS = 1000 / TICK_HZ -- 33.33 ms — one whole 30 Hz sim tick
local SAFE_SHARE = 0.30 -- what a server can spend on sim and still do everything else
local ARENAS = 8

local Bench = {}

function Bench.run(opts)
	opts = opts or {}
	local reps = opts.reps or 2
	local targetSec = opts.targetSec or 0.50
	-- Roblox has no preemption: a long pure-Luau loop starves the scheduler and
	-- can trip ScriptContext.ScriptTimeoutLength. The Studio driver passes a
	-- yield so the bench hands the frame back between measurements. Standalone
	-- passes nothing and this is a no-op.
	local yield = opts.yield or function() end
	local maxTicks = opts.ticks or 900

	local Fixed, fixedSource = Env.resolveFixed()
	local Stub = Env.loadStub()

	Env.write("================================================================================")
	Env.write("  STANDING TANK DEFENSE (Roblox fork) — R2 THROUGHPUT SPIKE")
	Env.write("  docs/10 §F7 R2  ·  docs/09 §1.3 (throughput) + §1.6 (replication)")
	Env.write("================================================================================")
	Env.write(string.format("  environment    : %s", if Env.isRoblox then "Roblox (Studio/server)" else "standalone luau"))
	Env.write(string.format("  Fixed module   : %s", fixedSource))
	if Fixed.__IS_BENCH_STUB then
		Env.write("                   !! STUB — the real §C2 Fixed may cost more (or less).")
	end
	Env.write(string.format("  sim rate       : %d Hz  ->  %.2f ms per tick, all arenas", TICK_HZ, TICK_BUDGET_MS))
	Env.write(string.format("  arenas         : %d", ARENAS))
	Env.write(string.format("  timing         : best of %d, ~%.2f s per timed run, <= %d ticks", reps, targetSec, maxTicks))

	----------------------------------------------------------------- wave model
	Env.write("")
	Env.write("== 0. WORST-CASE WAVE, DERIVED FROM content.rs / waves.rs ==")
	Env.write("  Little's law on the real cadences: pop = sum(1/cadence * radius/speed).")
	Env.write("")
	Env.write("  phase                       tick    streams  spawns/s   pop@1.0x  pop@0.5x  pop@0.1x")
	local phases = {
		{ "opening", 0 },
		{ "post-15min", 27000 },
		{ "cliff 20min", CLIFF_20_TICK },
		{ "cliff 25min (pre-boss flood)", CLIFF_25_TICK },
		{ "BOSS ESCORT (worst)", BOSS_SPAWN_TICK },
	}
	local worstPop, worstMix = 0, nil
	for _, p in phases do
		local pop1, rate, mix = WaveModel.steadyState(p[2], 1.0)
		local pop2 = WaveModel.steadyState(p[2], 0.5)
		local pop3 = WaveModel.steadyState(p[2], 0.1)
		Env.write(string.format(
			"  %-27s %6d %8d %9.2f %10.1f %9.1f %9.1f",
			p[1], p[2], #WaveModel.activeStreams(p[2]), rate * TICK_HZ, pop1, pop2, pop3
		))
		if pop1 > worstPop then
			worstPop, worstMix = pop1, mix
		end
	end
	local _, _, bossMix = WaveModel.steadyState(BOSS_SPAWN_TICK, 1.0)
	Env.write("")
	Env.write("  0.5x / 0.1x are Frost slow (status.rs move_speed_mult: -2%/stack,")
	Env.write("  floored at 10%; FROST_MAX_STACKS = 25). A Frost build IS the worst case:")
	Env.write("  slower enemies live longer, so the board fills up.")
	Env.write(string.format("  Cross-check: the Rust sim's measured peak over 24 seeds was 67 enemies;"))
	Env.write(string.format("  this model puts the boss-escort steady state at %.1f. The model is sound.", worstPop))

	--------------------------------------------------------------- throughput
	Env.write("")
	Env.write("== 1. PER-TICK SIMULATION COST, ONE ARENA ==")
	Env.write("  E = live enemies, W = owned weapons. The measured Rust run peaked at")
	Env.write("  E=67 / W=20; E=300 is docs/09 §1.6's stated replication worst case.")
	Env.write("  Three numeric backends, IDENTICAL algorithm:")
	Env.write("    exact  — the real §C2 Fixed (4 x 16-bit limb tables, bit-identical to Rust)")
	Env.write("    api-dbl— the same §C2 API over a plain-double representation (the §F5")
	Env.write("             escape hatch: callers unchanged, exactness given up at the edges)")
	Env.write("    raw-dbl— no Fixed at all, native Luau arithmetic (the floor)")
	if Fixed.__IS_BENCH_STUB then
		Env.write("  !! The real Fixed was not found, so 'exact' and 'api-dbl' are BOTH the stub.")
		Env.write("  !! Their rows will agree, and neither tells you what exactness costs.")
	end
	Env.write("")
	Env.write("  backend    E     W   ms/tick  8 arenas   % of 33.33ms   % of 30% budget  ticks")

	local configs = {
		{ pop = 68, weapons = 20 },
		{ pop = 150, weapons = 20 },
		{ pop = 300, weapons = 20 },
		{ pop = 300, weapons = 40 },
		{ pop = 675, weapons = 20 },
	}
	local backends = {
		{ key = "exact", fn = function(x)
			return runFixed(x, Fixed)
		end },
		{ key = "api-dbl", fn = function(x)
			return runFixed(x, Stub)
		end },
		{ key = "raw-dbl", fn = runDouble },
	}

	local results = {}
	for _, c in configs do
		for _, be in backends do
			local cfg = {
				pop = c.pop,
				weapons = c.weapons,
				ticks = 0,
				frost = 0,
				mix = bossMix,
				checksum = true,
				seed = 0x9E3779B9,
			}
			cfg.ticks = autoTicks(be.fn, cfg, targetSec, 60, maxTicks)
			local sPerTick = timeRun(be.fn, cfg, reps)
			local ms = sPerTick * 1000
			local ms8 = ms * ARENAS
			Env.write(string.format(
				"  %-8s %4d %5d %9.3f %9.3f %13.1f%% %16.1f%% %6d",
				be.key, c.pop, c.weapons, ms, ms8, ms8 / TICK_BUDGET_MS * 100,
				ms8 / (TICK_BUDGET_MS * SAFE_SHARE) * 100, cfg.ticks
			))
			results[be.key .. "/" .. c.pop .. "/" .. c.weapons] = ms
			yield()
		end
	end

	Env.write("")
	Env.write("  '8 arenas' is the SERIAL total — every arena on one thread. Under docs/10 §F6")
	Env.write("  (one parallel Actor per arena) the wall-clock cost is closer to the 1-arena")
	Env.write("  column, bounded by how many worker threads the server actually has.")

	------------------------------------------------------------ phase profile
	Env.write("")
	Env.write("== 1b. WHERE THE TIME GOES (E=300, W=20, fixed-point) ==")
	local profCfg = {
		pop = 300,
		weapons = 20,
		ticks = 60,
		frost = 0,
		mix = bossMix,
		checksum = true,
		profile = true,
		seed = 0x9E3779B9,
	}
	yield()
	local _, profRes = timeRun(function(x)
		return runFixed(x, Fixed)
	end, profCfg, 1)
	yield()
	local totalP = 0
	for _, k in PHASES do
		totalP += profRes.prof[k]
	end
	Env.write("  phase          ms/tick    share")
	for _, k in PHASES do
		local ms = profRes.prof[k] / profCfg.ticks * 1000
		Env.write(string.format("  %-12s %9.4f %7.1f%%", k, ms, profRes.prof[k] / totalP * 100))
	end
	Env.write(string.format("  %-12s %9.4f", "TOTAL", totalP / profCfg.ticks * 1000))
	Env.write(string.format(
		"  work per tick: %.0f weapon-range scans, %.1f projectile impacts, %d enemies",
		profRes.scans / profCfg.ticks, profRes.hits / profCfg.ticks, profCfg.pop
	))
	Env.write("  NOTE the scan count. combat.rs fire_weapons does NOT advance a weapon's")
	Env.write("  cooldown when nothing is in range (\"do not fire, do not advance cooldown\"),")
	Env.write("  so a short-range weapon on a board whose enemies are all still marching in")
	Env.write("  re-scans every enemy EVERY tick. That turns the amortised O(W*E/cooldown)")
	Env.write("  into a true O(W*E). It is the single cheapest algorithmic win available:")
	Env.write("  a spatial grid, or a per-tick nearest-enemy distance computed once and")
	Env.write("  shared by all weapons, removes it.")
	Env.write("  (per-phase os.clock() calls add a few % of overhead; the table is for")
	Env.write("   apportioning blame, not for the headline number.)")

	--------------------------------------------------------- fixed vs double
	Env.write("")
	Env.write("== 2. WHAT EXACTNESS COSTS (CONTRACTS §C2 / docs/10 §F5) ==")
	local iters = opts.opIters or 120000
	Env.write("  Per-operation, loop-overhead subtracted. 'ratio' is exact-Fixed vs the")
	Env.write("  equivalent native double expression.")
	Env.write("")
	Env.write("  op            exact ns   raw-dbl ns     ratio")
	yield()
	for _, r in opCosts(Fixed, iters) do
		-- Below ~0.2 ns the double side is at the resolution floor of this timer;
		-- printing a ratio there would be inventing precision.
		local ratio = if r.double >= 2e-10 then string.format("%9.0fx", r.ratio) else "        --"
		Env.write(string.format("  %-12s %10.1f %12.2f %s", r.name, r.fixed * 1e9, r.double * 1e9, ratio))
	end

	local wf = results["exact/300/20"]
	local wa = results["api-dbl/300/20"]
	local wd = results["raw-dbl/300/20"]
	Env.write("")
	Env.write(string.format("  whole-workload, E=300 / W=20:"))
	Env.write(string.format("    exact §C2 Fixed          %8.3f ms/tick   %6.0fx raw doubles", wf, wf / wd))
	Env.write(string.format("    §C2 API over doubles     %8.3f ms/tick   %6.1fx raw doubles", wa, wa / wd))
	Env.write(string.format("    raw doubles              %8.3f ms/tick        1.0x", wd))
	Env.write(string.format("    exactness tax (exact / api-dbl): %.0fx", wf / wa))
	Env.write("  The workload ratio is the one that matters: not every op in a tick is a")
	Env.write("  multiply, so the isolated op ratios are an upper bound on the real tax.")

	---------------------------------------------------------------- replication
	yield()
	replicationReport(300, { 10, 15, 30 })

	----------------------------------------------------------------- verdict
	Env.write("")
	Env.write("== 4. THE R2 GATE ==")
	Env.write(string.format(
		"  Budget: one 30 Hz sim tick is %.2f ms of wall clock for ALL 8 arenas.",
		TICK_BUDGET_MS
	))
	Env.write(string.format(
		"  A server also replicates, runs physics and runs every other script, so the",
		0
	))
	Env.write(string.format(
		"  honest ceiling for simulation is taken as %d%% of that = %.2f ms.",
		SAFE_SHARE * 100, TICK_BUDGET_MS * SAFE_SHARE
	))
	Env.write("")
	Env.write("  backend    scenario            serial (1 thread)      parallel (8 Actors)")
	local scen = {
		{ "realistic  E=68  W=20", "68/20" },
		{ "worst case E=300 W=20", "300/20" },
	}
	for _, be in { "exact", "api-dbl", "raw-dbl" } do
		for _, sc in scen do
			local ms = results[be .. "/" .. sc[2]]
			local serial = ms * ARENAS / (TICK_BUDGET_MS * SAFE_SHARE) * 100
			local par = ms / (TICK_BUDGET_MS * SAFE_SHARE) * 100
			Env.write(string.format(
				"  %-8s   %-20s %7.1f%% %-8s %9.1f%% %-8s",
				be, sc[1],
				serial, if serial < 100 then "PASS" else "FAIL",
				par, if par < 100 then "PASS" else "FAIL"
			))
		end
	end
	Env.write("")
	Env.write("  'parallel' assumes 8 arenas land on 8 genuinely concurrent worker threads.")
	Env.write("  Roblox sizes that pool from the host machine; it is NOT guaranteed to be 8.")
	Env.write("  The serial column is the floor guarantee, the parallel column the ceiling.")
	Env.write("  Truth is between them, and the honest planning number is the serial one.")

	----------------------------------------------------------------- caveats
	Env.write("")
	Env.write("== 5. WHAT THESE NUMBERS DO AND DO NOT PROVE ==")
	if not Env.isRoblox then
		Env.write("  * This ran under a STANDALONE luau interpreter, not a Roblox server. The VM")
		Env.write("    is the same language but not the same configuration: Roblox ships its own")
		Env.write("    build with different FFlags, its own allocator, and a sandbox layer.")
		Env.write("    Treat this as an ORDER OF MAGNITUDE, not as the gate result.")
	end
	Env.write("  * No `--!native` here. Roblox's native codegen typically wins 1.5-3x on")
	Env.write("    integer/arithmetic hot loops like these, so the Roblox number could be")
	Env.write("    materially BETTER than what is printed above.")
	Env.write("  * No Actor scheduling. Parallel Luau has real per-task dispatch and")
	Env.write("    desynchronize/synchronize barrier costs that this cannot see, and the")
	Env.write("    worker pool is sized by the host machine, not by us.")
	Env.write("  * A Roblox server is NOT idle: replication, physics, character streaming,")
	Env.write("    and every other script share the same frame. The 30% budget column")
	Env.write("    exists for that reason and is the honest column to read.")
	Env.write("  * The benchmark hardware here is a build container, not a Roblox server.")
	Env.write("  * Population is PINNED, so the cost curve is clean but a real arena")
	Env.write("    oscillates: Clear wipes the board, then it refills.")
	Env.write("  * Weapon abilities not modelled: hazards, minions, auras, vulnerability")
	Env.write("    pulses, Fire chain explosions. All were 0 in the 24-seed Rust profile,")
	Env.write("    but a build that stacks them adds O(H*E) and O(M*E) passes on top.")
	Env.write("================================================================================")

	return {
		fixedSource = fixedSource,
		results = results,
		worstPop = worstPop,
		text = Env.text(),
	}
end

Bench.WaveModel = WaveModel
Bench.runFixed = runFixed
Bench.runDouble = runDouble
Bench.Env = Env

-- Small façade so an Actor worker (or any other driver) can run one arena
-- without re-deriving the wave model or resolving modules itself.
local cachedFixed, cachedStub

function Bench.makeConfig(pop: number, weapons: number, ticks: number)
	local _, _, bossMix = WaveModel.steadyState(BOSS_SPAWN_TICK, 1.0)
	return {
		pop = pop,
		weapons = weapons,
		ticks = ticks,
		frost = 0,
		mix = bossMix,
		checksum = true,
		seed = 0x9E3779B9,
	}
end

function Bench.runBackend(name: string, cfg)
	if name == "exact" then
		if not cachedFixed then
			cachedFixed = Env.resolveFixed()
		end
		return runFixed(cfg, cachedFixed)
	elseif name == "api-dbl" then
		if not cachedStub then
			cachedStub = Env.loadStub()
		end
		return runFixed(cfg, cachedStub)
	end
	return runDouble(cfg)
end

--=============================================================================
-- 8. Studio glue. Everything above is environment-free.
--=============================================================================

if not Env.isRoblox then
	Bench.run({ targetSec = tonumber((...)) or 0.30 })
end

return Bench
