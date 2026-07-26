--!strict
--[[
	Render.luau — draws ONE arena from a `RenderView`.

	This is a port of the Godot front-end's `_draw_cell` (`godot/match.gd`), not a
	redesign: same 2D top-down presentation, same world→screen mapping constants,
	same marker sizes, same layer order, same HP-bar-and-label chrome. What
	changes is the *cost model*, and that change is the whole point of the file.

	THE COST MODEL (why this file looks the way it does)
	----------------------------------------------------
	Godot repaints an immediate-mode canvas every frame: an enemy costs a
	`draw_texture_rect` call and nothing else. Roblox has no immediate-mode 2D —
	every visible thing is a retained `Instance` — and the naive translation
	("one `ImageLabel` per enemy, created on spawn, destroyed on death") is what
	kills Roblox performance. At the measured peak (~67 enemies / ~84
	projectiles, `docs/09 §1.6`) that is ~150 instance create/destroy pairs per
	*second*, each one a property-replication event and a GC candidate.

	So: **no `Instance` per enemy.** Instead there are fixed POOLS of `Frame`s,
	one pool per entity class, grown once to the high-water mark and never shrunk.
	A frame's work is: assign `Position`/`Size`/`BackgroundColor3` on the first N
	pooled frames and set `Visible = false` on the rest. Nothing is created, and
	nothing is destroyed, after warm-up. `handle:instanceCount()` reports the pool
	size so a soak test can assert it plateaus.

	INTERPOLATION (P4)
	------------------
	Views arrive at **10–15 Hz** (`PHASE2-CONTRACTS.md` P4) while the client
	renders at 60+. Drawing the latest snapshot directly gives visibly stepped
	motion. `Render.newInterp()` therefore buffers the last two snapshots and
	samples at `now - delay`, where `delay` is one send interval — the standard
	snapshot-interpolation trade of a fixed ~80 ms of latency for smooth motion.
	Since the client is a pure observer (F6: the server simulates, clients only
	render), that latency costs nothing competitively — there is no local
	prediction to be wrong about.

	Enemies interpolate by `id`, which `RenderView` gives us and which is stable
	across snapshots. Projectiles and minions carry NO id (see `View.luau`:
	`RenderPoint` is `{x, y}`), so they are matched by array index with a jump
	guard: if an index-matched pair moved further than `SNAP_DIST` in one
	interval, the pair is assumed to be two different entities that happened to
	land on the same index, and the current position is used unsmoothed. This is
	a real limitation of the view contract and is flagged as such rather than
	papered over — a `RenderPoint.id` would remove the guess entirely.

	PURE CORE / SHELL SPLIT
	-----------------------
	Everything above the `-- Roblox shell` banner is pure Luau: no Roblox global,
	no `Instance`, no wall clock (timestamps are arguments). `Render.newInterp`
	and `Render.layout` are directly testable, and `Render.layout` returns a plain
	`DrawPlan` table of pixel-space rectangles that the shell mechanically applies
	to its pools. Reading the plan is how you check the drawing without a host.
]]

local View = require("../shared/sim/View")
local Env = require("./RobloxEnv")

export type RenderView = View.RenderView

local Render = {}

--=============================================================================
-- Palette (pure data — plain 0..1 floats, never a `Color3`)
--=============================================================================

export type Rgb = { r: number, g: number, b: number }

local function rgb(r: number, g: number, b: number): Rgb
	return { r = r, g = g, b = b }
end

--- Colours mirroring the Godot front-end's grimdark cell chrome. The shell
--- converts these to `Color3` at the last moment (`RobloxEnv.color`).
Render.PALETTE = table.freeze({
	ground = rgb(0.055, 0.06, 0.075),
	ring = rgb(0.20, 0.22, 0.30),
	tank = rgb(0.72, 0.80, 0.95),
	tank_dead = rgb(0.34, 0.34, 0.38),
	projectile = rgb(1.00, 0.90, 0.55),
	minion = rgb(0.45, 0.85, 0.70),
	hp_fill = rgb(0.78, 0.26, 0.26),
	hp_back = rgb(0.00, 0.00, 0.00),
	hp_dead = rgb(0.50, 0.50, 0.55),
	border = rgb(0.28, 0.30, 0.38),
	accent = rgb(0.95, 0.78, 0.36),
	dead_veil = rgb(0.00, 0.00, 0.00),
})

