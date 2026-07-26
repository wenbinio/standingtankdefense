--[[
	ArenaRunner.luau — one player's arena, server-side (`docs/10` F6).

	The server simulates every arena and clients render only, so this module is
	the thing that actually owns a live `ArenaState`: it holds the state, accepts
	validated input codes, steps the sim at 30 Hz, decides when a run is over, and
	exposes the render view. One runner == one player == one arena. Runners never
	reference each other — that is the arena-independence property F6 leans on.

	WHY THIS FILE HAS NO ROBLOX GLOBAL IN IT
	----------------------------------------
	F6 puts each arena in its own parallel Luau `Actor`. An `Actor` is an
	isolation boundary: what crosses it is plain data, and what runs inside it
	should not need the surrounding script tree. So the stepping core below is
	written as a pure state machine —

	    in:  a seed, a player id, input CODES (integers)
	    out: a view table, a result table

	— and nothing else. It runs unchanged under the bare `luau` interpreter,
	which is how it is tested, and it will run unchanged inside an `Actor`.
	The only wall-clock-aware code is `ArenaRunner.pump`, fenced off at the bottom
	under SCHEDULING SEAM; it decides *how many* ticks to run and never touches
	sim math. The `Heartbeat`/`Actor` glue that calls it is a separate file.

	INPUT AUTHORITY (`PHASE2-CONTRACTS.md` P5)
	------------------------------------------
	A client may submit exactly one thing: an input code (`SIM-SPEC` §S6,
	`kind * 256 + slot`). `ArenaRunner.validateCode` is the structural gate and it
	REJECTS rather than clamps — a malformed or out-of-range code is an error the
	caller is told about, never a silently-substituted Noop. Semantic outcomes
	(can you afford it? is the slot empty?) stay where they already live, inside
	`Input.apply`; this gate only decides whether the integer is a legal code at
	all, which makes it stateless and therefore identical on every host.

	ONE INPUT PER TICK
	------------------
	`Arena.step` takes one input per tick — that is the sim's contract, not a
	simplification here. Submitted codes therefore sit in a small FIFO and are
	drained one per tick. The queue is capped (`MAX_QUEUED_INPUTS`): a client that
	spams gets its excess REJECTED, not buffered, so it can never bank a burst of
	actions to spend later.

	RUN LIFECYCLE (F2)
	------------------
	A run is solo and disposable: `start` → ticks → `death` or `completed` →
	`result()` → `start` again. The runner is reused across runs; each `start`
	takes a fresh seed from the director. There is no elimination state here and
	no cross-player coupling — see `Director.luau` for the match layer.
]]

local Arena = require("../shared/sim/Arena")
local Bot = require("../shared/sim/Bot")
local Input = require("../shared/sim/Input")
local Rng = require("../shared/Rng")
local Shop = require("../shared/sim/Shop")
local View = require("../shared/sim/View")

local ArenaRunner = {}
ArenaRunner.__index = ArenaRunner

--- Simulation frequency, restated from the sim so schedulers do not have to
--- reach into `Arena` for it.
ArenaRunner.TICK_HZ = Arena.TICK_HZ
--- Seconds per tick, for the scheduling seam only.
local TICK_DT = 1 / Arena.TICK_HZ

--- How many un-applied input codes a runner will hold. One is drained per tick,
--- so this is a third of a second of buffered intent — enough to absorb network
--- jitter, far too little to bank a combo.
ArenaRunner.MAX_QUEUED_INPUTS = 8

--- Never step more than this many ticks in one `pump` call. A hitching server
--- must fall behind rather than spend an unbounded slice catching up.
ArenaRunner.MAX_CATCHUP_TICKS = 8

--- Run status. `idle` = between runs (instant retry is available), `running` =
--- simulating, `finished` = a result is waiting to be collected.
ArenaRunner.Status = table.freeze({
	Idle = "idle",
	Running = "running",
	Finished = "finished",
})

--- Why a run ended.
ArenaRunner.EndReason = table.freeze({
	--- The tank died (`Economy.resolveDeaths`).
	Death = "death",
	--- The run reached its tick limit with the tank alive (a survived run).
	Completed = "completed",
	--- The player left / the seat was reclaimed mid-run.
	Abandoned = "abandoned",
})

