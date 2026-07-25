--[[
	run.server.luau — Studio / live-server entry point for the R2 spike.

	Rojo maps `roblox/bench/` to ServerScriptService.Bench, so this Script runs
	automatically when the place starts. It does two things the standalone
	interpreter cannot:

	  1. Runs the same measurement core inside the real Roblox VM, so the numbers
	     reflect Roblox's own Luau build rather than a vanilla one.
	  2. Runs the arenas on 8 genuinely parallel Luau `Actor`s (docs/10 §F6) and
	     measures WALL CLOCK for all 8 — which is the number the R2 gate is
	     actually about. The standalone bench can only extrapolate.

	Nothing here is imported by the measurement core; `throughput.luau` has no
	dependency on this file.
]]

local RunService = game:GetService("RunService")

local Bench = require(script.Parent.throughput)

--=============================================================================
-- 1. Serial, in-process: the same report the standalone interpreter prints.
--=============================================================================

print("[R2] serial in-process run starting; this takes ~30-60 s.")
Bench.run({
	-- Keep each individual timed run short and hand the frame back between
	-- them, so a live server is not starved and ScriptTimeoutLength is not hit.
	targetSec = 0.15,
	opIters = 40000,
	yield = function()
		task.wait()
	end,
})

--=============================================================================
-- 2. Parallel: 8 arenas, 8 Actors, wall clock.
--=============================================================================

local ARENAS = 8
local TICKS = 300
local POP = 300
local WEAPONS = 20
local TICK_BUDGET_MS = 1000 / 30
local SAFE_SHARE = 0.30

local template = script.Parent:FindFirstChild("ArenaWorker")
if not template then
	warn("[R2] ArenaWorker missing; skipping the parallel measurement.")
	return
end

local host = Instance.new("Folder")
host.Name = "R2Actors"
host.Parent = script.Parent

local actors = {}
for i = 1, ARENAS do
	local actor = Instance.new("Actor")
	actor.Name = "Arena" .. i

	local done = Instance.new("BoolValue")
	done.Name = "Done"
	done.Parent = actor

	local elapsed = Instance.new("NumberValue")
	elapsed.Name = "Elapsed"
	elapsed.Parent = actor

	local worker = template:Clone()
	worker.Disabled = false
	worker.Parent = actor

	actor.Parent = host
	actors[i] = actor
end

-- Give the workers a frame to bind their message handlers.
task.wait(0.5)

local function runWave(backend: string): (number, number)
	for _, a in actors do
		a.Done.Value = false
		a.Elapsed.Value = 0
	end
	local t0 = os.clock()
	for _, a in actors do
		a:SendMessage("run", TICKS, POP, WEAPONS, backend)
	end
	local deadline = os.clock() + 120
	while os.clock() < deadline do
		local all = true
		for _, a in actors do
			if not a.Done.Value then
				all = false
				break
			end
		end
		if all then
			break
		end
		RunService.Heartbeat:Wait()
	end
	local wall = os.clock() - t0
	local worst = 0
	for _, a in actors do
		worst = math.max(worst, a.Elapsed.Value)
	end
	return wall / TICKS * 1000, worst / TICKS * 1000
end

print("")
print("================================================================================")
print(string.format("  R2 PARALLEL ACTOR MEASUREMENT — %d arenas, E=%d, W=%d, %d ticks", ARENAS, POP, WEAPONS, TICKS))
print("================================================================================")
print("  backend    wall ms/tick   slowest arena   % of 33.33ms   % of 30% budget")
for _, backend in { "exact", "api-dbl", "raw-dbl" } do
	local wallMs, worstMs = runWave(backend)
	print(string.format(
		"  %-8s %13.3f %15.3f %13.1f%% %16.1f%%",
		backend, wallMs, worstMs, wallMs / TICK_BUDGET_MS * 100, wallMs / (TICK_BUDGET_MS * SAFE_SHARE) * 100
	))
end
print("")
print("  'wall ms/tick' includes Actor dispatch and the synchronize barrier, which is")
print("  exactly what the standalone bench cannot see. If it is far above")
print("  'slowest arena', the worker pool is smaller than 8 and the arenas are")
print("  queueing rather than running concurrently — that is the thing to check.")
print("================================================================================")

host:Destroy()