--- Per-enemy-kind marker colours, indexed by `RenderEnemy.kind + 1` (content ids
--- are 0-based, `SIM-SPEC` §S3). Twelve entries for the twelve shipped enemies;
--- an out-of-range kind falls back to the last entry rather than erroring, so a
--- catalog addition degrades to "drawn in grey" instead of a crash mid-run.
Render.ENEMY_COLORS = table.freeze({
	rgb(0.78, 0.42, 0.36), -- 0  Squeakzilla
	rgb(0.86, 0.55, 0.30), -- 1  Fanged Death
	rgb(0.95, 0.30, 0.30), -- 2  The Hippocrate (boss)
	rgb(0.88, 0.80, 0.42), -- 3  Doomduck
	rgb(0.80, 0.50, 0.55), -- 4  Bacon
	rgb(0.55, 0.72, 0.44), -- 5  Honk
	rgb(0.62, 0.60, 0.68), -- 6  Bonk
	rgb(0.45, 0.72, 0.60), -- 7  Nope Rope
	rgb(0.50, 0.75, 0.45), -- 8  Croak
	rgb(0.92, 0.48, 0.28), -- 9  Spicy
	rgb(0.55, 0.78, 0.92), -- 10 Popsicle
	rgb(0.60, 0.60, 0.62), -- 11 Dodo
})

--- Minion marker colours by `RenderMinion.kind + 1` (larvae, spores).
Render.MINION_COLORS = table.freeze({
	rgb(0.55, 0.88, 0.60),
	rgb(0.45, 0.70, 0.95),
})

--=============================================================================
-- World → screen mapping
--=============================================================================

--[[
	These three constants come STRAIGHT from `godot/match.gd` and are deliberately
	not re-derived:

	    scl    = min(w, h) * 0.42 / 1700          (match.gd `_draw_cell`)
	    ring Ø = 2 * 1500 * scl / 0.90            (match.gd `_blit(ring, …)`)

	1700 is the reference world radius the Godot cell was tuned against; the
	widest spawn ring in `content.json` sits at 1800 world units, so at this scale
	the outermost spawn lands just outside 0.42 of the half-extent and stays on
	screen with the marker's own radius to spare.
]]
Render.WORLD_REFERENCE_RADIUS = 1700
Render.FIT = 0.42
Render.RING_WORLD_RADIUS = 1500
Render.RING_FUDGE = 0.90

--- Base marker diameters in PIXELS at `uiScale == 1`, from `match.gd`.
Render.SIZE_TANK = 40
Render.SIZE_BOSS = 56
Render.SIZE_HEAVY = 30
Render.SIZE_ENEMY = 20
Render.SIZE_MINION = 18
Render.SIZE_PROJECTILE = 7

--- The Godot cell that these sizes were tuned for was ~420 px on its short side
--- (`us = 1.0`), and its featured panel used `us = 1.55`. A full-screen Roblox
--- viewport is much larger, so marker sizes scale with the short side and clamp
--- so a phone screen stays legible and a 4K screen does not turn into blobs.
Render.UI_REFERENCE_SHORT_SIDE = 420
Render.UI_SCALE_MIN = 0.70
Render.UI_SCALE_MAX = 2.20

--- World-unit distance beyond which an index-matched projectile/minion pair is
--- treated as two DIFFERENT entities and snapped rather than lerped. The fastest
--- projectile crosses well under this in one 10 Hz interval.
Render.SNAP_DIST = 600

local function clamp(v: number, lo: number, hi: number): number
	if v < lo then
		return lo
	elseif v > hi then
		return hi
	end
	return v
end

local function lerp(a: number, b: number, t: number): number
	return a + (b - a) * t
end

--=============================================================================
-- Interpolator (pure)
--=============================================================================

export type Snapshot = { time: number, view: RenderView }

export type SampledEnemy = {
	id: number,
	x: number,
	y: number,
	kind: number,
	boss: boolean,
	hp: number,
	base_hp: number,
	name: string,
}