export type Seed = { hi: number, lo: number }

export type RunResult = {
	--- The OWNER — the director's key for whoever played this run (a Roblox
	--- `UserId`, or a synthetic id for a backfill bot). Not the sim's id.
	player_id: number,
	--- The sim's `player_id` for the arena, i.e. the seat. See `ArenaRunner.new`.
	seat: number,
	is_bot: boolean,
	run_index: number,
	seed: string,
	reason: string,
	started_at: number,
	ended_at: number,
	ticks_survived: number,
	round_reached: number,
	damage_dealt: number,
	gold_earned: number,
	weapons_owned: number,
	weapons_bought: number,
	economy_purchases: number,
	checksum: string,
}

-- =========================================================================
-- input validation (P5) — structural, stateless, rejecting
-- =========================================================================

local KIND_BUY = Input.Kind.BuyOffer
local KIND_CLEAR = Input.Kind.Clear
local OFFER_SLOTS = Shop.OFFER_SLOTS

--- Is `code` a legal input code at all? Returns `true`, or `false, reason`.
---
--- The checks are exactly the ones that hold for every state, so this answer
--- never depends on when it is asked:
---   * it is a non-negative integer (a `number`, not a string that looks like one),
---   * its kind byte names a real `ids::Input` variant (0..3),
---   * `BuyOffer`'s slot addresses a shop slot that structurally exists,
---   * every other kind carries slot 0 — the payload-free variants have no slot,
---     so a non-zero low byte is a fabricated code, not a "close enough" one.
---
--- Deliberately NOT checked here: affordability, whether the slot currently
--- holds an offer, or the `Clear` cooldown. Those are state-dependent, they are
--- already enforced authoritatively inside `Input.apply`, and re-deciding them
--- at submit time would answer for a tick that is not the tick the input lands
--- on. A legal-but-ineffective input is accepted and then ignored by the sim;
--- that is the correct outcome, and it is not the same thing as a malformed one.
function ArenaRunner.validateCode(code: any): (boolean, string?)
	if type(code) ~= "number" then
		return false, "code_not_a_number"
	end
	if code ~= code or code % 1 ~= 0 then
		return false, "code_not_an_integer"
	end
	if code < 0 then
		return false, "code_negative"
	end
	local kind = Input.kind(code)
	local slot = Input.slot(code)
	if kind > KIND_CLEAR then
		return false, "unknown_input_kind"
	end
	if kind == KIND_BUY then
		if slot >= OFFER_SLOTS then
			return false, "buy_slot_out_of_range"
		end
	elseif slot ~= 0 then
		return false, "slot_on_slotless_kind"
	end
	return true
end

-- =========================================================================
-- construction / run lifecycle
-- =========================================================================

export type Runner = typeof(setmetatable(
	{} :: {
		player_id: number,
		owner_id: number,
		is_bot: boolean,
		status: string,
		state: any,
		bot: any,
		challenge: any,
		queue: { number },
		queue_head: number,
		run_index: number,
		seed: Seed?,
		started_at: number,
		ended_at: number,
		tick_limit: number?,
		timeline: any,
		max_queued: number,
		accepted: number,
		rejected: number,
		last_result: RunResult?,
		accum: number,
	},
	ArenaRunner
))

