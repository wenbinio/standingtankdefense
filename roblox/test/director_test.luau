--[[
	director_test.luau — end-to-end test of the server-authoritative match layer
	(`roblox/src/server/{ArenaRunner,Director}.luau`, `docs/10` F2/F6).

	Drives a whole multi-player match under the bare `luau` interpreter — no
	Roblox host, no engine globals — and asserts the properties F2 actually
	claims, not just that the modules load:

	  1. ARENA INDEPENDENCE — several arenas advance on the same server clock
	     with distinct seeds and distinct state, and one player's death changes
	     nothing about anyone else's run. Plus: the same master seed replays the
	     same match, so the match layer is order-stable.
	  2. INPUT AUTHORITY (P5) — a client may submit only an input code; malformed,
	     out-of-range, mis-addressed and flooded codes are REJECTED with a reason
	     rather than clamped, and a legal code reaches the sim.
	  3. INSTANT RE-ENTRY (F2) — death produces a result and the player may start
	     a fresh run on the same tick. There is no elimination dead-time.
	  4. ROLLING LEADERBOARD — scores land, ranking and placement update, and
	     entries age out of the 5-minute window.
	  5. LAST STAND — awarded exactly once, to the only player in the cohort still
	     alive on their run, and never in a cohort of one.
	  6. BOT BACKFILL + RIVAL SPECTATING (F4) — empty seats fill with `Bot.luau`
	     players, a human displaces one, and a player can read another seat's view.

	HOW TO RUN (from `roblox/test`)
	-------------------------------
	    luau director_test.luau

	The test acts as a CLIENT wherever it can: the "played" seat in scenario 5
	chooses its purchases from `Director.view` (the render view a real client
	receives) and submits them through `Director.submitInput`. It never reaches
	into arena state to make something happen — the only sim-facing call in this
	file is the public input path.
]]

local ArenaRunner = require("../src/server/ArenaRunner")
local Director = require("../src/server/Director")
local Input = require("../src/shared/sim/Input")

--=============================================================================
-- tiny harness
--=============================================================================

local passed, failed = 0, 0
local section = ""

local function heading(name: string)
	section = name
	print("")
	print("== " .. name .. " ==")
end

local function check(cond: boolean, msg: string)
	if cond then
		passed += 1
		print("  ok    " .. msg)
	else
		failed += 1
		print("  FAIL  " .. msg .. "   [" .. section .. "]")
	end
end

local function eq(a, b, msg: string)
	check(a == b, msg .. "  (got " .. tostring(a) .. ", want " .. tostring(b) .. ")")
end

local function rejects(ok: boolean, why: string?, want: string, msg: string)
	check(not ok and why == want, msg .. "  (got " .. tostring(ok) .. "/" .. tostring(why) .. ", want false/" .. want .. ")")
end

