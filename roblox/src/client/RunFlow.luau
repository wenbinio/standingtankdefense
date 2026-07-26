--!strict
--[[
	RunFlow.luau — the run loop, and the instant retry that `docs/10` F2 makes a
	core design decision rather than a convenience.

	F2, RESTATED SO IT IS NOT QUIETLY UNDONE
	----------------------------------------
	`docs/09 §2.3` found elimination to be a structural problem on Roblox: half
	the lobby is dead early, and dead players leave rather than spectate. F2's
	answer is to **stop making a run and a match the same object** — your run is
	solo and instantly restartable, and death → results → a NEW RUN takes about
	three seconds with approximately zero dead time. The competitive layer becomes
	a rolling leaderboard over run scores, not a bracket.

	So this state machine has **no waiting-on-other-players state at all**. There
	is no "eliminated", no "spectating until the round ends", no lobby to return
	to. From the moment the tank dies, the ONLY thing between the player and a
	fresh arena is a short results beat and a server round-trip — and the retry is
	requested automatically, so a player who does nothing still gets a new run.
	`roblox/test/client_test.luau` asserts exactly that: with zero player input,
	a retry request is emitted within `RunFlow.MAX_DEAD_TIME` of the death.

	The sharded architecture is what makes this free: your arena never depended on
	anyone else's, so restarting it is a local act (`docs/10` F2 — "this requires
	no simulation change at all").

	PHASES

	    booting  ──first view──►  running
	    running  ──view.dead───►  result      (result captured, countdown starts)
	    result   ──auto / press─►  retrying    (emits `request_retry`)
	    retrying ──fresh view──►  running
	    retrying ──timeout─────►  retrying    (re-emits `request_retry`, attempt+1)

	AUTHORITY: this module decides only WHEN to ask for a new run. It never starts
	one. The server owns run lifecycle, seeds and placement (P5); `request_retry`
	is a request, and the flow stays in `retrying` until the server actually sends
	a live arena back.

	PURITY: no Roblox global, no wall clock — every entry point takes `now` in
	seconds. That is what lets the whole machine be driven from a test in
	microseconds instead of in real time.
]]

local View = require("../shared/sim/View")
local Hud = require("./Hud")
local Env = require("./RobloxEnv")

export type RenderView = View.RenderView

local RunFlow = {}

--- F2's budget: death → new run in about three seconds. This is the number the
--- design commits to, and the test asserts the machine cannot exceed it without
--- player input.
RunFlow.MAX_DEAD_TIME = 3.0

--- When the automatic retry request goes out. Deliberately shorter than
--- `MAX_DEAD_TIME`: the remaining ~0.6 s is the server's round-trip budget for
--- standing a fresh arena up, so the *run* is live by three seconds, not just
--- the request.
RunFlow.DEFAULT_AUTO_RETRY = 2.4

--- If the server has not produced a live arena this long after a request, ask
--- again. The request carries no state, so a duplicate is harmless — and a
--- dropped retry that stranded a player on a results screen forever would be
--- exactly the dead time F2 exists to remove.
RunFlow.DEFAULT_RETRY_TIMEOUT = 2.0

export type Phase = "booting" | "running" | "result" | "retrying"

export type RunResult = {
	round: number,
	ticks: number,
	time_text: string,
	damage: number,
	damage_text: string,
	gold_earned: number,
	gold_earned_text: string,
	weapons_bought: number,
	economy_purchases: number,
	score: number,
	score_text: string,
}

--- `kind` is one of `"run_started"`, `"run_ended"`, `"request_retry"`.
--- `result` is set on `run_ended`; `attempt` (1-based) on `request_retry`.
export type Event = { kind: string, result: RunResult?, attempt: number? }

export type Options = {
	autoRetrySeconds: number?,
	retryTimeoutSeconds: number?,
	tickHz: number?,
}

export type Flow = {
	phase: (Flow) -> Phase,
	observe: (Flow, RenderView, number) -> { Event },
	update: (Flow, number) -> { Event },
	retryNow: (Flow, number) -> { Event },
	result: (Flow) -> RunResult?,
	--- Seconds until the automatic retry fires; 0 outside `result`.
	countdown: (Flow, number) -> number,
	--- Seconds since the tank died; 0 while alive.
	deadFor: (Flow, number) -> number,
	--- Number of `request_retry` events emitted for the CURRENT death.
	attempts: (Flow) -> number,
	runs: (Flow) -> number,
	reset: (Flow) -> (),
}

local Impl = {}
Impl.__index = Impl

--- Snapshot the finished run. Formatting reuses `Hud`'s pure helpers so the
--- results card and the live HUD can never disagree about what a score is.
local function captureResult(view: RenderView, tickHz: number): RunResult
	local s = view.stats
	local score = Hud.score(s.damage_dealt)
	return {
		round = view.round,
		ticks = view.tick,
		time_text = Hud.formatClock(view.tick, tickHz),
		damage = s.damage_dealt,
		damage_text = Hud.formatCompact(s.damage_dealt),
		gold_earned = s.gold_earned,
		gold_earned_text = Hud.formatCompact(s.gold_earned),
		weapons_bought = s.weapons_bought,
		economy_purchases = s.economy_purchases,
		score = score,
		score_text = string.format("SCORE %d", score),
	}
