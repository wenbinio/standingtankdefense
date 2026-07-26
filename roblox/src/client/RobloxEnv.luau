--!nonstrict
--[[
	RobloxEnv.luau — the ONE place in `src/client/` allowed to notice Roblox.

	WHY THIS EXISTS
	---------------
	Everything else in the client is written as a **pure core plus a thin drawing
	shell** (`PHASE2-CONTRACTS.md` P6 / the house style established by
	`src/shared/**`): the formatting, layout, encoding and state-machine logic is
	plain Luau that a bare `luau` interpreter can run and a test can assert on,
	and only the last hop — creating `Instance`s and pushing pixels — touches a
	Roblox global.

	Concentrating the globals here buys three things:

	  1. `Render`/`Hud`/`ShopUi`/`RunFlow` stay `--!strict` and never name a
	     Roblox global, so `luau-analyze` is clean on them without exemptions.
	  2. `roblox/test/client_test.luau` can `require` all four under a bare
	     interpreter; they simply take the `Env.available == false` path.
	  3. If the presentation layer is ever swapped (3D parts instead of a
	     `ScreenGui`, or a different UI framework), this file is the seam.

	THE DEAD-BRANCH ASSIGNMENT
	--------------------------
	`luau-analyze` rejects a read of an unbound global ("Unknown global 'game';
	consider assigning to it first"). The `if (nil :: any) then ... end` block
	below is never executed — it exists purely to bind those names for the
	analyzer, which is the mechanism the diagnostic itself suggests. The
	alternative, `getfenv(0)`, trips `DeprecatedApi` and deoptimizes the script
	under Roblox, so it is deliberately not used.

	Under a bare interpreter every one of these globals reads as `nil`, so
	`Env.available` is `false` and no shell code runs.
]]

if (nil :: any) then
	-- NEVER EXECUTED. Analyzer-only global bindings; see the header.
	game = (nil :: any)
	script = (nil :: any)
	task = (nil :: any)
	Instance = (nil :: any)
	Color3 = (nil :: any)
	UDim2 = (nil :: any)
	UDim = (nil :: any)
	Vector2 = (nil :: any)
	Enum = (nil :: any)
end

local Env = {}

--- True only inside a real Roblox VM. Everything below is `nil` when false.
Env.available = (game ~= nil) and (Instance ~= nil)

Env.game = game
Env.script = script
Env.task = task
Env.Instance = Instance
Env.Color3 = Color3
Env.UDim2 = UDim2
Env.UDim = UDim
Env.Vector2 = Vector2
Env.Enum = Enum

--- `game:GetService(name)`, or `nil` off-Roblox. Never raises, so a caller can
--- probe for a service without guarding twice.
function Env.service(name: string): any
	if not Env.available then
		return nil
	end
	local ok, svc = pcall(function()
		return game:GetService(name)
	end)
	if ok then
		return svc
	end
	return nil
end

--- A monotonic-ish seconds clock. `os.clock` exists in BOTH environments, which
--- is what lets the interpolator and the run-flow timers be driven identically
--- from a test and from a live frame loop. The pure modules never call this —
--- they take `now` as an argument — so no determinism surface is involved.
function Env.now(): number
	return os.clock()
end

--- Convert a pure `{r, g, b}` palette entry (0..1 floats) to a `Color3`.
--- Off-Roblox this returns the table unchanged so layout code can be inspected.
function Env.color(c): any
	if not Env.available then
		return c
	end
	return Color3.new(c.r, c.g, c.b)
end

return Env