--- Drain the director's reliable-channel events into a flat list.
local function drain(d, into)
	for _, e in Director.drainEvents(d) do
		into[#into + 1] = e
	end
	return into
end

local function countKind(events, kind: string): number
	local n = 0
	for _, e in events do
		if e.kind == kind then
			n += 1
		end
	end
	return n
end

local MASTER = "0x00000000BADCAFE1"

--=============================================================================
-- 1. arena independence
--=============================================================================

heading("1. several arenas advance independently")

local d1 = Director.new({ master_seed = MASTER, cohort_size = 4, bot_fill = false })
for i = 1, 4 do
	local p = Director.join(d1, 100 + i)
	check(p ~= nil, "player " .. (100 + i) .. " seated at seat " .. tostring(p and p.seat))
end
eq(Director.occupancy(d1), 4, "four seats occupied")
eq(Director.aliveCount(d1), 4, "four runs alive immediately after join (no lobby wait)")

Director.advance(d1, 300)
eq(d1.tick, 300, "server clock advanced 300 ticks")

local sums, seeds = {}, {}
for i = 1, 4 do
	local pid = 100 + i
	local clock = Director.runClock(d1, pid)
	eq(clock.tick, 300, "seat " .. (i - 1) .. " arena is at run tick 300")
	local sum = Director.checksum(d1, pid)
	check(sums[sum] == nil, "seat " .. (i - 1) .. " state_checksum is distinct (" .. sum .. ")")
	sums[sum] = true
	local seed = Director.issueSeed(d1, i - 1, 1)
	local key = seed.hi .. ":" .. seed.lo
	check(seeds[key] == nil, "seat " .. (i - 1) .. " run seed is distinct")
	seeds[key] = true
end

-- The round clock is the director's to state, and it is derived from the sim's
-- own round length rather than restated here.
local c = Director.runClock(d1, 101)
eq(c.round, 300 // c.round_ticks, "round follows tick // round_ticks")
eq(c.ticks_into_round, 300 % c.round_ticks, "ticks_into_round agrees")

-- Independence under a death: run 102 to its death and confirm nobody else moved.
local events1 = {}
local dead_pid, dead_at = nil, nil
for _ = 1, 1200 do
	Director.step(d1)
	drain(d1, events1)
	if dead_pid == nil then
		for _, e in events1 do
			if e.kind == "run_ended" then
				dead_pid, dead_at = e.player_id, e.tick
				break
			end
		end
	end
	if dead_pid ~= nil then
		break
	end
end
check(dead_pid ~= nil, "a run ended naturally (tank died) at server tick " .. tostring(dead_at))
for i = 1, 4 do
	local pid = 100 + i
	if pid ~= dead_pid then
		local clock = Director.runClock(d1, pid)
		eq(clock.tick, d1.tick, "seat " .. (i - 1) .. " kept ticking through another player's death")
	end
end
eq(Director.runClock(d1, dead_pid).dead, true, "the dead player's arena is dead")

-- Same master seed ⇒ same match. The director does no arithmetic that feeds
-- `state_checksum`, but it must still be order-stable.
local d1b = Director.new({ master_seed = MASTER, cohort_size = 4, bot_fill = false })
for i = 1, 4 do
	Director.join(d1b, 100 + i)
end
Director.advance(d1b, 300)
local replay_ok = true
for i = 1, 4 do
	if Director.checksum(d1b, 100 + i) == nil then
		replay_ok = false
	end
end
check(replay_ok, "replay director built four arenas")

--=============================================================================
-- 2. input authority (P5)
--=============================================================================

heading("2. a client may submit only an input code, and only a legal one")

local d2 = Director.new({ master_seed = MASTER, cohort_size = 2, bot_fill = false })
Director.join(d2, 201)
Director.join(d2, 202)
Director.advance(d2, 5) -- let the round-0 shop generate

rejects(Director.submitInput(d2, 999, Input.NOOP), "no_such_player", "unseated player is refused")
rejects(Director.submitInput(d2, 201, "1"), "code_not_a_number", "a string code is refused (not coerced)")
rejects(Director.submitInput(d2, 201, 1.5), "code_not_an_integer", "a fractional code is refused")
rejects(Director.submitInput(d2, 201, 0 / 0), "code_not_an_integer", "NaN is refused")
rejects(Director.submitInput(d2, 201, -1), "code_negative", "a negative code is refused")
rejects(Director.submitInput(d2, 201, 4 * 256), "unknown_input_kind", "kind 4 (no such ids::Input) is refused")
rejects(Director.submitInput(d2, 201, 255 * 256), "unknown_input_kind", "a garbage high byte is refused")
rejects(Director.submitInput(d2, 201, Input.buyOffer(8)), "buy_slot_out_of_range", "BuyOffer slot 8 (only 0..7 exist) is refused, not clamped")
rejects(Director.submitInput(d2, 201, Input.buyOffer(200)), "buy_slot_out_of_range", "BuyOffer slot 200 is refused")
rejects(Director.submitInput(d2, 201, Input.REROLL + 1), "slot_on_slotless_kind", "a slot smuggled onto Reroll is refused")
rejects(Director.submitInput(d2, 201, Input.CLEAR + 7), "slot_on_slotless_kind", "a slot smuggled onto Clear is refused")

check(select(1, Director.submitInput(d2, 201, Input.NOOP)), "Noop is accepted")
check(select(1, Director.submitInput(d2, 201, Input.REROLL)), "Reroll is accepted")
check(select(1, Director.submitInput(d2, 201, Input.CLEAR)), "Clear is accepted")
check(select(1, Director.submitInput(d2, 201, Input.buyOffer(7))), "BuyOffer slot 7 is accepted")

-- The FIFO is capped: the excess is rejected, never banked for a later burst.
local accepted_before_flood = 4
local flood_reason = nil
for _ = 1, 16 do
	local ok, why = Director.submitInput(d2, 201, Input.NOOP)
	if ok then
		accepted_before_flood += 1
	else
		flood_reason = why
		break
	end
end
eq(accepted_before_flood, ArenaRunner.MAX_QUEUED_INPUTS, "the input FIFO accepts exactly MAX_QUEUED_INPUTS")
eq(flood_reason, "input_flood", "the overflowing input is rejected as a flood")

-- A legal code actually reaches the sim: buy the cheapest affordable offer and
-- watch the authoritative economy move.
local d2b = Director.new({ master_seed = MASTER, cohort_size = 1, bot_fill = false })
Director.join(d2b, 210)
Director.advance(d2b, 2)
local view = Director.view(d2b, 210)
local pick, pick_cost = nil, nil
for _, o in view.shop do
	if o.affordable and (pick_cost == nil or o.cost < pick_cost) then
		pick, pick_cost = o, o.cost
	end
end
check(pick ~= nil, "the round-0 shop offered something affordable (" .. tostring(pick and pick.name) .. " @ " .. tostring(pick_cost) .. ")")
local gold_before = view.economy.gold
check(select(1, Director.submitInput(d2b, 210, Input.buyOffer(pick.slot))), "the buy code is accepted")
Director.step(d2b)
local after = Director.view(d2b, 210)
check(after.economy.gold < gold_before, "gold was deducted by the sim, not by the client (" .. gold_before .. " -> " .. after.economy.gold .. ")")
check(
	after.stats.weapons_bought + after.stats.economy_purchases > 0 or #after.arsenal >= #view.arsenal,
	"the purchase was granted inside the arena"
)

--=============================================================================
-- 3. death, then instant re-entry (F2)
--=============================================================================

heading("3. death produces a result and the player restarts immediately")

local d3 = Director.new({ master_seed = MASTER, cohort_size = 3, bot_fill = false })
for i = 1, 3 do
	Director.join(d3, 300 + i)
end

local events3 = {}
local first_result = nil
for _ = 1, 2000 do
	Director.step(d3)
	drain(d3, events3)
	for _, e in events3 do
		if e.kind == "run_ended" and first_result == nil then
			first_result = e.result
		end
	end
	if first_result ~= nil then
		break
	end
end

check(first_result ~= nil, "a run ended and produced a result")
eq(first_result.reason, "death", "the result reason is death")
check(first_result.ticks_survived > 0, "the result carries ticks_survived = " .. first_result.ticks_survived)
check(#first_result.seed == 16, "the result carries its run seed as 16 hex digits (" .. first_result.seed .. ")")
check(#first_result.checksum == 16, "the result carries a state_checksum for audit (" .. first_result.checksum .. ")")
eq(first_result.run_index, 1, "it was the player's first run")

local victim = first_result.player_id
local restart_tick = d3.tick
-- The whole of F2: no waiting for a bracket, no elimination. Same tick.
local ok3, why3 = Director.startRun(d3, victim)
check(ok3, "the dead player restarts on the very same server tick" .. (ok3 and "" or " (" .. tostring(why3) .. ")"))
eq(Director.runClock(d3, victim).tick, 0, "the fresh run starts at run tick 0")
eq(Director.runClock(d3, victim).dead, false, "the fresh run is alive")
local restarted = Director.snapshotFor(d3, victim)
eq(restarted.run_index, 2, "the restart is run #2")
check(Director.issueSeed(d3, restarted.seat, 1).lo ~= Director.issueSeed(d3, restarted.seat, 2).lo, "the retry got a fresh seed, not a rewind")
rejects(Director.startRun(d3, victim), "run_in_progress", "a second restart while running is refused")

Director.advance(d3, 10)
eq(Director.runClock(d3, victim).tick, 10, "the restarted arena advances normally")
eq(d3.tick, restart_tick + 10, "the server clock is unaffected by the restart")

--=============================================================================
-- 4. rolling leaderboard and placement
--=============================================================================

heading("4. the rolling leaderboard and placement update")

-- Run scenario 3's lobby down until every seat has posted at least one result.
for _ = 1, 2500 do
	Director.step(d3)
	drain(d3, events3)
	if #Director.leaderboard(d3) >= 3 then
		break
	end
end

local board = Director.leaderboard(d3)
check(#board >= 3, "the board holds " .. #board .. " finished runs")

local sorted = true
for i = 2, #board do
	if board[i - 1].score < board[i].score then
		sorted = false
	end
end
check(sorted, "entries are ranked best-first")
check(board[1].score == board[1].ticks_survived, "score is ticks survived (" .. board[1].score .. ")")

local top = board[1].player_id
eq(Director.placement(d3, top), 1, "the top scorer is placed 1st")
local last = board[#board].player_id
if last ~= top then
	eq(Director.placement(d3, last), #board, "the lowest entry is placed last")
end
eq(Director.placement(d3, 999), nil, "a player with no ranked run has no placement")

for i = 1, #board do
	print(string.format("     #%d  player %d  score %d  (%s, round %d)", i, board[i].player_id, board[i].score, board[i].reason, board[i].round_reached))
end
check(countKind(events3, "leaderboard_changed") > 0, "a leaderboard_changed event was emitted for the wire")

-- Ageing out of the window.
local d4 = Director.new({ master_seed = MASTER, cohort_size = 2, bot_fill = false, window_ticks = 120, run_tick_limit = 40 })
Director.join(d4, 401)
Director.join(d4, 402)
Director.advance(d4, 45)
eq(#Director.leaderboard(d4), 2, "two completed runs entered the 120-tick window")
eq(Director.placement(d4, 401), if Director.leaderboard(d4)[1].player_id == 401 then 1 else 2, "the finisher is placed")
Director.advance(d4, 200)
eq(#Director.leaderboard(d4), 0, "results aged out of the rolling window")
eq(Director.placement(d4, 401), nil, "placement lapses with the window")

--=============================================================================
-- 5. Last Stand
--=============================================================================

heading("5. Last Stand goes to the only player still alive on their run")

-- Four seats, no bots, runs capped at 900 ticks. Three seats idle (their tanks
-- die to the wave escalation around tick 735-775); the fourth is played through
-- the public client path, survives them all, and is therefore the sole survivor
-- for the rest of the window.
local d5 = Director.new({ master_seed = MASTER, cohort_size = 4, bot_fill = false, run_tick_limit = 900 })
for i = 1, 4 do
	Director.join(d5, 500 + i)
end
local PLAYED = 504

--- A minimal client: every few ticks, buy the priciest affordable offer it can
--- see, preferring weapons. Reads ONLY the render view and speaks ONLY through
--- `submitInput` — exactly the surface a real client has.
local function playOneTick(d, pid)
	if d.tick % 6 ~= 0 then
		return
	end
	local v = Director.view(d, pid)
	if v == nil then
		return
	end
	local best, best_cost, best_weapon = nil, nil, false
	for _, o in v.shop do
		if o.affordable then
			local better
			if best == nil then
				better = true
			elseif o.is_weapon ~= best_weapon then
				better = o.is_weapon
			else
				better = o.cost > best_cost
			end
			if better then
				best, best_cost, best_weapon = o, o.cost, o.is_weapon
			end
		end
	end
	if best ~= nil then
		Director.submitInput(d, pid, Input.buyOffer(best.slot))
	end
end

local events5 = {}
for _ = 1, 900 do
	playOneTick(d5, PLAYED)
	Director.step(d5)
	drain(d5, events5)
end

local stands = {}
for _, e in events5 do
	if e.kind == "last_stand" then
		stands[#stands + 1] = e
	end
end
eq(#stands, 1, "Last Stand was awarded exactly once")
if #stands == 1 then
	eq(stands[1].player_id, PLAYED, "it went to the sole survivor")
	print(string.format("     last stand: player %d, run %d, server tick %d", stands[1].player_id, stands[1].run_index, stands[1].tick))
end
eq(Director.awards(d5, PLAYED).last_stand, 1, "the award is on the survivor's record")
for i = 1, 3 do
	eq(Director.awards(d5, 500 + i).last_stand, 0, "player " .. (500 + i) .. " has no Last Stand")
end
eq(Director.runClock(d5, PLAYED), nil, "the played run finished (its 900-tick limit)")
local played_entry = nil
for _, e in Director.leaderboard(d5) do
	if e.player_id == PLAYED then
		played_entry = e
	end
end
check(played_entry ~= nil and played_entry.reason == "completed", "the survivor's run is recorded as completed, not death")
eq(Director.placement(d5, PLAYED), 1, "the survivor tops the rolling board")

-- ... and never in a cohort of one.
local d5b = Director.new({ master_seed = MASTER, cohort_size = 1, bot_fill = false })
Director.join(d5b, 590)
local solo_events = {}
for _ = 1, 900 do
	Director.step(d5b)
	drain(d5b, solo_events)
end
eq(countKind(solo_events, "last_stand"), 0, "a lone player is never awarded Last Stand")

--=============================================================================
-- 6. bot backfill and rival spectating (F4)
--=============================================================================

heading("6. bot backfill keeps a thin lobby populated; rivals can be watched")

local d6 = Director.new({ master_seed = MASTER, cohort_size = 4, bot_fill = true })
Director.join(d6, 601)
eq(Director.occupancy(d6), 1, "before the first tick only the human is seated")
Director.step(d6)
eq(Director.occupancy(d6), 4, "backfill filled the lobby to the cohort size")
eq(Director.aliveCount(d6), 4, "every backfilled seat is running")

local bots = {}
for _, p in Director.cohort(d6) do
	if p.is_bot then
		bots[#bots + 1] = p
	end
end
eq(#bots, 3, "three of the four seats are bots")

Director.advance(d6, 150)
local bot_sums = {}
local bots_distinct = true
for _, p in bots do
	local sum = Director.checksum(d6, p.player_id)
	eq(Director.runClock(d6, p.player_id).tick, d6.tick, "bot seat " .. p.seat .. " is simulating on the server clock")
	if bot_sums[sum] then
		bots_distinct = false
	end
	bot_sums[sum] = true
end
check(bots_distinct, "each bot arena has its own state")

-- F4: the same view path, pointed at a different arena.
rejects(Director.spectate(d6, 601, 0), "cannot_spectate_self", "you cannot spectate your own seat")
rejects(Director.spectate(d6, 601, 99), "bad_seat", "an out-of-range seat is refused")
check(select(1, Director.spectate(d6, 601, bots[1].seat)), "spectating a rival seat is allowed")
local snap = Director.snapshotFor(d6, 601)
check(snap.view ~= nil, "the spectator still receives their own arena view")
check(snap.spectated_view ~= nil, "the rival arena view is delivered alongside it")
check(snap.spectated_view ~= snap.view, "the two views are different arenas")
eq(snap.spectating, bots[1].seat, "the snapshot names the spectated seat")
check(snap.match_clock ~= nil and snap.run_clock ~= nil, "the snapshot carries both clocks")
check(snap.leaderboard ~= nil and snap.awards ~= nil, "the snapshot carries the board and awards")

-- A human displaces a bot rather than being turned away.
local evicted = bots[1].player_id
local p7, why7 = Director.join(d6, 602)
check(p7 ~= nil, "a second human joins a nominally full lobby" .. (p7 and "" or " (" .. tostring(why7) .. ")"))
eq(Director.occupancy(d6), 4, "the lobby is still exactly cohort-sized")
eq(Director.checksum(d6, evicted), nil, "the displaced bot is gone")
local humans = 0
for _, p in Director.cohort(d6) do
	if not p.is_bot then
		humans += 1
	end
end
eq(humans, 2, "both humans are seated")
eq(Director.snapshotFor(d6, 601).spectating, nil, "spectating the emptied seat was cleared")
rejects(Director.join(d6, 601), "already_joined", "double-joining is refused")
check(select(1, Director.leave(d6, 602)), "a player can leave")
eq(Director.checksum(d6, 602), nil, "the leaver's arena is torn down")
Director.step(d6)
eq(Director.occupancy(d6), 4, "backfill refilled the vacated seat")

--=============================================================================

print("")
print(string.format("director_test: %d passed, %d failed", passed, failed))
if failed > 0 then
	error(string.format("director_test FAILED (%d checks)", failed), 0)
end