export type SampledPoint = { x: number, y: number }
export type SampledMinion = { x: number, y: number, kind: number }

export type Sampled = {
	--- The most recent snapshot verbatim. Discrete state (hp, gold, round, shop,
	--- `dead`) is read from HERE, never interpolated — interpolating a gold count
	--- or a round number would invent state the server never had.
	view: RenderView,
	--- Blend factor actually used, for diagnostics/tests.
	alpha: number,
	tank: SampledPoint,
	enemies: { SampledEnemy },
	projectiles: { SampledPoint },
	minions: { SampledMinion },
}

export type Interp = {
	delay: number,
	prev: Snapshot?,
	curr: Snapshot?,
	push: (Interp, RenderView, number) -> (),
	sample: (Interp, number) -> Sampled?,
	reset: (Interp) -> (),
}

local InterpImpl = {}
InterpImpl.__index = InterpImpl

--- Default render delay in seconds: one send interval at the slow end of P4's
--- 10–15 Hz band, so the interpolator always has a bracketing pair even when the
--- server is sending at 10 Hz.
Render.DEFAULT_DELAY = 1 / 10

--- A two-snapshot interpolation buffer. `delay` is the render lag in seconds.
function Render.newInterp(delay: number?): Interp
	local self = setmetatable({
		delay = delay or Render.DEFAULT_DELAY,
		prev = nil,
		curr = nil,
	}, InterpImpl)
	return (self :: any) :: Interp
end

--- Accept a snapshot stamped at `time` (seconds, same clock as `sample`).
--- Out-of-order and duplicate arrivals are dropped: `UnreliableRemoteEvent` may
--- reorder, and a reordered snapshot would drag entities backwards.
function InterpImpl.push(self: Interp, view: RenderView, time: number)
	local curr = self.curr
	if curr ~= nil and time <= curr.time then
		return
	end
	self.prev = curr
	self.curr = { time = time, view = view }
end

--- Drop the buffer. Called on a run boundary so the first frame of a new run
--- does not lerp from the corpse of the last one.
function InterpImpl.reset(self: Interp)
	self.prev = nil
	self.curr = nil
end

