--[[
	timeline_test.luau — the F1 Roblox timeline as a RUNTIME option
	(`PHASE2-CONTRACTS.md` P3, `docs/10` F1).

	WHAT THIS GUARDS
	----------------
	Two timelines ship as data in `content.json`. Which one an arena runs is a
	per-state choice (`state.timeline`; `nil` ⇒ Steam). Three things have to hold
	at once, and each has a failure mode that is silent rather than loud:

	  1. STEAM IS UNTOUCHED. The R3 oracle (`trace_test.luau`) is the only proof
	     the sim is correct, and it is recorded from the Rust, which is hardcoded
	     to Steam. That gate is the real acceptance test for this file; what is
	     asserted here is the *defaulting* that keeps it true — a state with no
	     timeline must resolve to `Content.TIMELINE.steam` (900-tick rounds, boss
	     at 54000) with the wave gates at their unrescaled content values.

	  2. THE ROBLOX TIMELINE ACTUALLY RUNS. Boss at 9000, a new shop every 600
	     ticks. Failure mode: nothing errors, the run is just 30 minutes long.

	  3. THE WAVE GATES RESCALE. The `start_tick` gates in `content.json` ship at
	     STEAM scale (up to 45000) so one catalog serves both timelines, and
	     `Waves.gateTick` divides them by `tick_scale_num/den`. Without it, 7 of
	     the 17 roster entries never open before a 9000-tick boss and the roster
	     silently collapses to its opening mix — no error, anywhere. §4 asserts
	     that 7 both as arithmetic and as an observed spawn deficit, by running
	     the real `Waves.spawn` with the rescale disabled.

	  4. TIMELINES DO NOT LEAK BETWEEN ARENAS. One server simulates 8 arenas
	     (`docs/10` F6); a Roblox-timeline arena must not perturb a Steam one
	     running beside it. §7 interleaves two states and checks the Steam one
	     against a solo control, checksum for checksum.

	HOW TO RUN
	----------
	    luau roblox/test/timeline_test.luau

	No argv, no file I/O — `Content.luau` self-loads its embedded content.
	Exits non-zero on failure.
]]

local Content = require("../src/shared/Content")
local Arena = require("../src/shared/sim/Arena")
local Input = require("../src/shared/sim/Input")
local Waves = require("../src/shared/sim/Waves")
local Fixed = require("../src/shared/Fixed")

local passed, failed = 0, 0
local firstFailures = {}

