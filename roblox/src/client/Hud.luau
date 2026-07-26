--!strict
--[[
	Hud.luau — the run read-outs: HP, gold, round, timer, arsenal, run stats.

	Same information the Godot front-end puts on a cell (`godot/match.gd`
	`_draw_cell`: HP bar, `R%d · %dg · %dw`, the log-compressed SCORE), promoted
	to a full-screen single-arena HUD because on Roblox you are looking at your
	own arena, not a grid of eight.

	WHAT IS PURE AND WHY IT MATTERS
	-------------------------------
	`Hud.model(view, opts)` turns a `RenderView` into a fully-formatted plain
	table — every string the HUD will ever show, already rendered. The Roblox
	shell then does nothing but copy `model.hp.text` into a `TextLabel`. That
	split is the point: the interesting part of a HUD is the arithmetic (round
	number from a tick count, "boss in 1:12", thousands separators, a damage
	figure that runs to eight digits), and all of it is asserted headlessly in
	`roblox/test/client_test.luau`.

	THE TIMELINE SEAM (flagged, not widened)
	----------------------------------------
	`RenderView` carries `tick` and `round` but NOT the timeline in force, so the
	client cannot derive a round clock or a boss countdown from the view alone —
	`round_ticks` is 900 on Steam and 600 on Roblox (`docs/10` F1,
	`PHASE2-CONTRACTS.md` P3). This module therefore defaults to
	`Content.TIMELINE.roblox`, which is correct for this fork, and takes
	`opts.timeline` for anything else. That default is an ASSUMPTION about state
	the client is not told; the clean fix is for the reliable channel (P4) to
	carry the arena's timeline once at run start. Flagged in the report.

	No Roblox global is referenced above the shell banner.
]]

local View = require("../shared/sim/View")
local Content = require("../shared/Content")
local Env = require("./RobloxEnv")

export type RenderView = View.RenderView

local Hud = {}

--- The fork's timeline (F1: 600-tick rounds, boss at 9000). Both timelines ship
--- as data; nothing here hardcodes either one's numbers.
Hud.DEFAULT_TIMELINE = Content.TIMELINE.roblox

--=============================================================================
-- Formatting primitives (pure)
--=============================================================================

--- `12345 -> "12,345"`. Negative and zero handled; fractions are floored, since
--- every quantity reaching the HUD is an integer by `View.luau`'s contract and a
--- stray decimal point in a gold counter is a bug worth making visible.
function Hud.formatInt(n: number): string
	local neg = n < 0
	local s = tostring(math.floor(math.abs(n)))
	local out = ""
	local len = #s
	for i = 1, len do
		out ..= s:sub(i, i)
		local remaining = len - i
		if remaining > 0 and remaining % 3 == 0 then
			out ..= ","
		end
	end
	if neg then
		return "-" .. out
	end
	return out
end

--- Compact magnitude for figures that run to eight digits (`total_damage_dealt`
--- reaches the tens of millions in a full run). `1234 -> "1.2k"`,
--- `33500000 -> "33.5M"`. Below 1000 it is exact, so early-run numbers stay
--- readable rather than collapsing to "0.1k".
function Hud.formatCompact(n: number): string
	local neg = n < 0
	local v = math.abs(n)
	local sign = if neg then "-" else ""
	if v < 1000 then
		return sign .. tostring(math.floor(v))
	end
	local units = { { 1e9, "B" }, { 1e6, "M" }, { 1e3, "k" } }
	for _, u in units do
		local div = u[1] :: number
		if v >= div then
			local scaled = v / div
			-- One decimal below 100, none above, so the field never widens past
			-- five characters and the layout cannot jump.
			if scaled < 100 then
				return string.format("%s%.1f%s", sign, math.floor(scaled * 10) / 10, u[2] :: string)
			end
			return string.format("%s%d%s", sign, math.floor(scaled), u[2] :: string)
		end
	end
	return sign .. tostring(math.floor(v))
end