end

function RunFlow.new(opts: Options?): Flow
	local o: Options = opts or {}
	local self = setmetatable({
		_auto = o.autoRetrySeconds or RunFlow.DEFAULT_AUTO_RETRY,
		_timeout = o.retryTimeoutSeconds or RunFlow.DEFAULT_RETRY_TIMEOUT,
		_tickHz = o.tickHz or Hud.DEFAULT_TIMELINE.tick_hz,
		_phase = "booting" :: Phase,
		_result = nil :: RunResult?,
		_deadAt = 0,
		_requestedAt = 0,
		_attempts = 0,
		_runs = 0,
		_lastTick = -1,
	}, Impl)
	return (self :: any) :: Flow
end

function Impl.phase(self: any): Phase
	return self._phase
end

function Impl.result(self: any): RunResult?
	return self._result
end

function Impl.attempts(self: any): number
	return self._attempts
end

function Impl.runs(self: any): number
	return self._runs
end

function Impl.reset(self: any)
	self._phase = "booting"
	self._result = nil
	self._deadAt = 0
	self._requestedAt = 0
	self._attempts = 0
	self._lastTick = -1
end

local function startRun(self: any, view: RenderView, out: { Event })
	self._phase = "running"
	self._result = nil
	self._attempts = 0
	self._deadAt = 0
	self._requestedAt = 0
	self._runs += 1
	self._lastTick = view.tick
	out[#out + 1] = { kind = "run_started" }
end

local function emitRetry(self: any, now: number, out: { Event })
	self._phase = "retrying"
	self._attempts += 1
	self._requestedAt = now
	out[#out + 1] = { kind = "request_retry", attempt = self._attempts }
end

--[[
	Feed the newest `RenderView`. Returns the events this observation produced,
	oldest first.

	NEW-RUN DETECTION. The server owns run lifecycle and there is no `run_id` on
	`RenderView`, so a fresh run is recognised from the view itself:

	  * a live arena (`dead == false`) while we are showing results, or
	  * a tick that went BACKWARDS, which only a new `ArenaState` can do.

	Both are needed. A dead arena keeps ticking (`Arena.step` increments `tick`
	and returns early when `dead`), so "dead == false" alone is the normal signal;
	the tick regression catches the case where a replacement arena arrives already
	dead — impossible today, but a silent stall if it ever happened. Flagged in
	the report: a `run_id` (or a run epoch) on the reliable channel would make
	this exact rather than inferred.
]]
function Impl.observe(self: any, view: RenderView, now: number): { Event }
	local out: { Event } = {}
	local phase: Phase = self._phase

	if phase == "booting" then
		if view.dead then
			-- First contact with an already-dead arena: go straight to results so
			-- the player is never parked on a blank screen.
			self._lastTick = view.tick
			self._phase = "running"
			-- fall through to the death branch below
			phase = "running"
		else
			startRun(self, view, out)
			return out
		end
	end

	local regressed = view.tick < self._lastTick
	self._lastTick = view.tick

	if phase == "result" or phase == "retrying" then
		if not view.dead or regressed then
			startRun(self, view, out)
		end
		return out
	end

	-- phase == "running"
	if regressed and not view.dead then
		-- The server replaced the arena without us observing a death (a director
		-- restart, or a spectate target switch, F4). Treat it as a new run.
		startRun(self, view, out)
		return out
	end

	if view.dead then
		self._phase = "result"
		self._deadAt = now
		self._attempts = 0
		self._result = captureResult(view, self._tickHz)
		out[#out + 1] = { kind = "run_ended", result = self._result }
	end
	return out
end

--- Advance timers. Call every frame; `now` is seconds on the same clock as
--- `observe`. This is the ONLY thing that turns a death into a retry when the
--- player does nothing at all — which is the F2 guarantee.
function Impl.update(self: any, now: number): { Event }
	local out: { Event } = {}
	if self._phase == "result" then
		if now - self._deadAt >= self._auto then
			emitRetry(self, now, out)
		end
	elseif self._phase == "retrying" then
		if now - self._requestedAt >= self._timeout then
			emitRetry(self, now, out)
		end
	end
	return out
end

--- The player asked for a new run immediately (pressed RETRY, or tapped the
--- results card). Valid only while results are showing; ignored otherwise, so a
--- mashed button cannot spam the server with requests during `running`.
function Impl.retryNow(self: any, now: number): { Event }
	local out: { Event } = {}
	if self._phase == "result" then
		emitRetry(self, now, out)
	end
	return out
end

function Impl.countdown(self: any, now: number): number
	if self._phase ~= "result" then
		return 0
	end
	return math.max(self._auto - (now - self._deadAt), 0)
end

function Impl.deadFor(self: any, now: number): number
	if self._phase == "running" or self._phase == "booting" then
		return 0
	end
	return math.max(now - self._deadAt, 0)
end

--=============================================================================
-- Roblox shell — the results card
--=============================================================================

export type Screen = {
	--- Show/hide and refresh from the flow. Returns true when visible.
	update: (Screen, Flow, number) -> boolean,
	instanceCount: (Screen) -> number,
	destroy: (Screen) -> (),
}

local ScreenImpl = {}
ScreenImpl.__index = ScreenImpl

local TEXT = { r = 0.92, g = 0.94, b = 0.98 }
local DIM = { r = 0.62, g = 0.64, b = 0.72 }
local ACCENT = { r = 0.95, g = 0.78, b = 0.36 }

local function line(parent: any, name: string, y: number, size: number, color: any): any
	local l = Env.Instance.new("TextLabel")
	l.Name = name
	l.BackgroundTransparency = 1
	l.Position = Env.UDim2.new(0, 0, 0, y)
	l.Size = Env.UDim2.new(1, 0, 0, math.floor(size * 1.4))
	l.Font = Env.Enum.Font.GothamMedium
	l.TextSize = size
	l.TextColor3 = Env.color(color)
	l.Text = ""
	l.Parent = parent
	return l
end

--[[
	The results card: what the run scored, and a RETRY that fires immediately.
	Deliberately small — F2's point is that this screen is a beat, not a
	destination, so there is nothing here to browse and nothing to wait for.

	`onRetry` is invoked when the player presses RETRY; the caller feeds that to
	`flow:retryNow(now)` and sends the resulting request. Returns `nil`
	off-Roblox.
]]
function RunFlow.newScreen(parent: any, onRetry: () -> ()): Screen?
	if not Env.available then
		return nil
	end

	local root = Env.Instance.new("Frame")
	root.Name = "RunResult"
	root.BorderSizePixel = 0
	root.AnchorPoint = Env.Vector2.new(0.5, 0.5)
	root.Position = Env.UDim2.fromScale(0.5, 0.5)
	root.Size = Env.UDim2.fromOffset(360, 268)
	root.BackgroundColor3 = Env.color({ r = 0.07, g = 0.075, b = 0.10 })
	root.BackgroundTransparency = 0.08
	root.Visible = false
	root.Parent = parent

	local pad = Env.Instance.new("UIPadding")
	pad.PaddingLeft = Env.UDim.new(0, 20)
	pad.PaddingRight = Env.UDim.new(0, 20)
	pad.PaddingTop = Env.UDim.new(0, 18)
	pad.Parent = root

	local rows = {
		title = line(root, "Title", 0, 26, TEXT),
		score = line(root, "Score", 38, 30, ACCENT),
		round = line(root, "Round", 78, 16, DIM),
		time = line(root, "Time", 100, 16, DIM),
		damage = line(root, "Damage", 122, 16, DIM),
		gold = line(root, "Gold", 144, 16, DIM),
		bought = line(root, "Bought", 166, 16, DIM),
	}
	rows.title.Text = "RUN OVER"

	local btn = Env.Instance.new("TextButton")
	btn.Name = "Retry"
	btn.BorderSizePixel = 0
	btn.Position = Env.UDim2.new(0, 0, 0, 200)
	btn.Size = Env.UDim2.new(1, 0, 0, 44)
	btn.BackgroundColor3 = Env.color({ r = 0.16, g = 0.42, b = 0.22 })
	btn.Font = Env.Enum.Font.GothamBold
	btn.TextSize = 18
	btn.TextColor3 = Env.color({ r = 0.72, g = 0.98, b = 0.78 })
	btn.Text = "RETRY"
	btn.Parent = root
	btn.Activated:Connect(onRetry)

	local self = setmetatable({ _root = root, _rows = rows, _btn = btn }, ScreenImpl)
	return (self :: any) :: Screen
end

function ScreenImpl.update(self: any, flow: Flow, now: number): boolean
	local phase = flow:phase()
	local visible = phase == "result" or phase == "retrying"
	self._root.Visible = visible
	if not visible then
		return false
	end

	local r = flow:result()
	local rows = self._rows
	if r ~= nil then
		rows.score.Text = r.score_text
		rows.round.Text = string.format("Reached round %d", r.round)
		rows.time.Text = string.format("Survived %s", r.time_text)
		rows.damage.Text = string.format("Damage %s", r.damage_text)
		rows.gold.Text = string.format("Gold earned %s", r.gold_earned_text)
		rows.bought.Text = string.format("%d weapons · %d economy", r.weapons_bought, r.economy_purchases)
	end

	if phase == "retrying" then
		self._btn.Text = "STARTING…"
	else
		self._btn.Text = string.format("RETRY  (%.0f)", math.ceil(flow:countdown(now)))
	end
	return true
end

function ScreenImpl.instanceCount(self: any): number
	-- root + padding + 7 labels + button.
	return 10
end

function ScreenImpl.destroy(self: any)
	self._root:Destroy()
end

return RunFlow
