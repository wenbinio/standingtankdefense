--[[
	realsim.luau — R2 THROUGHPUT, MEASURED ON THE REAL SIM (docs/10 §F7 R2, §F8)

	WHY THIS FILE EXISTS
	  `throughput.luau` is a *model*: a struct-of-arrays mini-arena that mirrors
	  `sim::step`'s phase order and pins the population to a constant so the
	  cost/entity curve is clean. It was written before R3, when there was no
	  Luau sim to measure. It answers "what does exact fixed-point cost per
	  entity", and it still answers that well.

	  It cannot answer "does R2 clear", because it does not run the shipped code
	  and it cannot see the population the shipped code actually reaches. The
	  `[11]` balance pass proved that gap matters: `WAVE_M0`'s cadences did not
	  change at all, so the analytic Little's-law model still says ~67 enemies —
	  but the balance pass ramped enemy HP (`content::enemy_hp_mult`), enemies
	  now live far longer than the kill-free model assumes, and the real board
	  peaks near 4x that.

	  This file runs `Arena.step` — the real sim, the real `Fixed`, the real
	  `Combat`, driven by the real `Bot` — for whole matches, and reports the
	  population it actually reaches and the wall clock it actually costs.

	WHAT IT MEASURES
	  1. Population: enemies / projectiles / weapons per tick, over whole matches
	     on several seeds — mean, p50/p90/p99, peak. This is the workload.
	  2. Cost: ms per `Arena.step`, mean over the match and at the peak, then
	     extrapolated to 8 arenas at 30 Hz as a % of the 33.33 ms tick budget and
	     of the 30% (10.00 ms) share `[10] §F8` takes as the honest ceiling.
	  3. Cost bucketed by live-enemy count, which is what shows whether the sim
	     is linear in E.

	  Both shipped timelines are measured: `steam` (boss at 54000 — the timeline
	  the R3 oracle traces are recorded against, and the one `[10] §F8`'s
	  "peak 218 / mean 53" figure came from) and `roblox` (boss at 9000 — F1, the
	  timeline this fork actually ships).

	HONESTY
	  Same caveats as `throughput.luau`, and they are printed in full at the end
	  of the report: this is a standalone interpreter on container hardware, not
	  a Roblox server; no `--!native`; no Actor scheduling; a real server frame
	  also carries replication and physics. Serial is the honest planning column.

	RUN
	    cd roblox/bench
	    luau -O2 realsim.luau               # default: 4 seeds, both timelines
	    luau -O2 realsim.luau -a 8          # 8 seeds
	    luau -O2 --codegen realsim.luau     # closest proxy for --!native
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
	Env.clock = os.clock

	local lines: { string } = {}
	function Env.write(s: string)
		table.insert(lines, s)
		print(s)
	end
	function Env.text(): string
		return table.concat(lines, "\n")
	end

	--- Resolve a `dist/shared/sim` module in either host. Standalone reads
	--- `src/` directly (the sources the test suite and the R3 oracle run);
	--- Roblox reads the Rojo-synced `dist/` copies.
	function Env.sim(name: string)
		if isRoblox then
			local rs = game:GetService("ReplicatedStorage")
			return require(rs.Shared.sim[name])
		end
		return require("../src/shared/sim/" .. name)
	end
end

local Arena = Env.sim("Arena")
local Bot = Env.sim("Bot")
local Input = Env.sim("Input")

--=============================================================================
-- 1. Budget constants — identical to throughput.luau, deliberately.
--=============================================================================

local TICK_HZ = 30
local TICK_BUDGET_MS = 1000 / TICK_HZ -- 33.33 ms for ALL arenas, one 30 Hz tick
local SAFE_SHARE = 0.30 -- what a server can spend on sim and still do everything else
local ARENAS = 8
local SAFE_MS = TICK_BUDGET_MS * SAFE_SHARE

-- Whole-match lengths. A run ends when the tank dies or the boss falls; the
-- sim keeps ticking after death (frozen) purely to keep traces length-aligned,
-- so a run is cut at `deathGrace` ticks past death rather than padded with
-- free ticks that would flatter the mean.
local LENGTHS = {
	steam = 56000, -- boss at 54000 + the fight
	roblox = 11000, -- boss at 9000 + the fight (F1)
}

--=============================================================================
-- 2. One measured match.
--=============================================================================

export type Sample = {
	ticks: number,
	tickMs: { number }, -- ms per Arena.step
	botMs: number, -- total ms spent in Bot.decide (NOT in tickMs)
	enemies: { number }, -- live enemies AFTER that step
	projectiles: { number },
	weapons: { number },
	diedAt: number?,
	reachedBoss: boolean,
}

--- Run one whole match, timing every `Arena.step` and recording the board.
---
--- The two `os.clock()` calls per tick cost ~0.1 us against ticks that cost
--- hundreds of us at the populations that matter, so per-tick timing is used
--- rather than block timing: the peak tick is the number the frame budget is
--- actually about, and a block mean hides it.
local function runMatch(seed: number, timeline: string?, maxTicks: number): Sample
	local s = Arena.newState(seed, 0, timeline)
	local bot = Bot.new()
	local clock = Env.clock

	local tickMs, enemies, projectiles, weapons = {}, {}, {}, {}
	local diedAt: number? = nil
	local n = 0
	-- `Bot.decide` is timed separately and deliberately EXCLUDED from tickMs: a
	-- human's input arrives over the network, so only a backfill-bot seat pays
	-- it. Reported as its own row so the bot-seat case is not hidden.
	local botMs = 0

	for tick = 0, maxTicks - 1 do
		local b0 = clock()
		local inp = Bot.decide(bot, s) or Input.NOOP
		botMs += (clock() - b0) * 1000
		local t0 = clock()
		Arena.step(s, inp)
		local t1 = clock()
		n += 1
		tickMs[n] = (t1 - t0) * 1000
		enemies[n] = #s.enemies
		projectiles[n] = #s.projectiles
		weapons[n] = #s.weapons
		if s.dead and diedAt == nil then
			diedAt = tick
			break
		end
	end

	return {
		ticks = n,
		tickMs = tickMs,
		botMs = botMs,
		enemies = enemies,
		projectiles = projectiles,
		weapons = weapons,
		diedAt = diedAt,
		reachedBoss = s.tick >= Arena.bossTick(s),
	}
end

--=============================================================================
-- 3. Statistics. Nothing clever: sort and index.
--=============================================================================

local function quantile(sorted: { number }, q: number): number
	local n = #sorted
	if n == 0 then
		return 0
	end
	local i = math.max(1, math.min(n, math.ceil(q * n)))
	return sorted[i]
end

local function stats(values: { number })
	local copy = table.clone(values)
	table.sort(copy)
	local sum = 0
	for _, v in copy do
		sum += v
	end
	local n = #copy
	return {
		n = n,
		mean = if n > 0 then sum / n else 0,
		p50 = quantile(copy, 0.50),
		p90 = quantile(copy, 0.90),
		p99 = quantile(copy, 0.99),
		max = if n > 0 then copy[n] else 0,
	}
end

--- Worst sustained second: the largest mean over any 30 consecutive ticks.
--- A single expensive tick can be absorbed; 30 in a row cannot.
local function worstWindow(values: { number }, w: number): number
	local n = #values
	if n < w then
		return 0
	end
	local sum = 0
	for i = 1, w do
		sum += values[i]
	end
	local best = sum
	for i = w + 1, n do
		sum += values[i] - values[i - w]
		if sum > best then
			best = sum
		end
	end
	return best / w
end

--=============================================================================
-- 4. The report.
--=============================================================================

local Bench = {}

local BUCKETS = { 0, 25, 50, 75, 100, 150, 200, 250, 300, 1e9 }

function Bench.run(opts)
	opts = opts or {}
	local seedCount = opts.seeds or 4
	local timelines = opts.timelines or { "steam", "roblox" }
	local yield = opts.yield or function() end

	Env.write("================================================================================")
	Env.write("  STANDING TANK DEFENSE (Roblox fork) — R2 THROUGHPUT, REAL SIM")
	Env.write("  Arena.step + Bot.decide, whole matches. docs/10 §F7 R2 / §F8")
	Env.write("================================================================================")
	Env.write(string.format("  environment    : %s", if Env.isRoblox then "Roblox (Studio/server)" else "standalone luau"))
	Env.write(string.format("  sim rate       : %d Hz  ->  %.2f ms per tick, all arenas", TICK_HZ, TICK_BUDGET_MS))
	Env.write(string.format("  arenas         : %d   ·  30%% share = %.2f ms", ARENAS, SAFE_MS))
	Env.write(string.format("  seeds          : %d per timeline", seedCount))

	local out = {}

	for _, tl in timelines do
		local maxTicks = LENGTHS[tl] or 56000
		local allMs, allE, allP = {}, {}, {}
		local perSeed = {}
		local botTotalMs, botTotalTicks = 0, 0
		local bucketSum, bucketN = {}, {}
		for i = 1, #BUCKETS do
			bucketSum[i], bucketN[i] = 0, 0
		end

		Env.write("")
		Env.write(string.format("== TIMELINE: %s (boss tick %d, run cut at %d) ==", tl, if tl == "roblox" then 9000 else 54000, maxTicks))
		Env.write("  seed   ticks  outcome        E mean   E p99   E max   P max   W max   ms/tick  ms max")

		for k = 0, seedCount - 1 do
			local seed = k
			local sample = runMatch(seed, tl, maxTicks)
			local se = stats(sample.enemies)
			local sp = stats(sample.projectiles)
			local sw = stats(sample.weapons)
			local sm = stats(sample.tickMs)
			local outcome = if sample.diedAt ~= nil
				then string.format("died@%d", sample.diedAt)
				elseif sample.reachedBoss then "survived"
				else "cut"
			Env.write(string.format(
				"  %4d %7d  %-13s %7.1f %7d %7d %7d %7d %9.4f %7.3f",
				seed, sample.ticks, outcome, se.mean, se.p99, se.max, sp.max, sw.max, sm.mean, sm.max
			))
			table.insert(perSeed, { seed = seed, e = se, p = sp, w = sw, ms = sm, sample = sample })
			botTotalMs += sample.botMs
			botTotalTicks += sample.ticks
			for i = 1, sample.ticks do
				table.insert(allMs, sample.tickMs[i])
				table.insert(allE, sample.enemies[i])
				table.insert(allP, sample.projectiles[i])
				-- bucket by live enemies
				local e = sample.enemies[i]
				for b = 1, #BUCKETS - 1 do
					if e >= BUCKETS[b] and e < BUCKETS[b + 1] then
						bucketSum[b] += sample.tickMs[i]
						bucketN[b] += 1
						break
					end
				end
			end
			yield()
		end

		local E = stats(allE)
		local P = stats(allP)
		local M = stats(allMs)
		-- Worst sustained second across the seeds, taken per seed then maximised:
		-- concatenating seeds would invent a window that straddles two matches.
		local worstSec = 0
		for _, r in perSeed do
			worstSec = math.max(worstSec, worstWindow(r.sample.tickMs, TICK_HZ))
		end

		Env.write("")
		Env.write(string.format("  POPULATION (all seeds, all ticks, n=%d)", E.n))
		Env.write(string.format("    enemies      mean %.1f   p50 %d   p90 %d   p99 %d   peak %d", E.mean, E.p50, E.p90, E.p99, E.max))
		Env.write(string.format("    projectiles  mean %.1f   p50 %d   p90 %d   p99 %d   peak %d", P.mean, P.p50, P.p90, P.p99, P.max))

		Env.write("")
		Env.write("  COST OF ONE ARENA, ms per Arena.step")
		Env.write("    statistic          1 arena   x8 serial   % of 33.33ms   % of 30% budget")
		local rows = {
			{ "mean (whole match)", M.mean },
			{ "p90 tick", M.p90 },
			{ "p99 tick", M.p99 },
			{ "worst 1 s (30 ticks)", worstSec },
			{ "worst single tick", M.max },
		}
		for _, r in rows do
			local ms8 = r[2] * ARENAS
			Env.write(string.format(
				"    %-18s %9.4f %11.4f %13.1f%% %16.1f%%",
				r[1], r[2], ms8, ms8 / TICK_BUDGET_MS * 100, ms8 / SAFE_MS * 100
			))
		end

		Env.write(string.format(
			"    (Bot.decide, excluded from the rows above: %.4f ms/tick — only a",
			if botTotalTicks > 0 then botTotalMs / botTotalTicks else 0
		))
		Env.write("     backfill-bot seat pays it; a human's input arrives over the network.)")

		Env.write("")
		Env.write("  COST BY LIVE-ENEMY COUNT (is it linear in E?)")
		Env.write("    enemies      ticks    ms/tick   us per enemy")
		for b = 1, #BUCKETS - 1 do
			if bucketN[b] > 0 then
				local mean = bucketSum[b] / bucketN[b]
				local mid = (BUCKETS[b] + math.min(BUCKETS[b + 1], BUCKETS[b] + 50)) / 2
				local label = if BUCKETS[b + 1] >= 1e9
					then string.format("%d+", BUCKETS[b])
					else string.format("%d..%d", BUCKETS[b], BUCKETS[b + 1] - 1)
				Env.write(string.format(
					"    %-10s %8d %10.4f %14.2f",
					label, bucketN[b], mean, if mid > 0 then mean * 1000 / mid else 0
				))
			end
		end

		out[tl] = { E = E, P = P, M = M, worstSec = worstSec, perSeed = perSeed }
		yield()
	end

	--------------------------------------------------------------------- gate
	Env.write("")
	Env.write("== THE R2 GATE, RE-MEASURED ==")
	Env.write(string.format(
		"  Budget: one 30 Hz tick is %.2f ms for ALL %d arenas; the honest simulation",
		TICK_BUDGET_MS, ARENAS
	))
	Env.write(string.format("  ceiling is %d%% of that = %.2f ms, because the server frame is not ours alone.", SAFE_SHARE * 100, SAFE_MS))
	Env.write("")
	Env.write("  timeline  statistic              serial (8 on 1 thread)   parallel (8 Actors)")
	for _, tl in timelines do
		local r = out[tl]
		if r then
			for _, row in { { "mean tick", r.M.mean }, { "worst 1 s", r.worstSec }, { "worst tick", r.M.max } } do
				local serial = row[2] * ARENAS / SAFE_MS * 100
				local par = row[2] / SAFE_MS * 100
				Env.write(string.format(
					"  %-9s %-20s %9.1f%% %-8s %11.1f%% %-8s",
					tl, row[1],
					serial, if serial < 100 then "PASS" else "FAIL",
					par, if par < 100 then "PASS" else "FAIL"
				))
			end
		end
	end
	Env.write("")
	Env.write("  The serial column is the floor guarantee and THE HONEST PLANNING NUMBER:")
	Env.write("  Roblox sizes the parallel-Luau worker pool from the host machine and does")
	Env.write("  not guarantee 8 threads. 'parallel' is a ceiling, not a promise.")

	------------------------------------------------------------------ caveats
	Env.write("")
	Env.write("== WHAT THESE NUMBERS DO AND DO NOT PROVE ==")
	if not Env.isRoblox then
		Env.write("  * This ran under a STANDALONE luau interpreter, not a Roblox server. The VM")
		Env.write("    is the same language but not the same configuration: Roblox ships its own")
		Env.write("    build with different FFlags, its own allocator, and a sandbox layer.")
		Env.write("    Treat this as an ORDER OF MAGNITUDE, not as the gate result.")
	end
	Env.write("  * No `--!native`. Roblox's native codegen typically wins 1.5-3x on hot")
	Env.write("    integer loops like these, so the Roblox figure could be materially BETTER.")
	Env.write("  * No Actor scheduling: per-task dispatch and desynchronize/synchronize")
	Env.write("    barrier costs are unmodelled, and the worker pool is sized by the host.")
	Env.write("  * A Roblox server is NOT idle: replication, physics, character streaming and")
	Env.write("    every other script share the frame. Hence the 30% column.")
	Env.write("  * The hardware is a build container, not a Roblox server.")
	Env.write("  * `View.snapshot` / `Wire.packDelta` are NOT in these ticks — this is the")
	Env.write("    simulation cost only. `wire_test` measures replication separately and")
	Env.write("    `[10] §F8` records it as not being a constraint.")
	Env.write("  * The reference `Bot` is not a human. It fires `Clear` off cooldown, which")
	Env.write("    wipes the board and therefore CAPS the population a human might exceed by")
	Env.write("    holding it. It also buys a broad arsenal, which is the expensive case for")
	Env.write("    `fire_weapons`.")
	Env.write("  * Timing is per-tick `os.clock()`. Two calls cost ~0.1 us against ticks of")
	Env.write("    hundreds of us at the populations that matter, but the sub-microsecond")
	Env.write("    rows (an empty opening board) carry that overhead visibly.")
	Env.write("================================================================================")

	return { timelines = out, text = Env.text() }
end

Bench.runMatch = runMatch
Bench.stats = stats
Bench.Env = Env

if not Env.isRoblox then
	Bench.run({ seeds = tonumber((...)) or 4 })
end

return Bench