--- A runner for one seat. It starts `idle`; the director calls `start` to open a
--- run. `opts`:
---   `player_id`   THE SIM'S player id, and it is a **u32 that the checksum
---                 hashes** (`state.rs`). A Roblox `UserId` must NOT be passed
---                 here: `UserId` is an i64 and live account ids already exceed
---                 `u32::MAX`, so feeding one in would silently truncate the
---                 value the digest is taken over. The director passes the SEAT
---                 (0..cohort-1), which also makes a run reproducible from
---                 `(master_seed, seat, run_index)` alone.
---   `owner_id`    the director's key for whoever occupies the seat (the Roblox
---                 `UserId`, or a synthetic bot id). Carried on results and never
---                 touched by the sim. Defaults to `player_id`.
---   `is_bot`      backfill seat: the runner drives itself via `Bot.decide`.
---   `tick_limit`  run tick at which a surviving run is `completed` (nil = only
---                 death ends it).
---   `timeline`    the `Content.TIMELINE.*` table to run (nil = Steam, per
---                 `PHASE2-CONTRACTS.md` P3). Carried on the state; nothing here
---                 hardcodes either schedule.
---   `challenge`   optional `Bot.Challenge` for a bot seat.
---   `max_queued`  input FIFO depth (defaults to `MAX_QUEUED_INPUTS`).
function ArenaRunner.new(opts): Runner
	local player_id = opts.player_id
	if type(player_id) ~= "number" or player_id % 1 ~= 0 or player_id < 0 or player_id > 4294967295 then
		error("ArenaRunner.new: player_id must be a u32 (pass the SEAT, not a Roblox UserId)", 2)
	end
	local owner_id = opts.owner_id
	if owner_id == nil then
		owner_id = player_id
	elseif type(owner_id) ~= "number" or owner_id % 1 ~= 0 or owner_id < 0 then
		error("ArenaRunner.new: owner_id must be a non-negative integer", 2)
	end
	return setmetatable({
		player_id = player_id,
		owner_id = owner_id,
		is_bot = opts.is_bot == true,
		status = ArenaRunner.Status.Idle,
		state = nil,
		bot = nil,
		challenge = opts.challenge,
		queue = {},
		queue_head = 1,
		run_index = 0,
		seed = nil,
		started_at = 0,
		ended_at = 0,
		tick_limit = opts.tick_limit,
		timeline = opts.timeline,
		max_queued = opts.max_queued or ArenaRunner.MAX_QUEUED_INPUTS,
		accepted = 0,
		rejected = 0,
		last_result = nil,
		accum = 0,
	}, ArenaRunner) :: any
end

--- Open a fresh run on `seed` (issued by the director — a runner never invents
--- its own seed; see `Director.issueSeed`). `now` is the director's server tick,
--- recorded so results can be placed in the rolling leaderboard window.
---
--- Instant re-entry (F2) is exactly this call: a dead runner is `start`ed again
--- and is simulating on the very next tick. Nothing is torn down or rebuilt
--- beyond the arena state itself.
function ArenaRunner.start(self: Runner, seed: Seed, run_index: number, now: number)
	local s = Arena.newState(seed, self.player_id)
	-- P3: `state.timeline` is the single source of truth and `nil` means Steam.
	-- Set through `Arena.setTimeline` (which accepts a timeline table or the
	-- names `"steam"`/`"roblox"`) and ONLY at tick 0 — the schedule is fixed at
	-- arena setup, because round boundaries and wave gates are derived from it
	-- every tick. Left untouched when the director chose nothing, so the default
	-- path stays byte-identical to what the R3 oracle runs.
	if self.timeline ~= nil then
		Arena.setTimeline(s, self.timeline)
	end
	self.state = s
	self.seed = seed
	self.run_index = run_index
	self.started_at = now
	self.ended_at = 0
	self.status = ArenaRunner.Status.Running
	self.last_result = nil
	self.accum = 0
	table.clear(self.queue)
	self.queue_head = 1
	if self.is_bot then
		self.bot = if self.challenge ~= nil then Bot.withChallenge(self.challenge) else Bot.new()
	end
end

--- Is this runner simulating a live run right now? This is the ONLY definition
--- of "alive" the director uses for Last Stand.
function ArenaRunner.isAlive(self: Runner): boolean
	return self.status == ArenaRunner.Status.Running
end

-- =========================================================================
-- input
-- =========================================================================