--- Sample the buffer at wall time `now`. Returns `nil` until the first snapshot
--- has arrived.
function InterpImpl.sample(self: Interp, now: number): Sampled?
	local curr = self.curr
	if curr == nil then
		return nil
	end
	local prev = self.prev

	local alpha = 1.0
	if prev ~= nil then
		local span = curr.time - prev.time
		if span > 0 then
			alpha = clamp((now - self.delay - prev.time) / span, 0, 1)
		end
	end

	local cv = curr.view
	local pv = if prev ~= nil then prev.view else nil

	-- Tank is immobile by design (`docs/02 §2.2`), but it is interpolated anyway
	-- so that knockback/arena-offset variants need no change here.
	local tank: SampledPoint = { x = cv.tank.x, y = cv.tank.y }
	if pv ~= nil and alpha < 1 then
		tank.x = lerp(pv.tank.x, cv.tank.x, alpha)
		tank.y = lerp(pv.tank.y, cv.tank.y, alpha)
	end

	-- Enemies: matched by stable `id`.
	local prevById: { [number]: View.RenderEnemy } = {}
	if pv ~= nil and alpha < 1 then
		for i = 1, #pv.enemies do
			local pe = pv.enemies[i]
			prevById[pe.id] = pe
		end
	end
	local enemies: { SampledEnemy } = table.create(#cv.enemies) :: any
	for i = 1, #cv.enemies do
		local e = cv.enemies[i]
		local x, y = e.x, e.y
		local pe = prevById[e.id]
		if pe ~= nil then
			x = lerp(pe.x, e.x, alpha)
			y = lerp(pe.y, e.y, alpha)
		end
		enemies[i] = {
			id = e.id,
			x = x,
			y = y,
			kind = e.kind,
			boss = e.boss,
			hp = e.hp,
			base_hp = e.base_hp,
			name = e.name,
		}
	end

	-- Projectiles / minions: index-matched with the jump guard (see header).
	local projectiles: { SampledPoint } = table.create(#cv.projectiles) :: any
	for i = 1, #cv.projectiles do
		local c = cv.projectiles[i]
		local x, y = c.x, c.y
		if pv ~= nil and alpha < 1 then
			local p = pv.projectiles[i]
			if p ~= nil and math.abs(c.x - p.x) + math.abs(c.y - p.y) <= Render.SNAP_DIST then
				x = lerp(p.x, c.x, alpha)
				y = lerp(p.y, c.y, alpha)
			end
		end
		projectiles[i] = { x = x, y = y }
	end

	local minions: { SampledMinion } = table.create(#cv.minions) :: any
	for i = 1, #cv.minions do
		local c = cv.minions[i]
		local x, y = c.x, c.y
		if pv ~= nil and alpha < 1 then
			local p = pv.minions[i]
			if
				p ~= nil
				and p.kind == c.kind
				and math.abs(c.x - p.x) + math.abs(c.y - p.y) <= Render.SNAP_DIST
			then
				x = lerp(p.x, c.x, alpha)
				y = lerp(p.y, c.y, alpha)
			end
		end
		minions[i] = { x = x, y = y, kind = c.kind }
	end

	return {
		view = cv,
		alpha = alpha,
		tank = tank,
		enemies = enemies,
		projectiles = projectiles,
		minions = minions,
	}
end

--=============================================================================
-- Layout (pure)
--=============================================================================

export type Viewport = { x: number, y: number, w: number, h: number }

--- One pooled rectangle, in PIXELS, positioned by its CENTRE.
export type Marker = { x: number, y: number, size: number, color: Rgb }

export type DrawPlan = {
	viewport: Viewport,
	--- World→pixel scale and the pixel centre of the arena.
	scale: number,
	cx: number,
	cy: number,
	uiScale: number,
	dead: boolean,
	ring: Marker,
	tank: Marker,
	enemies: { Marker },
	projectiles: { Marker },
	minions: { Marker },
	--- Tank HP bar, in pixels: `{x, y, w, h, fill}` where `fill` is 0..1.
	hpBar: { x: number, y: number, w: number, h: number, fill: number },
	--- Counts BEFORE culling, so a test can tell "culled" from "dropped".
	counts: { enemies: number, projectiles: number, minions: number },
}

local function colorFor(list: { Rgb }, kind: number): Rgb
	local c = list[kind + 1]
	if c == nil then
		return list[#list]
	end
	return c
end

--- Marker diameter for an enemy. Godot keyed the "heavy" size off content id 6;
--- the boss size keys off the view's `boss` FLAG rather than off id 2, so a
--- second boss in the catalog would not need this file changed.
local function enemySize(e: SampledEnemy, us: number): number
	if e.boss then
		return Render.SIZE_BOSS * us
	elseif e.kind == 6 then
		return Render.SIZE_HEAVY * us
	end
	return Render.SIZE_ENEMY * us
end

--- Turn a sampled arena into a flat, pooled-draw-ready plan in pixel space.
--- Pure: no Roblox type crosses this boundary, and nothing here is allocated per
--- entity beyond the marker table the pool will read.
function Render.layout(s: Sampled, vp: Viewport): DrawPlan
	local short = math.min(vp.w, vp.h)
	local scale = short * Render.FIT / Render.WORLD_REFERENCE_RADIUS
	local us = clamp(short / Render.UI_REFERENCE_SHORT_SIDE, Render.UI_SCALE_MIN, Render.UI_SCALE_MAX)
	local cx = vp.x + vp.w * 0.5
	local cy = vp.y + vp.h * 0.5
	local dead = s.view.dead

	-- World +Y is UP; screen +Y is DOWN. Same flip as `match.gd`.
	local function toScreenX(wx: number): number
		return cx + wx * scale
	end
	local function toScreenY(wy: number): number
		return cy - wy * scale
	end

	-- Cull anything whose marker cannot touch the viewport. Off-screen markers
	-- still cost a pooled frame if drawn, so this is what keeps the pool small
	-- when the viewport is a small spectator panel (F4).
	local function visible(px: number, py: number, size: number): boolean
		local r = size * 0.5
		return px + r >= vp.x and px - r <= vp.x + vp.w and py + r >= vp.y and py - r <= vp.y + vp.h
	end

	local enemies: { Marker } = {}
	for i = 1, #s.enemies do
		local e = s.enemies[i]
		local px, py = toScreenX(e.x), toScreenY(e.y)
		local size = enemySize(e, us)
		if visible(px, py, size) then
			enemies[#enemies + 1] = { x = px, y = py, size = size, color = colorFor(Render.ENEMY_COLORS, e.kind) }
		end
	end

	local projSize = Render.SIZE_PROJECTILE * us
	local projectiles: { Marker } = {}
	for i = 1, #s.projectiles do
		local p = s.projectiles[i]
		local px, py = toScreenX(p.x), toScreenY(p.y)
		if visible(px, py, projSize) then
			projectiles[#projectiles + 1] =
				{ x = px, y = py, size = projSize, color = Render.PALETTE.projectile }
		end
	end

	local minionSize = Render.SIZE_MINION * us
	local minions: { Marker } = {}
	for i = 1, #s.minions do
		local m = s.minions[i]
		local px, py = toScreenX(m.x), toScreenY(m.y)
		if visible(px, py, minionSize) then
			minions[#minions + 1] =
				{ x = px, y = py, size = minionSize, color = colorFor(Render.MINION_COLORS, m.kind) }
		end
	end

	local tank = s.view.tank
	local maxHp = math.max(tank.max_hp, 1)
	local fill = clamp(math.max(tank.hp, 0) / maxHp, 0, 1)
	local barH = 7 * us
	local barW = math.max(vp.w - 16, 1)

	return {
		viewport = vp,
		scale = scale,
		cx = cx,
		cy = cy,
		uiScale = us,
		dead = dead,
		ring = {
			x = cx,
			y = cy,
			size = 2 * Render.RING_WORLD_RADIUS * scale / Render.RING_FUDGE,
			color = Render.PALETTE.ring,
		},
		tank = {
			x = toScreenX(s.tank.x),
			y = toScreenY(s.tank.y),
			size = Render.SIZE_TANK * us,
			color = if dead then Render.PALETTE.tank_dead else Render.PALETTE.tank,
		},
		enemies = enemies,
		projectiles = projectiles,
		minions = minions,
		hpBar = { x = vp.x + 8, y = vp.y + 8, w = barW, h = barH, fill = fill },
		counts = {
			enemies = #s.enemies,
			projectiles = #s.projectiles,
			minions = #s.minions,
		},
	}
end

--=============================================================================
-- Roblox shell — everything below this line needs a real host
--=============================================================================

export type Handle = {
	--- Feed a snapshot as it arrives off the wire.
	submit: (Handle, RenderView, number?) -> (),
	--- Draw the interpolated state for wall time `now` (defaults to `os.clock`).
	draw: (Handle, number?) -> (),
	--- Forget buffered snapshots (run boundary).
	reset: (Handle) -> (),
	--- Total pooled `Instance`s alive. Must plateau, never climb with enemy count.
	instanceCount: (Handle) -> number,
	--- The most recent `DrawPlan`, for diagnostics.
	lastPlan: (Handle) -> DrawPlan?,
	destroy: (Handle) -> (),
	interp: Interp,
}

--- Per-class pool ceilings. Sized from the measured peak (`docs/09 §1.6`:
--- ~67 enemies / ~84 projectiles) with headroom, so the pool stops growing well
--- before it could become the performance problem it exists to avoid.
Render.POOL_LIMITS = table.freeze({
	enemies = 160,
	projectiles = 200,
	minions = 64,
})

--[[
	The pooling arithmetic, factored out so the "no `Instance` per enemy" claim is
	machine-checked rather than asserted in a comment. Given how many markers a
	frame wants, how big the pool already is, and the pool ceiling, it returns:

	    want — how many pooled frames to make visible this frame
	    grow — how many NEW frames to create (0 once the pool has seen its peak)

	The properties the test pins: `grow` is 0 whenever demand is at or below the
	existing pool, the pool is monotonically non-decreasing, and neither `want`
	nor the resulting pool size ever exceeds `limit`. Together those are exactly
	"instances plateau at the high-water mark and never track live enemy count".
]]
function Render.poolFit(count: number, poolSize: number, limit: number): (number, number)
	local want = math.min(math.max(count, 0), limit)
	local grow = math.max(want - poolSize, 0)
	return want, grow
end

local ShellImpl = {}
ShellImpl.__index = ShellImpl

local function makeMarkerFrame(parent: any, zIndex: number, circular: boolean): any
	local f = Env.Instance.new("Frame")
	f.BorderSizePixel = 0
	f.AnchorPoint = Env.Vector2.new(0.5, 0.5)
	f.Visible = false
	f.ZIndex = zIndex
	if circular then
		local corner = Env.Instance.new("UICorner")
		corner.CornerRadius = Env.UDim.new(0.5, 0)
		corner.Parent = f
	end
	f.Parent = parent
	return f
end

--- Grow `pool` to at least `n` frames (capped by `limit`), then position the
--- first `#markers` of them and hide the rest. This is the ONLY place a pooled
--- `Instance` is ever created, and it is never destroyed until `destroy()`.
local function applyPool(pool: { any }, parent: any, markers: { Marker }, limit: number, zIndex: number)
	local want, grow = Render.poolFit(#markers, #pool, limit)
	for _ = 1, grow do
		pool[#pool + 1] = makeMarkerFrame(parent, zIndex, true)
	end
	for i = 1, want do
		local m = markers[i]
		local f = pool[i]
		f.Size = Env.UDim2.fromOffset(math.round(m.size), math.round(m.size))
		f.Position = Env.UDim2.fromOffset(math.round(m.x), math.round(m.y))
		f.BackgroundColor3 = Env.color(m.color)
		f.Visible = true
	end
	for i = want + 1, #pool do
		local f = pool[i]
		if f.Visible then
			f.Visible = false
		end
	end
end

--[[
	Build the arena presentation under `parent` (any GuiObject; typically a
	`ScreenGui` from `PlayerGui`). Returns `nil` off-Roblox rather than raising,
	so a headless test can require this module and exercise the pure core.

	`opts`:
	  delay  — interpolation lag in seconds (default `Render.DEFAULT_DELAY`)
	  z      — base ZIndex for the arena layers (default 0)
]]
function Render.new(parent: any, opts: { delay: number?, z: number? }?): Handle?
	if not Env.available then
		return nil
	end
	local o: { delay: number?, z: number? } = opts or {}
	local z = o.z or 0

	local root = Env.Instance.new("Frame")
	root.Name = "Arena"
	root.BorderSizePixel = 0
	root.Size = Env.UDim2.fromScale(1, 1)
	root.BackgroundColor3 = Env.color(Render.PALETTE.ground)
	root.ZIndex = z + 1
	root.ClipsDescendants = true
	root.Parent = parent

	local ring = makeMarkerFrame(root, z + 2, true)
	ring.BackgroundTransparency = 1
	local ringStroke = Env.Instance.new("UIStroke")
	ringStroke.Thickness = 2
	ringStroke.Color = Env.color(Render.PALETTE.ring)
	ringStroke.Parent = ring

	local minionHost = Env.Instance.new("Frame")
	minionHost.Name = "Minions"
	minionHost.BackgroundTransparency = 1
	minionHost.Size = Env.UDim2.fromScale(1, 1)
	minionHost.ZIndex = z + 3
	minionHost.Parent = root

	local enemyHost = Env.Instance.new("Frame")
	enemyHost.Name = "Enemies"
	enemyHost.BackgroundTransparency = 1
	enemyHost.Size = Env.UDim2.fromScale(1, 1)
	enemyHost.ZIndex = z + 4
	enemyHost.Parent = root

	local projHost = Env.Instance.new("Frame")
	projHost.Name = "Projectiles"
	projHost.BackgroundTransparency = 1
	projHost.Size = Env.UDim2.fromScale(1, 1)
	projHost.ZIndex = z + 5
	projHost.Parent = root

	local tank = makeMarkerFrame(root, z + 6, true)

	local barBack = Env.Instance.new("Frame")
	barBack.Name = "HpBack"
	barBack.BorderSizePixel = 0
	barBack.BackgroundColor3 = Env.color(Render.PALETTE.hp_back)
	barBack.BackgroundTransparency = 0.45
	barBack.ZIndex = z + 7
	barBack.Parent = root

	local barFill = Env.Instance.new("Frame")
	barFill.Name = "HpFill"
	barFill.BorderSizePixel = 0
	barFill.BackgroundColor3 = Env.color(Render.PALETTE.hp_fill)
	barFill.ZIndex = z + 8
	barFill.Parent = root

	local veil = Env.Instance.new("Frame")
	veil.Name = "DeadVeil"
	veil.BorderSizePixel = 0
	veil.Size = Env.UDim2.fromScale(1, 1)
	veil.BackgroundColor3 = Env.color(Render.PALETTE.dead_veil)
	veil.BackgroundTransparency = 0.5
	veil.Visible = false
	veil.ZIndex = z + 9
	veil.Parent = root

	local self = setmetatable({
		interp = Render.newInterp(o.delay),
		_root = root,
		_ring = ring,
		_tank = tank,
		_barBack = barBack,
		_barFill = barFill,
		_veil = veil,
		_hosts = { enemies = enemyHost, projectiles = projHost, minions = minionHost },
		_pools = { enemies = {}, projectiles = {}, minions = {} },
		_z = z,
		_plan = nil,
	}, ShellImpl)
	return (self :: any) :: Handle
end

function ShellImpl.submit(self: any, view: RenderView, time: number?)
	self.interp:push(view, time or Env.now())
end

function ShellImpl.reset(self: any)
	self.interp:reset()
end

function ShellImpl.lastPlan(self: any): DrawPlan?
	return self._plan
end

function ShellImpl.instanceCount(self: any): number
	local n = 0
	for _, pool in self._pools do
		n += #pool
	end
	-- root, ring, 3 hosts, tank, 2 bar frames, veil.
	return n + 9
end

function ShellImpl.draw(self: any, now: number?)
	local s = self.interp:sample(now or Env.now())
	if s == nil then
		return
	end

	local abs = self._root.AbsoluteSize
	local vp: Viewport = { x = 0, y = 0, w = abs.X, h = abs.Y }
	if vp.w <= 0 or vp.h <= 0 then
		return
	end

	local plan = Render.layout(s, vp)
	self._plan = plan

	self._ring.Size = Env.UDim2.fromOffset(math.round(plan.ring.size), math.round(plan.ring.size))
	self._ring.Position = Env.UDim2.fromOffset(math.round(plan.ring.x), math.round(plan.ring.y))
	self._ring.Visible = true

	applyPool(self._pools.minions, self._hosts.minions, plan.minions, Render.POOL_LIMITS.minions, self._z + 3)
	applyPool(self._pools.enemies, self._hosts.enemies, plan.enemies, Render.POOL_LIMITS.enemies, self._z + 4)
	applyPool(
		self._pools.projectiles,
		self._hosts.projectiles,
		plan.projectiles,
		Render.POOL_LIMITS.projectiles,
		self._z + 5
	)

	self._tank.Size = Env.UDim2.fromOffset(math.round(plan.tank.size), math.round(plan.tank.size))
	self._tank.Position = Env.UDim2.fromOffset(math.round(plan.tank.x), math.round(plan.tank.y))
	self._tank.BackgroundColor3 = Env.color(plan.tank.color)
	self._tank.Visible = true

	local bar = plan.hpBar
	self._barBack.Position = Env.UDim2.fromOffset(math.round(bar.x), math.round(bar.y))
	self._barBack.Size = Env.UDim2.fromOffset(math.round(bar.w), math.round(bar.h))
	self._barFill.Position = Env.UDim2.fromOffset(math.round(bar.x), math.round(bar.y))
	self._barFill.Size = Env.UDim2.fromOffset(math.round(bar.w * bar.fill), math.round(bar.h))
	self._barFill.BackgroundColor3 =
		Env.color(if plan.dead then Render.PALETTE.hp_dead else Render.PALETTE.hp_fill)

	self._veil.Visible = plan.dead
end

function ShellImpl.destroy(self: any)
	self._root:Destroy()
	self._pools = { enemies = {}, projectiles = {}, minions = {} }
	self._plan = nil
end

return Render
