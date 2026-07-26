--!nonstrict
--[[
	Main.client.luau — composes the four client modules into a running shell.

	Rojo turns this into a `LocalScript` under `StarterPlayerScripts.Client`
	(`CONTRACTS.md` C5), so it is the client's entry point. It is deliberately
	THIN: it owns no formatting, no layout, no encoding and no state machine —
	every one of those lives in a pure module with a headless test. What lives
	here is the wiring that only a real Roblox host can exercise: the `ScreenGui`,
	the render loop, and the remotes.

	THE TRANSPORT SEAM (PROVISIONAL — flagged, not blessed)
	-------------------------------------------------------
	`src/shared/net/Wire.luau` is owned by the Replication task and does not exist
	yet, so this file must not depend on it. It therefore:

	  * looks for a `StdNet` folder under `ReplicatedStorage` and binds whatever
	    remotes it finds, and
	  * accepts a view payload that is EITHER a plain `RenderView` table or a
	    `buffer`; a `buffer` is handed to `Wire.unpackView` if `Wire` is present,
	    and otherwise dropped with one warning.

	**The remote names below are invented by this task and are NOT a contract.**
	They exist so the shell is runnable end to end; whatever the Replication task
	pins centrally wins, and this file changes to match. Same for the shape of the
	reliable `Sync` payload.

	Nothing here is verifiable without a Roblox host. Everything that IS verifiable
	headlessly was deliberately pushed out of this file.
]]

local Render = require("./Render")
local Hud = require("./Hud")
local ShopUi = require("./ShopUi")
local RunFlow = require("./RunFlow")
local Env = require("./RobloxEnv")

if not Env.available then
	-- Under a bare interpreter this module is inert, so the test suite can
	-- require the whole client tree without a host.
	return nil
end

local Players = Env.service("Players")
local RunService = Env.service("RunService")
local ReplicatedStorage = Env.service("ReplicatedStorage")

--- PROVISIONAL names — see the header. One folder, three remotes.
local NET_FOLDER = "StdNet"
local VIEW_REMOTE = "View" -- UnreliableRemoteEvent, 10-15 Hz (P4)
local SYNC_REMOTE = "Sync" -- RemoteEvent, low-rate truth (P4)
local INPUT_REMOTE = "Input" -- RemoteEvent, client -> server, codes only (P5)

local player = Players.LocalPlayer
local playerGui = player:WaitForChild("PlayerGui")

local gui = Env.Instance.new("ScreenGui")
gui.Name = "StandingTankDefense"
gui.ResetOnSpawn = false
gui.IgnoreGuiInset = true
gui.ZIndexBehavior = Env.Enum.ZIndexBehavior.Sibling
gui.Parent = playerGui

local net = ReplicatedStorage:FindFirstChild(NET_FOLDER)
local inputRemote = if net then net:FindFirstChild(INPUT_REMOTE) else nil

--- The ONLY thing this client ever sends: a `SIM-SPEC` §S6 input code (P5).
local function sendInput(code: number)
	if inputRemote then
		inputRemote:FireServer(code)
	end
end

--[[
	RETRY IS NOT A SIM INPUT. `ids::Input` has exactly four variants (Noop,
	BuyOffer, Reroll, Clear) and none of them means "give me a new run" — run
	lifecycle belongs to the director, not to the arena (`docs/10` F2/F6). So a
	retry request goes on the RELIABLE channel as its own message and is never
	smuggled through the input-code path. The message name below is provisional
	along with the rest of the transport seam; flagged in the report.
]]
local function sendRetry()
	if net then
		local sync = net:FindFirstChild(SYNC_REMOTE)
		if sync then
			sync:FireServer("retry")
		end
	end
end

local arena = Render.new(gui, { z = 0 })
local hud = Hud.new(gui)
local shop = ShopUi.new(gui, sendInput)
local flow = RunFlow.new()

local function pumpEvents(events)
	for _, e in events do
		if e.kind == "request_retry" then
			sendRetry()
		elseif e.kind == "run_started" and arena then
			-- Do not lerp the new arena out of the old one's corpse.
			arena:reset()
		end
	end
end

local results = RunFlow.newScreen(gui, function()
	pumpEvents(flow:retryNow(Env.now()))
end)

--- One arriving view: feed everything that reads a view.
local function onView(view)
	local now = Env.now()
	if arena then
		arena:submit(view, now)
	end
	if hud then
		hud:update(view)
	end
	if shop then
		shop:update(view)
	end
	pumpEvents(flow:observe(view, now))
end

if net then
	local viewRemote = net:FindFirstChild(VIEW_REMOTE)
	if viewRemote then
		local warned = false
		viewRemote.OnClientEvent:Connect(function(payload)
			if type(payload) == "table" then
				onView(payload)
				return
			end
			local wireModule = ReplicatedStorage:FindFirstChild("Shared")
			wireModule = wireModule and wireModule:FindFirstChild("net")
			wireModule = wireModule and wireModule:FindFirstChild("Wire")
			if wireModule then
				local Wire = require(wireModule)
				onView(Wire.unpackView(payload))
			elseif not warned then
				warned = true
				warn("[client] packed view received but Shared.net.Wire is not present; dropping.")
			end
		end)
	end
end

RunService.RenderStepped:Connect(function()
	local now = Env.now()
	pumpEvents(flow:update(now))
	if arena then
		arena:draw(now)
	end
	if results then
		results:update(flow, now)
	end
end)

return