--- Ticks to `M:SS` (or `H:MM:SS` past an hour). `tickHz` defaults to the
--- catalog's 30, which both timelines share.
function Hud.formatClock(ticks: number, tickHz: number?): string
	local hz = tickHz or Hud.DEFAULT_TIMELINE.tick_hz
	local total = math.max(math.floor(ticks / hz), 0)
	local s = total % 60
	local m = math.floor(total / 60) % 60
	local h = math.floor(total / 3600)
	if h > 0 then
		return string.format("%d:%02d:%02d", h, m, s)
	end
	return string.format("%d:%02d", m, s)
end

--- Godot's per-player damage tracker: `round(30 * log10(1 + damage))`. Log
--- compression so the number climbs steadily all run instead of running into the
--- millions (`godot/match.gd::_score`). Ported verbatim, including the `< 1`
--- short-circuit, so the two front-ends show the same score for the same run.
function Hud.score(damage: number): number
	if damage < 1 then
		return 0
	end
	return math.round(30 * math.log(1 + damage, 10))
end

--=============================================================================
-- The model (pure)
--=============================================================================

export type HudTimeline = { tick_hz: number, round_ticks: number, boss_spawn_tick: number }

export type HudModel = {
	dead: boolean,
	tick: number,
	hp: { current: number, max: number, frac: number, text: string },
	revives: { count: number, text: string },
	gold: { value: number, text: string },
	income: { per_tick: number, per_second: number, text: string },
	round: { index: number, text: string },
	rerolls: { remaining: number, cost: number, affordable: boolean, text: string },
	timer: {
		elapsed_ticks: number,
		elapsed_text: string,
		round_remaining_ticks: number,
		round_remaining_text: string,
		boss_in_ticks: number,
		boss_in_text: string,
		boss_active: boolean,
	},
	arsenal: { { name: string, count: number, text: string } },
	stats: {
		damage: number,
		damage_text: string,
		gold_earned: number,
		gold_earned_text: string,
		weapons_bought: number,
		economy_purchases: number,
		attack_mask: number,
		score: number,
		score_text: string,
	},
	population: { enemies: number, projectiles: number, minions: number, text: string },
}

