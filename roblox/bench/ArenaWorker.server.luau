--[[
	ArenaWorker.server.luau — one arena, one parallel Luau Actor (docs/10 §F6).

	Ships DISABLED (see ArenaWorker.meta.json). `run.server.luau` clones it into
	an `Actor`, enables it, and drives it by message. It is deliberately tiny:
	all it does is run the measurement core from `throughput.luau` in parallel
	context and report how long that took.

	Reporting goes through Instance values rather than a return channel because
	an Actor cannot send a message back out; writing a value requires serial
	context, hence the `task.synchronize()`.
]]

local actor = script:GetActor()
if not actor then
	return
end

-- Bench folder <- Actor <- this script.
local Bench = require(actor.Parent.Parent.throughput)

local doneFlag = actor:WaitForChild("Done")
local elapsedValue = actor:WaitForChild("Elapsed")

actor:BindToMessageParallel("run", function(ticks: number, pop: number, weapons: number, backend: string)
	local cfg = Bench.makeConfig(pop, weapons, ticks)
	local t0 = os.clock()
	Bench.runBackend(backend, cfg)
	local dt = os.clock() - t0

	task.synchronize()
	elapsedValue.Value = dt
	doneFlag.Value = true
end)