local function check(what, ok)
	if ok then
		passed += 1
	else
		failed += 1
		if #firstFailures < 20 then
			firstFailures[#firstFailures + 1] = what
		end
	end
end

local function eq(what, got, want)
	if got == want then
		passed += 1
	else
		failed += 1
		if #firstFailures < 20 then
			firstFailures[#firstFailures + 1] =
				string.format("%s: got %s, want %s", what, tostring(got), tostring(want))
		end
	end
end

local STEAM = Content.TIMELINE.steam
local ROBLOX = Content.TIMELINE.roblox

--- The wave table in content order, 1-based, whatever base `Content` uses.
local WAVES = {}
do
	local t = Content.WAVE_M0
	local base = if t[0] ~= nil then 0 else 1
	local n = if base == 0 then #t + 1 else #t
	for i = 1, n do
		WAVES[i] = t[i - 1 + base]
	end
end

-- ==================== 1. the timeline data itself ====================
-- Cheap, but it pins the two numbers every later section is written against; if
-- content moves, this fails first and names the reason instead of the symptom.

eq("steam.round_ticks", STEAM.round_ticks, 900)
eq("steam.boss_spawn_tick", STEAM.boss_spawn_tick, 54000)
eq("roblox.round_ticks", ROBLOX.round_ticks, 600)
eq("roblox.boss_spawn_tick", ROBLOX.boss_spawn_tick, 9000)
eq("wave table is the 17-entry roster", #WAVES, 17)
-- F1's load-bearing invariant: the ramp interval is NOT the round length, so the
-- ramp compounds the same number of times on both timelines and the rescale
-- stays a rescale rather than a rebalance.
eq("ramp compounds equally on both timelines", ROBLOX.ramp_intervals_to_boss, STEAM.ramp_intervals_to_boss)
check("roblox ramp_interval ~= round_ticks (F1)", ROBLOX.ramp_interval ~= ROBLOX.round_ticks)

-- ==================== 2. Steam is the default ====================
-- The whole point of P3: absent an explicit choice, everything resolves to the
-- schedule the Rust runs, because that is what the R3 traces are recorded from.

do
	local s = Arena.newState(1, 0)
	eq("fresh state carries no timeline", s.timeline, nil)
	eq("default timeline is steam", Arena.timelineFor(s), STEAM)
	eq("default round_ticks", Arena.roundTicks(s), 900)
	eq("default boss tick", Arena.bossTick(s), 54000)
	eq("nil state resolves to steam", Waves.timelineFor(nil), STEAM)
	-- The constant this task removed. A stale `s.tick // Arena.ROUND_TICKS`
	-- reader must break loudly on nil, not silently keep 900 forever.
	eq("Arena.ROUND_TICKS is gone (it is a lookup now)", Arena.ROUND_TICKS, nil)
	-- Explicit "steam" and the default must be the same object, not a copy.
	eq("explicit steam == default", Arena.timelineFor(Arena.newState(1, 0, "steam")), STEAM)
end

-- On Steam the gate rescale is the identity — this is why the oracle is
-- unaffected. `tick_scale_num == tick_scale_den == 1`.
do
	local identical = true
	for i, ws in WAVES do
		if Waves.gateTick(STEAM, ws.start_tick) ~= ws.start_tick then
			identical = false
			firstFailures[#firstFailures + 1] = "steam gate rescaled at entry " .. i
		end
	end
	check("steam gate rescale is the identity on all 17 entries", identical)
end

-- ==================== 3. selecting the Roblox timeline ====================

do
	local byName = Arena.newState(2, 0, "roblox")
	eq("newState(.., 'roblox') selects it", Arena.timelineFor(byName), ROBLOX)
	eq("roblox round_ticks", Arena.roundTicks(byName), 600)
	eq("roblox boss tick", Arena.bossTick(byName), 9000)

	local bySetter = Arena.setTimeline(Arena.newState(2, 0), "roblox")
	eq("Arena.setTimeline by name", Arena.timelineFor(bySetter), ROBLOX)
	eq("Arena.setTimeline returns the state", Arena.setTimeline(Arena.newState(2, 0), ROBLOX).timeline, ROBLOX)
	eq("Arena.setTimeline(nil) clears back to steam",
		Arena.timelineFor(Arena.setTimeline(bySetter, nil)), STEAM)

	-- Loud on nonsense: a table that is not a timeline would otherwise make the
	-- boss simply never spawn, with no error to trace it back from.
	check("unknown timeline name errors", not (pcall(Arena.setTimeline, Arena.newState(2, 0), "wc3")))
	check("a non-timeline table errors", not (pcall(Arena.setTimeline, Arena.newState(2, 0), { round_ticks = 600 })))
	check("Waves.resolveTimeline(nil) stays nil", Waves.resolveTimeline(nil) == nil)
	eq("Waves.resolveTimeline('roblox')", Waves.resolveTimeline("roblox"), ROBLOX)

	-- Fixed at setup. Round boundaries and gates are re-derived every tick, so a
	-- mid-run switch would make a replay of the same inputs diverge.
	local mid = Arena.newState(2, 0)
	Arena.step(mid, Input.NOOP)
	check("switching timeline mid-run errors", not (pcall(Arena.setTimeline, mid, "roblox")))
	check("re-setting the SAME timeline mid-run is allowed", (pcall(Arena.setTimeline, mid, nil)))
end

-- ==================== 4. the gate rescale, and the 7 it saves ====================
-- The bug this guards against, stated twice: once as arithmetic over the gates,
-- once as an observed spawn deficit out of the real `Waves.spawn`.

--- A copy of the Roblox timeline with the rescale switched off — i.e. what F1
--- would be if `tick_scale_num/den` were never consulted. Everything else about
--- the timeline is identical, so any difference below is the rescale alone.
local NO_RESCALE = {}
for k, v in ROBLOX do
	NO_RESCALE[k] = v
end
NO_RESCALE.tick_scale_num = 1
NO_RESCALE.tick_scale_den = 1

do
	local shutWithout, openWith, exact = 0, 0, true
	for i, ws in WAVES do
		local g = Waves.gateTick(ROBLOX, ws.start_tick)
		-- Every shipped gate divides evenly by 6, so the rescale is exact rather
		-- than merely floored — no gate drifts off its intended tick.
		if g * ROBLOX.tick_scale_den ~= ws.start_tick * ROBLOX.tick_scale_num then
			exact = false
			firstFailures[#firstFailures + 1] = "gate " .. i .. " rescale is not exact"
		end
		if g < ROBLOX.boss_spawn_tick then
			openWith += 1
		end
		if Waves.gateTick(NO_RESCALE, ws.start_tick) >= ROBLOX.boss_spawn_tick then
			shutWithout += 1
		end
	end
	check("every roblox gate rescale is exact (no flooring loss)", exact)
	eq("all 17 gates open before the roblox boss", openWith, 17)
	eq("without the rescale, 7 gates would never open", shutWithout, 7)
end

-- ==================== 5. observed spawns: the gates really fire ====================

--- Sweep `Waves.spawn` tick by tick over `[0, until)` on `tl`, clearing the
--- enemy list each tick (exactly what the Rust oracle binary does), and return
--- `{ [enemy def] = spawn count }` plus the total.
local function sweep(tl, untilTick)
	local s = Arena.newState(0xBEEF, 0, tl)
	local counts, total = {}, 0
	for tick = 0, untilTick - 1 do
		s.tick = tick
		table.clear(s.enemies)
		Waves.spawn(s)
		for _, e in s.enemies do
			counts[e.def] = (counts[e.def] or 0) + 1
			total += 1
		end
	end
	return counts, total
end

--- What the roster *should* produce over `[0, until)` on `tl`, derived from the
--- gates and cadences independently of `Waves.spawn`.
local function expected(tl, untilTick)
	local counts, total = {}, 0
	for _, ws in WAVES do
		if ws.cadence_ticks ~= 0 then
			local g = Waves.gateTick(tl, ws.start_tick)
			for tick = 0, untilTick - 1 do
				if tick >= g and tick % ws.cadence_ticks == 0 then
					counts[ws.enemy] = (counts[ws.enemy] or 0) + 1
					total += 1
				end
			end
		end
	end
	return counts, total
end

do
	local BOSS_T = ROBLOX.boss_spawn_tick
	local got, gotTotal = sweep(ROBLOX, BOSS_T)
	local want, wantTotal = expected(ROBLOX, BOSS_T)
	eq("roblox pre-boss spawn total matches the gate schedule", gotTotal, wantTotal)
	local perDef = true
	for def, n in want do
		if got[def] ~= n then
			perDef = false
			firstFailures[#firstFailures + 1] =
				string.format("roblox spawns for def %d: got %s, want %d", def, tostring(got[def]), n)
		end
	end
	check("roblox per-def spawn counts match the gate schedule", perDef)

	-- Every enemy def the 17-entry roster can produce must actually appear
	-- before the boss. This is the "all gates open" claim as behavior, not
	-- arithmetic: a shut gate for a def no other entry produces shows up here.
	local rosterDefs = {}
	for _, ws in WAVES do
		rosterDefs[ws.enemy] = true
	end
	local missing = 0
	for def in rosterDefs do
		if (got[def] or 0) == 0 then
			missing += 1
			firstFailures[#firstFailures + 1] = "roblox: enemy def " .. def .. " never spawns before the boss"
		end
	end
	eq("no roster enemy is missing from a roblox run", missing, 0)

	-- The control. Same timeline, rescale off: strictly fewer enemies, and the
	-- late-roster defs go missing.
	local without, withoutTotal = sweep(NO_RESCALE, BOSS_T)
	check("without the rescale the roster is strictly poorer", withoutTotal < gotTotal)
	local lost = 0
	for def in rosterDefs do
		if (without[def] or 0) == 0 and (got[def] or 0) > 0 then
			lost += 1
		end
	end
	check("without the rescale at least one enemy type never appears", lost > 0)
end

-- ==================== 6. the boss, and the shop cadence ====================
-- End to end through `Arena.step`, not through `Waves.spawn` in isolation: this
-- is the one place round-boundary detection, wave gating and HP scaling all run
-- together on the Roblox timeline.

local function runToBoss(tl, untilTick)
	local s = Arena.newState(11, 0, tl)
	-- The tank is kept alive on purpose. This test is about the SCHEDULE, and a
	-- no-input run dies long before the boss on either timeline; survival is
	-- balance's problem, not the timeline's.
	s.tank.max_hp = 1000000000
	s.tank.hp = s.tank.max_hp
	local bossTick, roundStarts, seq = nil, {}, s.shop.shop_seq
	for tick = 0, untilTick do
		Arena.step(s, Input.NOOP)
		s.tank.hp = s.tank.max_hp
		if s.shop.shop_seq ~= seq then
			seq = s.shop.shop_seq
			roundStarts[#roundStarts + 1] = tick
		end
		if bossTick == nil then
			for _, e in s.enemies do
				if e.def == Content.BOSS then
					bossTick = tick
					break
				end
			end
		end
		-- Keep the sweep O(1) per tick: the schedule is what is under test, and
		-- an unbounded 9000-tick population makes this test cost more than the
		-- whole rest of the suite.
		table.clear(s.enemies)
		table.clear(s.projectiles)
		table.clear(s.pending_kills)
	end
	return s, bossTick, roundStarts
end

do
	local s, bossTick, roundStarts = runToBoss("roblox", ROBLOX.boss_spawn_tick + 60)
	eq("roblox: the boss arrives at tick 9000", bossTick, 9000)
	eq("roblox: round counter at the boss", s.round, 15)

	-- A new shop every 600 ticks, on the boundary and nowhere else.
	local cadenceOk = #roundStarts > 0
	for i, t in roundStarts do
		if t ~= (i - 1) * ROBLOX.round_ticks then
			cadenceOk = false
		end
	end
	check("roblox: a new shop opens exactly every 600 ticks", cadenceOk)
	eq("roblox: 16 shops through the boss tick (15 rounds + round 0)", #roundStarts, 16)

	-- Same drive on the default timeline: 900-tick rounds, no boss in sight.
	local steamS, steamBoss, steamStarts = runToBoss(nil, 3600)
	eq("steam: no boss anywhere near tick 9000", steamBoss, nil)
	eq("steam: round counter at 3600", steamS.round, 4)
	eq("steam: 5 shops in 3600 ticks", #steamStarts, 5)
	local steamCadence = true
	for i, t in steamStarts do
		if t ~= (i - 1) * 900 then
			steamCadence = false
		end
	end
	check("steam: a new shop opens exactly every 900 ticks", steamCadence)
end

-- ==================== 7. HP scaling follows the timeline ====================

do
	-- F1's "rescale, not rebalance": the compressed run must land on the SAME
	-- difficulty endpoint as the 30-minute one.
	local function milli(x)
		return Fixed.toInt(Fixed.scaleI64(x, 1000000))
	end
	eq("roblox endpoint == steam endpoint",
		milli(Waves.enemyHpMult(ROBLOX.boss_spawn_tick, ROBLOX)),
		milli(Waves.enemyHpMult(STEAM.boss_spawn_tick, STEAM)))
	-- Halfway through each run must also agree — the whole curve is rescaled,
	-- not just its endpoints.
	eq("roblox midpoint == steam midpoint",
		milli(Waves.enemyHpMult(ROBLOX.boss_spawn_tick // 2, ROBLOX)),
		milli(Waves.enemyHpMult(STEAM.boss_spawn_tick // 2, STEAM)))
	-- And the default is still steam: at tick 9000 an unqualified call must give
	-- the early-game Steam value, not the Roblox endpoint.
	eq("enemyHpMult default is steam", milli(Waves.enemyHpMult(9000)), milli(Waves.enemyHpMult(9000, STEAM)))
	check("the two timelines genuinely differ at 9000",
		milli(Waves.enemyHpMult(9000, ROBLOX)) ~= milli(Waves.enemyHpMult(9000, STEAM)))
end

-- ==================== 8. no leakage between arenas ====================
-- `docs/10` F6: one server steps 8 arenas. A Roblox-timeline shard must not
-- perturb a Steam shard beside it — including through `Waves`' module-level
-- content/timeline state, which is shared by every arena in the process.

do
	local TICKS = 1200 -- past two Steam boundaries and three Roblox ones

	local control = Arena.newState(0x5EED, 3)
	local controlSums = {}
	for i = 1, TICKS do
		Arena.step(control, Input.NOOP)
		controlSums[i] = Arena.checksum(control)
	end

	local steam = Arena.newState(0x5EED, 3)
	local roblox = Arena.newState(0x5EED, 3, "roblox")
	local same, diverged = true, false
	for i = 1, TICKS do
		Arena.step(roblox, Input.NOOP) -- interleaved, and stepped FIRST
		Arena.step(steam, Input.NOOP)
		if Arena.checksum(steam) ~= controlSums[i] then
			same = false
		end
		if Arena.checksum(roblox) ~= controlSums[i] then
			diverged = true
		end
	end
	check("a roblox arena beside a steam one leaves it byte-identical", same)
	check("the roblox arena does diverge (the test is not vacuous)", diverged)

	-- Same timeline, same seed, twice ⇒ identical. Determinism is not weakened
	-- by making the timeline runtime state.
	local a = Arena.newState(0xC0FFEE, 1, "roblox")
	local b = Arena.newState(0xC0FFEE, 1, "roblox")
	local reproducible = true
	for _ = 1, 700 do
		Arena.step(a, Input.NOOP)
		Arena.step(b, Input.NOOP)
		if Arena.checksum(a) ~= Arena.checksum(b) then
			reproducible = false
		end
	end
	check("the roblox timeline is reproducible seed-for-seed", reproducible)
end

-- ==================== 9. the module-wide override ====================
-- `Waves.timeline` is a test hook, not the arena mechanism (P3): it applies to
-- every arena without one of its own. Verified, then put back — leaving it set
-- would silently move every other test in the process onto the Roblox schedule.

do
	Waves.setTimeline("roblox")
	local s = Arena.newState(3, 0)
	eq("module override applies to a state with no timeline", Arena.roundTicks(s), 600)
	eq("state.timeline still wins over the module override",
		Arena.roundTicks(Arena.newState(3, 0, "steam")), 900)
	Waves.setTimeline(nil)
	eq("the override is restored to steam", Arena.roundTicks(s), 900)
	eq("Waves.timeline back to nil", Waves.timeline, nil)
end

-- ==================== report ====================

print(string.format("timeline_test: %d passed, %d failed", passed, failed))
if failed > 0 then
	for _, f in firstFailures do
		print("  FAIL " .. f)
	end
	error("timeline_test: FAILED", 0)
end