--[[
	Build every string the HUD shows, from one `RenderView`.

	`opts.timeline` overrides the F1 default (see the header seam note). Nothing
	here mutates `view`: the client is a pure observer (P5), and a HUD that could
	write to the view would be the first step toward a client that thinks it owns
	state the server owns.
]]
function Hud.model(view: RenderView, opts: { timeline: HudTimeline? }?): HudModel
	local o: { timeline: HudTimeline? } = opts or {}
	local tl: HudTimeline = o.timeline or Hud.DEFAULT_TIMELINE
	local hz = tl.tick_hz
	local roundTicks = math.max(tl.round_ticks, 1)

	local tank = view.tank
	local maxHp = math.max(tank.max_hp, 1)
	local hp = math.max(tank.hp, 0)

	local eco = view.economy
	local perTick = eco.income_per_tick

	-- Ticks left in the CURRENT round. `view.round` is the sim's own counter, so
	-- the remainder is computed off the tick rather than off the round number —
	-- they agree, and using the tick keeps this correct if a run ever starts at a
	-- non-zero tick (spectating a run already in progress, F4).
	local intoRound = view.tick % roundTicks
	local roundRemaining = roundTicks - intoRound

	local bossIn = tl.boss_spawn_tick - view.tick
	local bossActive = bossIn <= 0

	local arsenal = {}
	for i = 1, #view.arsenal do
		local a = view.arsenal[i]
		arsenal[i] = {
			name = a.name,
			count = a.count,
			text = if a.count > 1 then string.format("%s ×%d", a.name, a.count) else a.name,
		}
	end

	local stats = view.stats
	local score = Hud.score(stats.damage_dealt)

	return {
		dead = view.dead,
		tick = view.tick,
		hp = {
			current = hp,
			max = tank.max_hp,
			frac = math.min(hp / maxHp, 1),
			text = string.format("%s / %s", Hud.formatInt(hp), Hud.formatInt(tank.max_hp)),
		},
		revives = {
			count = tank.revives,
			text = if tank.revives > 0 then string.format("REVIVES %d", tank.revives) else "",
		},
		gold = { value = eco.gold, text = Hud.formatInt(eco.gold) .. "g" },
		income = {
			per_tick = perTick,
			per_second = perTick * hz,
			text = string.format("+%sg/s", Hud.formatInt(perTick * hz)),
		},
		round = { index = view.round, text = string.format("ROUND %d", view.round) },
		rerolls = {
			remaining = eco.rerolls_remaining,
			cost = eco.reroll_cost,
			affordable = eco.reroll_cost <= eco.gold,
			text = string.format("REROLL %d left · %sg", eco.rerolls_remaining, Hud.formatInt(eco.reroll_cost)),
		},
		timer = {
			elapsed_ticks = view.tick,
			elapsed_text = Hud.formatClock(view.tick, hz),
			round_remaining_ticks = roundRemaining,
			round_remaining_text = Hud.formatClock(roundRemaining, hz),
			boss_in_ticks = math.max(bossIn, 0),
			boss_in_text = if bossActive then "BOSS" else "BOSS IN " .. Hud.formatClock(bossIn, hz),
			boss_active = bossActive,
		},
		arsenal = arsenal,
		stats = {
			damage = stats.damage_dealt,
			damage_text = Hud.formatCompact(stats.damage_dealt),
			gold_earned = stats.gold_earned,
			gold_earned_text = Hud.formatCompact(stats.gold_earned),
			weapons_bought = stats.weapons_bought,
			economy_purchases = stats.economy_purchases,
			attack_mask = stats.bought_attack_mask,
			score = score,
			score_text = string.format("SCORE %d", score),
		},
		population = {
			enemies = #view.enemies,
			projectiles = #view.projectiles,
			minions = #view.minions,
			text = string.format("%d enemies", #view.enemies),
		},
	}
end

--=============================================================================
-- Roblox shell
--=============================================================================

export type Handle = {
	update: (Handle, RenderView, { timeline: HudTimeline? }?) -> HudModel?,
	instanceCount: (Handle) -> number,
	lastModel: (Handle) -> HudModel?,
	destroy: (Handle) -> (),
}

--- Arsenal rows are pooled like the arena markers: a run can end with a dozen
--- distinct weapon names and the list churns every purchase, so rows are reused
--- rather than rebuilt.
Hud.ARSENAL_ROWS = 12

local ShellImpl = {}
ShellImpl.__index = ShellImpl

local TEXT = { r = 0.88, g = 0.91, b = 0.96 }
local DIM = { r = 0.58, g = 0.60, b = 0.68 }
local COIN = { r = 0.95, g = 0.78, b = 0.36 }
local DANGER = { r = 0.88, g = 0.42, b = 0.38 }

local function label(parent: any, name: string, pos: any, size: number, color: any, align: string): any
	local l = Env.Instance.new("TextLabel")
	l.Name = name
	l.BackgroundTransparency = 1
	l.Position = pos
	l.Size = Env.UDim2.fromOffset(360, math.floor(size * 1.4))
	l.Font = Env.Enum.Font.GothamMedium
	l.TextSize = size
	l.TextColor3 = Env.color(color)
	l.TextXAlignment = Env.Enum.TextXAlignment[align]
	l.Text = ""
	l.Parent = parent
	return l
end

--- Build the HUD under `parent`. Returns `nil` off-Roblox.
function Hud.new(parent: any): Handle?
	if not Env.available then
		return nil
	end

	local root = Env.Instance.new("Frame")
	root.Name = "Hud"
	root.BackgroundTransparency = 1
	root.Size = Env.UDim2.fromScale(1, 1)
	root.Parent = parent

	local hpBack = Env.Instance.new("Frame")
	hpBack.Name = "HpBack"
	hpBack.BorderSizePixel = 0
	hpBack.BackgroundColor3 = Env.color({ r = 0, g = 0, b = 0 })
	hpBack.BackgroundTransparency = 0.4
	hpBack.Position = Env.UDim2.fromOffset(16, 16)
	hpBack.Size = Env.UDim2.new(0.35, 0, 0, 14)
	hpBack.Parent = root

	local hpFill = Env.Instance.new("Frame")
	hpFill.Name = "HpFill"
	hpFill.BorderSizePixel = 0
	hpFill.BackgroundColor3 = Env.color(DANGER)
	hpFill.Position = Env.UDim2.fromScale(0, 0)
	hpFill.Size = Env.UDim2.fromScale(1, 1)
	hpFill.Parent = hpBack

	local rows = {
		hp = label(root, "Hp", Env.UDim2.fromOffset(16, 34), 16, TEXT, "Left"),
		gold = label(root, "Gold", Env.UDim2.fromOffset(16, 58), 20, COIN, "Left"),
		income = label(root, "Income", Env.UDim2.fromOffset(16, 84), 14, DIM, "Left"),
		rerolls = label(root, "Rerolls", Env.UDim2.fromOffset(16, 104), 14, DIM, "Left"),
		round = label(root, "Round", Env.UDim2.new(0.5, -180, 0, 16), 22, TEXT, "Center"),
		clock = label(root, "Clock", Env.UDim2.new(0.5, -180, 0, 44), 16, DIM, "Center"),
		boss = label(root, "Boss", Env.UDim2.new(0.5, -180, 0, 66), 14, DANGER, "Center"),
		score = label(root, "Score", Env.UDim2.new(1, -376, 0, 16), 18, COIN, "Right"),
		damage = label(root, "Damage", Env.UDim2.new(1, -376, 0, 40), 14, DIM, "Right"),
		earned = label(root, "Earned", Env.UDim2.new(1, -376, 0, 60), 14, DIM, "Right"),
		population = label(root, "Population", Env.UDim2.new(1, -376, 0, 80), 14, DIM, "Right"),
		revives = label(root, "Revives", Env.UDim2.fromOffset(16, 124), 14, COIN, "Left"),
	}

	local arsenalHost = Env.Instance.new("Frame")
	arsenalHost.Name = "Arsenal"
	arsenalHost.BackgroundTransparency = 1
	arsenalHost.Position = Env.UDim2.fromOffset(16, 152)
	arsenalHost.Size = Env.UDim2.fromOffset(240, Hud.ARSENAL_ROWS * 18)
	arsenalHost.Parent = root

	local arsenalRows = {}
	for i = 1, Hud.ARSENAL_ROWS do
		local l = label(arsenalHost, "Row" .. i, Env.UDim2.fromOffset(0, (i - 1) * 18), 14, TEXT, "Left")
		l.Visible = false
		arsenalRows[i] = l
	end

	local self = setmetatable({
		_root = root,
		_hpFill = hpFill,
		_rows = rows,
		_arsenal = arsenalRows,
		_model = nil,
	}, ShellImpl)
	return (self :: any) :: Handle
end

function ShellImpl.update(self: any, view: RenderView, opts: { timeline: HudTimeline? }?): HudModel
	local m = Hud.model(view, opts)
	self._model = m

	local r = self._rows
	r.hp.Text = m.hp.text
	r.gold.Text = m.gold.text
	r.income.Text = m.income.text
	r.rerolls.Text = m.rerolls.text
	r.round.Text = m.round.text
	r.clock.Text = string.format("%s  ·  round ends %s", m.timer.elapsed_text, m.timer.round_remaining_text)
	r.boss.Text = m.timer.boss_in_text
	r.score.Text = m.stats.score_text
	r.damage.Text = "DMG " .. m.stats.damage_text
	r.earned.Text = "EARNED " .. m.stats.gold_earned_text
	r.population.Text = m.population.text
	r.revives.Text = m.revives.text
	r.revives.Visible = m.revives.text ~= ""

	self._hpFill.Size = Env.UDim2.fromScale(m.hp.frac, 1)
	self._hpFill.BackgroundColor3 = Env.color(if m.dead then { r = 0.5, g = 0.5, b = 0.55 } else DANGER)

	local rowsN = math.min(#m.arsenal, Hud.ARSENAL_ROWS)
	for i = 1, rowsN do
		local l = self._arsenal[i]
		l.Text = m.arsenal[i].text
		l.Visible = true
	end
	for i = rowsN + 1, Hud.ARSENAL_ROWS do
		self._arsenal[i].Visible = false
	end

	return m
end

function ShellImpl.lastModel(self: any): HudModel?
	return self._model
end

function ShellImpl.instanceCount(self: any): number
	-- root + hp back/fill + 12 fixed labels + arsenal host + pooled rows.
	return 3 + 12 + 1 + #self._arsenal
end

function ShellImpl.destroy(self: any)
	self._root:Destroy()
	self._model = nil
end

return Hud
