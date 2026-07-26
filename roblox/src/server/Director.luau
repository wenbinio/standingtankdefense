--[[
	Director.luau — the authoritative match layer (`docs/10` F2/F6,
	`docs/03 §3.2`).

	The director is deliberately SMALL. Everything competitively load-bearing
	lives here and nowhere else: seat assignment, seed issue, the server clock,
	the rolling leaderboard, the Last Stand award, and the single gate every
	client input passes through. Everything else — the actual simulating — is
	`ArenaRunner`, one per seat, mutually independent.

	THE CENTRAL DECISION: A RUN IS NOT A MATCH (F2)
	----------------------------------------------
	`docs/09 §2.3` found elimination to be a structural problem on Roblox: half
	the lobby sits dead with nothing to do, and they leave rather than spectate.
	So this director does NOT run an elimination bracket, and adding one back
	would be a redesign, not a fix.

	  * **Runs are solo and instantly restartable.** Death produces a result and
	    the player may `startRun` again on the very next tick. There is no
	    lobby-wide round to wait for, no elimination, no dead time. This is free
	    architecturally — an arena never depended on anyone else's.
	  * **The competitive layer is a rolling 5-minute leaderboard** over the run
	    scores that finished inside the window. Placement is a live property of
	    the window, not the outcome of a bracket.
	  * **Last Stand survives as an AWARD**, granted when you are the only player
	    in the current cohort still alive on their run. It keeps the prestige
	    objective (`docs/02 §2.7`) and costs nobody any dead time.

	AUTHORITY (`PHASE2-CONTRACTS.md` P5)
	------------------------------------
	A client sends one thing: an input code. `Director.submitInput` is the only
	door, and it refuses — it never clamps — malformed codes, codes for a seat
	that is not yours, codes with no live run, and floods. Seeds, the clock,
	alive/dead truth, scores and placement are all decided here from server state
	that no client can touch.

	DETERMINISM OF THE MATCH LAYER
	------------------------------
	The sim's determinism invariants are not this file's to break (it never does
	arithmetic that feeds `state_checksum`), but the director is still written to
	be order-stable: every iteration over participants goes through the SEAT
	array in seat order, never `pairs()` over a hash map, and the leaderboard
	sort is a total order with explicit tie-breaks. Two directors fed the same
	master seed and the same input timeline produce the same match.

	REPLICATION SEAM
	----------------
	`Wire.luau` does not exist yet, so nothing here imports it. The seam is two
	plain-data functions — `drainEvents` (the reliable low-rate channel: run
	started/ended, leaderboard changed, Last Stand) and `snapshotFor` (one
	player's view + optional spectated view + clock + leaderboard). When the wire
	lands it packs these; nothing above it has to change.

	No Roblox global is referenced: the whole match layer runs, and is tested,
	under the bare `luau` interpreter.
]]

local ArenaRunner = require("./ArenaRunner")
local Bot = require("../shared/sim/Bot")
local Rng = require("../shared/Rng")

local Director = {}
Director.__index = Director

--- The rolling leaderboard window: 5 minutes at 30 Hz (F2's "rolling 5-minute
--- server leaderboard"). A result older than this is no longer ranked.
Director.LEADERBOARD_WINDOW_TICKS = 5 * 60 * 30

--- Default lobby size (`docs/09 §1.3`'s 8-arena throughput budget).
Director.DEFAULT_COHORT_SIZE = 8

--- How long a backfill bot waits before starting its next run. Humans restart
--- when they press retry; bots need *some* delay or a cohort could never drop to
--- one alive and Last Stand would be unreachable. 1.5 s is roughly the "~3
--- seconds to retry" F2 describes for a human, halved — bots should not be the
--- reason a human loses the award, nor the reason it is trivially handed out.
Director.BOT_RESTART_DELAY_TICKS = 45

--- Minimum cohort size for Last Stand to mean anything. Being the last one alive
--- when you are the only one here is not an achievement.
Director.LAST_STAND_MIN_COHORT = 2

--- RNG purpose id for run-seed derivation. `ids::Purpose` owns 0..4 for the
--- sim's internal streams (`State.Purpose`); this sits far outside that range so
--- a run seed can never collide with an in-sim stream, and so adding a Purpose
--- later cannot silently start reusing it.
local PURPOSE_RUN_SEED = 0x5244 -- "RD"

--- Synthetic OWNER ids for backfill bots. These are director keys only (never
--- the sim's `player_id`, which is the seat), so they only have to be
--- recognizable and never collide with a Roblox `UserId`. `UserId`s are i64 and
--- allocated upward from 1, so a block far above the issued range is safe.
local BOT_ID_BASE = 1e15

export type Seed = ArenaRunner.Seed

export type Entry = {
	player_id: number,
	is_bot: boolean,
	run_index: number,
	score: number,
	ticks_survived: number,
	damage_dealt: number,
	gold_earned: number,
	round_reached: number,
	reason: string,
	finished_at: number,
}

export type Participant = {
	player_id: number,
	seat: number,
	is_bot: boolean,
	runner: ArenaRunner.Runner,
	joined_at: number,
	runs: number,
	--- Server tick at which this seat may auto-start its next run (bots only).
	restart_at: number?,
	--- Last Stand already granted for the CURRENT run (reset on every start).
	last_stand_this_run: boolean,
	--- Career counters for this session.
	last_stand_awards: number,
	best_score: number,
	--- F4: the seat this player is watching, if any.
	spectating: number?,
}

-- =========================================================================
-- construction
-- =========================================================================

--- `opts`:
---   `master_seed`   the match seed (number / decimal string / "0x…" / {hi,lo}).
---                   Every per-run seed is derived from it — the director owns
---                   RNG issue and no client ever supplies a seed.
---   `cohort_size`   seats (default 8).
---   `bot_fill`      backfill empty seats with `Bot.luau` players (default true).
---   `window_ticks`  leaderboard window (default 5 min).
---   `run_tick_limit` run tick at which a surviving run is `completed`. `nil` =
---                   runs end only on death. See the note on `runLimit` below.
---   `timeline`      `Content.TIMELINE.roblox` for the F1 schedule, `nil` for
---                   Steam (P3 default). Passed through to every runner.
---   `bot_restart_delay` override for `BOT_RESTART_DELAY_TICKS`.
function Director.new(opts)
	opts = opts or {}
	local hi, lo = Rng.seedParts(opts.master_seed or 0)
	local cohort = opts.cohort_size or Director.DEFAULT_COHORT_SIZE
	if type(cohort) ~= "number" or cohort % 1 ~= 0 or cohort < 1 then
		error("Director.new: cohort_size must be a positive integer", 2)
	end
	local self = setmetatable({
		master_seed = table.freeze({ hi = hi, lo = lo }),
		--- The authoritative server clock. Runs carry their own tick; this one is
		--- the match's, and it is what the leaderboard window is measured in.
		tick = 0,
		cohort_size = cohort,
		bot_fill = opts.bot_fill ~= false,
		window_ticks = opts.window_ticks or Director.LEADERBOARD_WINDOW_TICKS,
		run_tick_limit = opts.run_tick_limit,
		timeline = opts.timeline,
		bot_restart_delay = opts.bot_restart_delay or Director.BOT_RESTART_DELAY_TICKS,
		--- Seats, 1-indexed internally; the seat NUMBER exposed to callers is
		--- 0-based, matching every other index in this codebase (§S3). `false`
		--- marks an empty seat, so the array is dense and iteration is order-stable.
		seats = {},
		by_player = {},
		--- Finished runs inside the window, newest last.
		board = {},
		--- Reliable-channel events since the last `drainEvents`.
		events = {},
		next_bot_seq = 0,
		--- Set when the board changed this tick, so the wire can skip resending.
		board_dirty = false,
	}, Director)
	for i = 1, cohort do
		self.seats[i] = false
	end
	return self
end

-- =========================================================================
-- seeds and clock — owned here, never by a client (`docs/03 §3.2`)
-- =========================================================================

--- Derive the seed for `(seat, run_index)` from the master seed. Pure: the same
--- director state always issues the same seed, so a run is reproducible from
--- `(master_seed, seat, run_index)` alone — which is what makes a result
--- auditable later against `ArenaRunner.checksum`.
function Director.issueSeed(self, seat: number, run_index: number): Seed
	local r = Rng.derive(self.master_seed, seat, PURPOSE_RUN_SEED, run_index)
	local hi, lo = Rng.state(r)
	return table.freeze({ hi = hi, lo = lo })
end

--- The tick limit a run is opened with. Kept in one place because F1/F2 pin the
--- boss at the END of a 5-minute run but do NOT say what ends a run the player
--- survives; `nil` (death only) is the conservative reading and the caller can
--- override. FLAGGED, not decided here.
local function runLimit(self): number?
	return self.run_tick_limit
end

--- The authoritative match clock, for the reliable channel.
function Director.clock(self)
	return {
		tick = self.tick,
		window_ticks = self.window_ticks,
		cohort_size = self.cohort_size,
		occupied = Director.occupancy(self),
		alive = Director.aliveCount(self),
	}
end

-- =========================================================================
-- seats
-- =========================================================================

local function emit(self, event)
	self.events[#self.events + 1] = event
end

--- Lowest free seat, or nil.
local function freeSeat(self): number?
	for i = 1, self.cohort_size do
		if self.seats[i] == false then
			return i - 1
		end
	end
	return nil
end

--- Lowest-seated bot, or nil — the one a joining human evicts.
local function lowestBot(self): Participant?
	for i = 1, self.cohort_size do
		local p = self.seats[i]
		if p ~= false and p.is_bot then
			return p
		end
	end
	return nil
end

local function seatParticipant(self, player_id: number, seat: number, is_bot: boolean, challenge): Participant
	local p: Participant = {
		player_id = player_id,
		seat = seat,
		is_bot = is_bot,
		runner = ArenaRunner.new({
			-- THE SIM'S player id is the SEAT, not the account id. `player_id` is
			-- hashed into `state_checksum` as a u32, and a Roblox `UserId` is an
			-- i64 whose live values already exceed `u32::MAX` — passing one
			-- through would truncate inside the digest. Seat ids also make a run
			-- reproducible from `(master_seed, seat, run_index)`, which is what
			-- `issueSeed` promises.
			player_id = seat,
			owner_id = player_id,
			is_bot = is_bot,
			tick_limit = runLimit(self),
			timeline = self.timeline,
			challenge = challenge,
		}),
		joined_at = self.tick,
		runs = 0,
		restart_at = nil,
		last_stand_this_run = false,
		last_stand_awards = 0,
		best_score = 0,
		spectating = nil,
	} :: any
	self.seats[seat + 1] = p
	self.by_player[player_id] = p
	return p
end

--- Free a seat, abandoning any live run. Internal: `leave` and bot eviction.
local function unseat(self, p: Participant)
	ArenaRunner.abandon(p.runner, self.tick)
	ArenaRunner.reset(p.runner)
	self.seats[p.seat + 1] = false
	self.by_player[p.player_id] = nil
	-- Nobody keeps spectating a seat that emptied.
	for i = 1, self.cohort_size do
		local q = self.seats[i]
		if q ~= false and q.spectating == p.seat then
			q.spectating = nil
		end
	end
end

--- Seat a real player and open their first run. `player_id` is the caller's
--- account key — a Roblox `UserId` — and is NOT bounded to a u32 (it is never
--- the sim's player id; see `seatParticipant`). Returns the participant, or
--- `nil, reason`. A human displaces a backfill bot when the lobby is nominally
--- full — bots exist to make a thin lobby feel populated, never to keep a
--- player out.
function Director.join(self, player_id: number): (Participant?, string?)
	if type(player_id) ~= "number" or player_id % 1 ~= 0 or player_id < 0 then
		return nil, "bad_player_id"
	end
	if self.by_player[player_id] ~= nil then
		return nil, "already_joined"
	end
	local seat = freeSeat(self)
	if seat == nil then
		local bot = lowestBot(self)
		if bot == nil then
			return nil, "lobby_full"
		end
		seat = bot.seat
		unseat(self, bot)
		emit(self, { kind = "seat_released", player_id = bot.player_id, seat = seat, tick = self.tick })
	end
	local p = seatParticipant(self, player_id, seat :: number, false, nil)
	emit(self, { kind = "joined", player_id = player_id, seat = p.seat, tick = self.tick })
	Director.startRun(self, player_id)
	return p
end

--- Remove a player. Their in-flight run is abandoned and NOT scored — a
--- disconnect must never be a way to bank or dodge a result.
function Director.leave(self, player_id: number): (boolean, string?)
	local p = self.by_player[player_id]
	if p == nil then
		return false, "no_such_player"
	end
	unseat(self, p)
	emit(self, { kind = "left", player_id = player_id, seat = p.seat, tick = self.tick })
	return true
end

--- Participants in seat order. The canonical iteration order for anything that
--- must be deterministic.
function Director.cohort(self): { Participant }
	local out = {}
	for i = 1, self.cohort_size do
		local p = self.seats[i]
		if p ~= false then
			out[#out + 1] = p
		end
	end
	return out
end

function Director.occupancy(self): number
	local n = 0
	for i = 1, self.cohort_size do
		if self.seats[i] ~= false then
			n += 1
		end
	end
	return n
end

--- How many participants are simulating a live run right now.
function Director.aliveCount(self): number
	local n = 0
	for i = 1, self.cohort_size do
		local p = self.seats[i]
		if p ~= false and ArenaRunner.isAlive(p.runner) then
			n += 1
		end
	end
	return n
end

-- =========================================================================
-- bot backfill
-- =========================================================================

--- Fill empty seats so a thin lobby still feels populated (F2/`docs/09 §2.3`).
--- Bots are ordinary participants driven by `Bot.luau`: they take a director-
--- issued seed like anyone else, they land on the leaderboard like anyone else,
--- and they are the first thing evicted when a human arrives.
local function backfill(self)
	if not self.bot_fill then
		return
	end
	while true do
		local seat = freeSeat(self)
		if seat == nil then
			break
		end
		local id = BOT_ID_BASE + self.next_bot_seq
		self.next_bot_seq += 1
		-- The challenge is a purchase bias only (`Bot.luau`); deriving it from the
		-- master seed and the seat keeps a lobby's bot personalities stable and
		-- reproducible instead of wall-clock random.
		local r = Rng.derive(self.master_seed, seat, PURPOSE_RUN_SEED, 0xB07)
		-- Codes 0..8 only: 9 is `EcoOnly`, which `Bot.luau` marks MEASUREMENT
		-- ONLY (it buys no weapons at all and dies almost immediately). A live
		-- lobby should not be backfilled with a probe.
		local challenge = Bot.challengeFromCode(Rng.below(r, 9))
		local p = seatParticipant(self, id, seat, true, challenge)
		emit(self, { kind = "joined", player_id = id, seat = p.seat, tick = self.tick, is_bot = true })
		Director.startRun(self, id)
	end
end

--- Top the lobby back up to `cohort_size` with bots. Called by `step`; exposed
--- so a host can force it at match start.
function Director.backfill(self)
	backfill(self)
end

-- =========================================================================
-- run lifecycle — F2's instant re-entry
-- =========================================================================

--- Start (or restart) `player_id`'s run. This is the whole of "instant retry":
--- it is legal the moment the previous run finished, it costs one tick, and it
--- issues a FRESH director-owned seed so a retry is a new run, not a rewind.
function Director.startRun(self, player_id: number): (boolean, string?)
	local p = self.by_player[player_id]
	if p == nil then
		return false, "no_such_player"
	end
	if ArenaRunner.isAlive(p.runner) then
		return false, "run_in_progress"
	end
	p.runs += 1
	p.last_stand_this_run = false
	p.restart_at = nil
	local seed = Director.issueSeed(self, p.seat, p.runs)
	ArenaRunner.start(p.runner, seed, p.runs, self.tick)
	emit(self, {
		kind = "run_started",
		player_id = player_id,
		seat = p.seat,
		run_index = p.runs,
		tick = self.tick,
	})
	return true
end

-- =========================================================================
-- input — THE ONLY THING A CLIENT MAY SEND (P5)
-- =========================================================================

--- Submit one input code on behalf of `player_id`.
---
--- THE VALIDATION PATH, end to end:
---   1. the caller (the `RemoteEvent` handler) supplies the player id from the
---      Roblox `Player` argument the ENGINE provides — never from the payload,
---      so a client cannot address another seat;
---   2. the seat must exist and must not be a bot seat;
---   3. `ArenaRunner.validateCode` checks the code structurally and REJECTS
---      anything malformed or out of range (it does not clamp, and it does not
---      substitute a Noop);
---   4. the FIFO depth check rejects floods rather than banking them;
---   5. only then is the code queued, to be applied by `Arena.step` on a later
---      tick, where `Input.apply` makes the final state-dependent decision
---      (affordability, empty slot, cooldown).
---
--- Nothing else about the input is client-supplied — no tick, no target, no
--- cost, no seed. Returns `true`, or `false, reason`.
function Director.submitInput(self, player_id: number, code: any): (boolean, string?)
	local p = self.by_player[player_id]
	if p == nil then
		return false, "no_such_player"
	end
	return ArenaRunner.submit(p.runner, code)
end

-- =========================================================================
-- scoring and the rolling leaderboard
-- =========================================================================

--- A run's score. Survival IS the game (`last-man-standing`), so the score is
--- ticks survived — an integer, monotone in the thing the player is actually
--- trying to do, and impossible to inflate by farming. Damage and gold ride
--- along as tie-breaks rather than as terms, so no amount of damage ever buys a
--- rank over someone who lived longer.
---
--- Isolated in one function on purpose: this is the number balance will want to
--- change, and nothing else in the director knows how it is computed.
function Director.scoreRun(result): number
	return result.ticks_survived
end

--- Total order over entries: score, then damage, then gold, then the earlier
--- finish, then seat-stable player id. Fully specified so the board never
--- depends on table order.
local function outranks(a: Entry, b: Entry): boolean
	if a.score ~= b.score then
		return a.score > b.score
	end
	if a.damage_dealt ~= b.damage_dealt then
		return a.damage_dealt > b.damage_dealt
	end
	if a.gold_earned ~= b.gold_earned then
		return a.gold_earned > b.gold_earned
	end
	if a.finished_at ~= b.finished_at then
		return a.finished_at < b.finished_at
	end
	return a.player_id < b.player_id
end

--- Drop entries that have aged out of the rolling window.
local function prune(self)
	local cutoff = self.tick - self.window_ticks
	local board = self.board
	local kept = {}
	local dropped = false
	for i = 1, #board do
		local e = board[i]
		if e.finished_at > cutoff then
			kept[#kept + 1] = e
		else
			dropped = true
		end
	end
	if dropped then
		self.board = kept
		self.board_dirty = true
	end
end

--- Record a finished run on the board. Abandoned runs are never scored.
local function record(self, p: Participant, result)
	if result.reason == ArenaRunner.EndReason.Abandoned then
		return nil
	end
	local entry: Entry = {
		player_id = result.player_id,
		is_bot = result.is_bot,
		run_index = result.run_index,
		score = Director.scoreRun(result),
		ticks_survived = result.ticks_survived,
		damage_dealt = result.damage_dealt,
		gold_earned = result.gold_earned,
		round_reached = result.round_reached,
		reason = result.reason,
		finished_at = result.ended_at,
	}
	self.board[#self.board + 1] = entry
	self.board_dirty = true
	if entry.score > p.best_score then
		p.best_score = entry.score
	end
	return entry
end

--- The current rolling leaderboard, best first. `limit` truncates.
function Director.leaderboard(self, limit: number?): { Entry }
	local out = {}
	local board = self.board
	for i = 1, #board do
		out[i] = board[i]
	end
	table.sort(out, outranks)
	if limit ~= nil and #out > limit then
		for i = #out, limit + 1, -1 do
			out[i] = nil
		end
	end
	return out
end

--- 1-based placement of `player_id`'s BEST entry in the current window, or nil
--- if they have no ranked run in it.
function Director.placement(self, player_id: number): number?
	local board = Director.leaderboard(self)
	for i = 1, #board do
		if board[i].player_id == player_id then
			return i
		end
	end
	return nil
end

-- =========================================================================
-- Last Stand
-- =========================================================================

--- Granted when a player is the only one in the current cohort still alive on
--- their run (F2). Evaluated after every tick's deaths are harvested, at most
--- once per run, and only in a cohort of at least two — so it survives as the
--- prestige objective from `docs/02 §2.7` without reintroducing any of the dead
--- time elimination cost.
local function evaluateLastStand(self)
	if Director.occupancy(self) < Director.LAST_STAND_MIN_COHORT then
		return
	end
	local sole: Participant? = nil
	for i = 1, self.cohort_size do
		local p = self.seats[i]
		if p ~= false and ArenaRunner.isAlive(p.runner) then
			if sole ~= nil then
				return -- more than one alive: nobody is last standing
			end
			sole = p
		end
	end
	if sole == nil or sole.last_stand_this_run then
		return
	end
	sole.last_stand_this_run = true
	sole.last_stand_awards += 1
	emit(self, {
		kind = "last_stand",
		player_id = sole.player_id,
		seat = sole.seat,
		run_index = sole.runner.run_index,
		tick = self.tick,
	})
end

--- Last Stand awards this session.
function Director.awards(self, player_id: number)
	local p = self.by_player[player_id]
	if p == nil then
		return nil
	end
	return {
		last_stand = p.last_stand_awards,
		best_score = p.best_score,
		runs = p.runs,
	}
end

-- =========================================================================
-- the tick
-- =========================================================================

--- Advance the match by exactly one server tick:
---   1. backfill empty seats,
---   2. auto-restart bots whose retry delay elapsed,
---   3. step every arena (in seat order),
---   4. record finished runs and age the board,
---   5. evaluate Last Stand.
---
--- Arenas are stepped independently and in a fixed order; no arena reads
--- another's state, which is precisely why step 3 can move into one `Actor` per
--- arena later without changing anything else here (F6).
function Director.step(self)
	self.tick += 1
	backfill(self)

	for i = 1, self.cohort_size do
		local p = self.seats[i]
		if p ~= false and p.is_bot and p.restart_at ~= nil and self.tick >= p.restart_at then
			Director.startRun(self, p.player_id)
		end
	end

	for i = 1, self.cohort_size do
		local p = self.seats[i]
		if p ~= false then
			local result = ArenaRunner.step(p.runner, self.tick)
			if result ~= nil then
				local entry = record(self, p, result)
				if p.is_bot then
					p.restart_at = self.tick + self.bot_restart_delay
				end
				emit(self, {
					kind = "run_ended",
					player_id = p.player_id,
					seat = p.seat,
					run_index = result.run_index,
					reason = result.reason,
					result = result,
					score = if entry ~= nil then entry.score else nil,
					tick = self.tick,
				})
			end
		end
	end

	prune(self)
	if self.board_dirty then
		self.board_dirty = false
		emit(self, { kind = "leaderboard_changed", tick = self.tick })
	end
	evaluateLastStand(self)
end

--- Advance `n` server ticks.
function Director.advance(self, n: number)
	for _ = 1, n do
		Director.step(self)
	end
end

-- =========================================================================
-- read side / replication seam
-- =========================================================================

--- F4 rival spectating: watch another OCCUPIED seat. Purely a read — it adds no
--- simulation coupling and no new determinism surface, because it is the same
--- view path pointed at a different arena.
function Director.spectate(self, player_id: number, seat: number?): (boolean, string?)
	local p = self.by_player[player_id]
	if p == nil then
		return false, "no_such_player"
	end
	if seat == nil then
		p.spectating = nil
		return true
	end
	if type(seat) ~= "number" or seat % 1 ~= 0 or seat < 0 or seat >= self.cohort_size then
		return false, "bad_seat"
	end
	if seat == p.seat then
		return false, "cannot_spectate_self"
	end
	if self.seats[seat + 1] == false then
		return false, "empty_seat"
	end
	p.spectating = seat
	return true
end

--- A player's own arena view, or nil between runs.
function Director.view(self, player_id: number)
	local p = self.by_player[player_id]
	if p == nil then
		return nil
	end
	return ArenaRunner.view(p.runner)
end

--- A player's authoritative run clock, or nil between runs.
function Director.runClock(self, player_id: number)
	local p = self.by_player[player_id]
	if p == nil then
		return nil
	end
	return ArenaRunner.clock(p.runner)
end

--- `state_checksum` of a player's live arena. The audit hook: a disputed result
--- can be replayed from `(issueSeed(seat, run_index), inputs)` and compared.
--- Read-only — building it never mutates the sim.
function Director.checksum(self, player_id: number): string?
	local p = self.by_player[player_id]
	if p == nil then
		return nil
	end
	return ArenaRunner.checksum(p.runner)
end

--- The view of the seat this player is spectating (F4), or nil.
function Director.spectatedView(self, player_id: number)
	local p = self.by_player[player_id]
	if p == nil or p.spectating == nil then
		return nil
	end
	local target = self.seats[p.spectating + 1]
	if target == false then
		return nil
	end
	return ArenaRunner.view(target.runner)
end

--- Everything one client is entitled to see, as plain data. This is the shape
--- `Wire.luau` will pack: the high-rate part is `view`/`spectated_view`, the
--- reliable low-rate part is everything else. Deliberately no `buffer` packing
--- here — that belongs to the wire, not the director.
function Director.snapshotFor(self, player_id: number, board_limit: number?)
	local p = self.by_player[player_id]
	if p == nil then
		return nil
	end
	return {
		seat = p.seat,
		run_index = p.runs,
		alive = ArenaRunner.isAlive(p.runner),
		can_retry = not ArenaRunner.isAlive(p.runner),
		view = ArenaRunner.view(p.runner),
		spectating = p.spectating,
		spectated_view = Director.spectatedView(self, player_id),
		run_clock = ArenaRunner.clock(p.runner),
		match_clock = Director.clock(self),
		leaderboard = Director.leaderboard(self, board_limit or 10),
		placement = Director.placement(self, player_id),
		last_result = ArenaRunner.result(p.runner),
		awards = Director.awards(self, player_id),
	}
end

--- Take the reliable-channel events queued since the last call, and clear them.
--- Plain tables; the wire decides how to encode them.
function Director.drainEvents(self)
	local out = self.events
	self.events = {}
	return out
end

return Director