--- Queue one client-submitted input code. Returns `true`, or `false, reason`.
--- Every rejection path is a refusal, never a clamp:
---   `no_active_run`  nothing to apply it to,
---   `bot_seat`       a backfill seat is not client-drivable,
---   `input_flood`    the FIFO is full — the excess is dropped, not banked,
---   plus every structural reason from `validateCode`.
function ArenaRunner.submit(self: Runner, code: any): (boolean, string?)
	if self.status ~= ArenaRunner.Status.Running then
		self.rejected += 1
		return false, "no_active_run"
	end
	if self.is_bot then
		self.rejected += 1
		return false, "bot_seat"
	end
	local ok, why = ArenaRunner.validateCode(code)
	if not ok then
		self.rejected += 1
		return false, why
	end
	local queued = #self.queue - self.queue_head + 1
	if queued >= self.max_queued then
		self.rejected += 1
		return false, "input_flood"
	end
	self.queue[#self.queue + 1] = code
	self.accepted += 1
	return true
end

--- Pop the code to apply this tick: a queued client input first, else the bot's
--- decision on a backfill seat, else `Noop`.
local function nextInput(self: Runner): number
	local head = self.queue_head
	if self.queue[head] ~= nil then
		local code = self.queue[head]
		self.queue[head] = nil
		self.queue_head = head + 1
		if self.queue[self.queue_head] == nil then
			-- Drained: collapse so the array never grows without bound.
			table.clear(self.queue)
			self.queue_head = 1
		end
		return code
	end
	local bot = self.bot
	if bot ~= nil then
		-- The bot reads only the public state and draws no sim RNG (`Bot.luau`),
		-- so a backfilled seat cannot perturb anyone's determinism. The apply-time
		-- challenge guard is applied for the same reason the director would apply
		-- it to a human: the shop may have moved since the decision.
		return Bot.challengeFilter(bot.challenge, Bot.decide(bot, self.state), self.state)
	end
	return Input.NOOP
end

-- =========================================================================
-- stepping
-- =========================================================================

--- Build the immutable record of a finished run. The director scores it; the
--- runner only reports facts. `checksum` is carried so a disputed result can be
--- audited against a replay of `(seed, inputs)` later.
local function finish(self: Runner, reason: string, now: number): RunResult
	local s = self.state
	local ticks = if s.dead and s.death_tick ~= nil then s.death_tick + 1 else s.tick
	-- The seed in the canonical u64 wire form: 16 lowercase hex digits, NO `0x`
	-- prefix (`CONTRACTS.md` C4 — the same form `Arena.checksum` returns).
	-- `Rng.toHex` prefixes, which is a rendering helper, not the wire form.
	local hi, lo = Rng.seedParts(self.seed)
	local result: RunResult = {
		player_id = self.owner_id,
		seat = self.player_id,
		is_bot = self.is_bot,
		run_index = self.run_index,
		seed = string.format("%08x%08x", hi, lo),
		reason = reason,
		started_at = self.started_at,
		ended_at = now,
		ticks_survived = ticks,
		round_reached = if s.round == 4294967295 then 0 else s.round,
		damage_dealt = s.total_damage_dealt,
		gold_earned = s.total_gold_earned,
		weapons_owned = #s.weapons,
		weapons_bought = s.weapons_bought,
		economy_purchases = s.economy_purchases,
		checksum = Arena.checksum(s),
	}
	self.status = ArenaRunner.Status.Finished
	self.ended_at = now
	self.last_result = result
	table.clear(self.queue)
	self.queue_head = 1
	return result
end

--- Advance exactly one sim tick. Returns a `RunResult` on the tick the run ends,
--- otherwise `nil`. A runner that is not `running` is a no-op — the director does
--- not have to special-case dead seats in its loop.
---
--- `now` is the director's server tick (the authoritative clock); the run's own
--- tick lives on the state and the two are deliberately independent, because
--- runs start and restart at arbitrary server ticks (F2).
function ArenaRunner.step(self: Runner, now: number): RunResult?
	if self.status ~= ArenaRunner.Status.Running then
		return nil
	end
	local s = self.state
	Arena.step(s, nextInput(self))
	if s.dead then
		return finish(self, ArenaRunner.EndReason.Death, now)
	end
	local limit = self.tick_limit
	if limit ~= nil and s.tick >= limit then
		return finish(self, ArenaRunner.EndReason.Completed, now)
	end
	return nil
end

--- Step up to `n` ticks, stopping early if the run ends. Returns the result if
--- it did.
function ArenaRunner.advance(self: Runner, n: number, now: number): RunResult?
	for _ = 1, n do
		local result = ArenaRunner.step(self, now)
		if result ~= nil then
			return result
		end
	end
	return nil
end

--- End a live run without a result (seat reclaimed, player left). Returns the
--- abandoned result so the director can log it; it is deliberately NOT scored.
function ArenaRunner.abandon(self: Runner, now: number): RunResult?
	if self.status ~= ArenaRunner.Status.Running then
		return nil
	end
	return finish(self, ArenaRunner.EndReason.Abandoned, now)
end

--- Drop the state entirely (seat freed). Keeps the last result for reporting.
function ArenaRunner.reset(self: Runner)
	self.status = ArenaRunner.Status.Idle
	self.state = nil
	self.bot = nil
	table.clear(self.queue)
	self.queue_head = 1
end

-- =========================================================================
-- read-only exposure — everything the replication layer may send
-- =========================================================================

--- The current render view (`View.snapshot`). This is the payload `Wire.luau`
--- will pack; building it never mutates the sim. `nil` between runs.
---
--- F4 rival spectating needs no new path: the director simply calls this on
--- somebody else's runner. Same function, different arena.
function ArenaRunner.view(self: Runner)
	if self.state == nil then
		return nil
	end
	return View.snapshot(self.state)
end

--- The authoritative run clock, for the reliable low-rate channel.
function ArenaRunner.clock(self: Runner)
	local s = self.state
	if s == nil then
		return nil
	end
	-- Round length and boss tick are per-state LOOKUPS, not constants (P3): the
	-- active timeline is `state.timeline` and nothing here may hardcode either
	-- schedule. `Arena` owns the lookup; this file only reads it.
	local rt = Arena.roundTicks(s)
	local boss = Arena.bossTick(s)
	return {
		tick = s.tick,
		round = if s.round == 4294967295 then 0 else s.round,
		round_ticks = rt,
		ticks_into_round = s.tick % rt,
		ticks_to_next_round = rt - (s.tick % rt),
		boss_tick = boss,
		ticks_to_boss = if s.tick >= boss then 0 else boss - s.tick,
		tick_limit = self.tick_limit,
		dead = s.dead,
	}
end

--- The result of the most recently finished run, if any.
function ArenaRunner.result(self: Runner): RunResult?
	return self.last_result
end

--- `state_checksum` of the live run — the audit hook (`SIM-SPEC` §S6).
function ArenaRunner.checksum(self: Runner): string?
	if self.state == nil then
		return nil
	end
	return Arena.checksum(self.state)
end

-- =========================================================================
-- SCHEDULING SEAM — the only wall-clock-aware code in this file
-- =========================================================================
-- Everything above is a pure state machine over integer ticks. `pump` is the
-- adapter for a real frame loop: it converts elapsed seconds into a whole number
-- of ticks and calls `step` that many times. It is fenced off here so the
-- distinction stays obvious — the accumulator decides only HOW MANY ticks run,
-- and no value from it ever reaches sim math (`docs/05 §5.6`: no wall-clock in
-- anything feeding `state_checksum`).
--
-- The Roblox side (`RunService.Heartbeat`, or the `Actor`'s own loop under F6)
-- lives in a separate file and does nothing but call this.

--- Run the ticks that `dt` seconds have earned, up to `MAX_CATCHUP_TICKS`.
--- Returns the result if the run ended, plus the number of ticks stepped.
function ArenaRunner.pump(self: Runner, dt: number, now: number): (RunResult?, number)
	self.accum += dt
	local budget = ArenaRunner.MAX_CATCHUP_TICKS
	local stepped = 0
	local result: RunResult? = nil
	while self.accum >= TICK_DT and stepped < budget do
		self.accum -= TICK_DT
		stepped += 1
		result = ArenaRunner.step(self, now)
		if result ~= nil then
			break
		end
	end
	if self.accum > budget * TICK_DT then
		-- Fell far behind: drop the backlog rather than spiral. The server is
		-- authoritative, so slowing down is a visible-but-safe failure; unbounded
		-- catch-up is not.
		self.accum = 0
	end
	return result, stepped
end

return ArenaRunner
