--!strict
--[[
	ShopUi.luau — the per-round offer grid, and the buy / reroll / clear actions.

	AUTHORITY (PHASE2-CONTRACTS.md P5 — the load-bearing property of this file)
	---------------------------------------------------------------------------
	**This module never applies an effect.** It reads a `RenderView`, draws the
	offers in it, and when the player presses something it emits ONE non-negative
	integer — the `SIM-SPEC` §S6 input code — to its `onInput` callback. It does
	not decrement gold, does not append to the arsenal, does not grey a slot it
	just "bought", does not touch the view table at all. The server applies the
	input and the next snapshot shows the result. A shop UI that predicted its own
	purchases would be the first client-authored state in the game, and everything
	downstream of that (gold, offers, RNG) is competitively load-bearing.

	`affordable` / `enabled` in the model are therefore **presentation hints**
	derived from the server's own last snapshot, not permissions. Pressing a
	disabled control emits nothing purely so the player is not confused; it is not
	a security boundary, and the server validates every code regardless.

	THE ENCODING (the known bug source — asserted in the test)
	-----------------------------------------------------------
	    code = kind * 256 + slot      kind in the HIGH byte
	    kind 0 Noop · 1 BuyOffer(slot) · 2 Reroll · 3 Clear
	    slot is the 0-BASED shop index and is 0 for every kind but BuyOffer

	Two agents previously picked opposite byte orders and cost a diverge-at-tick-0
	on the first oracle run (`SIM-SPEC` §S6). So this module does not re-derive the
	packing: it delegates to `sim/Input.luau`, the module that owns it, and
	`client_test.luau` asserts the result against BOTH the literal expected
	integers and `Input.kind`/`Input.slot`'s decode. `RenderOffer.slot` is already
	0-based (`View.luau`), so slots pass through untouched — the 1-indexing hazard
	lives entirely on the sim side of the boundary.

	No Roblox global is referenced above the shell banner.
]]

local View = require("../shared/sim/View")
local Input = require("../shared/sim/Input")
local Env = require("./RobloxEnv")

export type RenderView = View.RenderView

local ShopUi = {}

--- Re-export of `ids::Input`'s wire tags, so a caller never writes `1` for
--- `BuyOffer` by hand.
ShopUi.Kind = Input.Kind

ShopUi.NOOP_CODE = Input.NOOP
ShopUi.REROLL_CODE = Input.REROLL
ShopUi.CLEAR_CODE = Input.CLEAR

--- `shop.rs` draws a fixed 4-weapon + 4-modifier layout (`docs/10` F3's
--- correction: there is no rarity weighting and no variable layout). The pool is
--- sized to that, and `model` still iterates `#view.shop` so a layout change is
--- a data change, not a code change.
ShopUi.MAX_SLOTS = 8

--- `slot` is a `u8` in the encoding, so this is the hard bound.
ShopUi.MAX_SLOT_INDEX = 255

--- 0 common · 1 uncommon · 2 rare · 3 epic (`View.luau`'s `RenderOffer.rarity`).
ShopUi.RARITY_NAMES = table.freeze({ "COMMON", "UNCOMMON", "RARE", "EPIC" })

export type Rgb = { r: number, g: number, b: number }

ShopUi.RARITY_COLORS = table.freeze({
	{ r = 0.68, g = 0.70, b = 0.74 }, -- common
	{ r = 0.42, g = 0.78, b = 0.52 }, -- uncommon
	{ r = 0.40, g = 0.62, b = 0.94 }, -- rare
	{ r = 0.76, g = 0.48, b = 0.92 }, -- epic
})

--=============================================================================
-- Encoding (pure) — delegated to `sim/Input.luau`, never re-derived
--=============================================================================

--- The input code for buying shop slot `slot` (0-BASED, as `RenderOffer.slot`
--- already is). Errors on a slot that cannot survive the `u8` packing, rather
--- than silently emitting a code that decodes to a different action.
function ShopUi.buyCode(slot: number): number
	if slot ~= math.floor(slot) or slot < 0 or slot > ShopUi.MAX_SLOT_INDEX then
		error(string.format("ShopUi.buyCode: slot %s is not a u8 shop index", tostring(slot)), 2)
	end
	return Input.buyOffer(slot)
end

--- Decode a code back to `(kind, slot)` — the inverse used by the test and by
--- any server-side echo.
function ShopUi.decode(code: number): (number, number)
	return Input.kind(code), Input.slot(code)
end

--=============================================================================
-- Model (pure)
--=============================================================================

export type OfferModel = {
	slot: number,
	code: number,
	name: string,
	cost: number,
	cost_text: string,
	is_weapon: boolean,
	affordable: boolean,
	enabled: boolean,
	rarity: number,
	rarity_name: string,
	color: Rgb,
	label: string,
	kind_text: string,
}

export type ShopModel = {
	dead: boolean,
	gold: number,
	gold_text: string,
	offers: { OfferModel },
	reroll: {
		code: number,
		cost: number,
		cost_text: string,
		remaining: number,
		affordable: boolean,
		enabled: boolean,
		label: string,
	},
	clear: {
		code: number,
		enabled: boolean,
		--- FALSE, always: `RenderView` exposes no `clear_cooldown_end`, so the
		--- client cannot show a real cooldown. See the seam note in the report.
		cooldown_known: boolean,
		label: string,
	},
}

local function group(n: number): string
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
	return (if neg then "-" else "") .. out
end

local function rarityName(rarity: number): string
	return ShopUi.RARITY_NAMES[rarity + 1] or "?"
end

local function rarityColor(rarity: number): Rgb
	return ShopUi.RARITY_COLORS[rarity + 1] or ShopUi.RARITY_COLORS[1]
end

--- Everything the shop panel shows, fully formatted, from one `RenderView`.
--- Read-only: `view` is never written to.
function ShopUi.model(view: RenderView): ShopModel
	local eco = view.economy
	local dead = view.dead

	local offers: { OfferModel } = {}
	for i = 1, #view.shop do
		local off = view.shop[i]
		local affordable = off.affordable
		offers[i] = {
			slot = off.slot,
			code = ShopUi.buyCode(off.slot),
			name = off.name,
			cost = off.cost,
			cost_text = group(off.cost) .. "g",
			is_weapon = off.is_weapon,
			affordable = affordable,
			enabled = affordable and not dead,
			rarity = off.rarity,
			rarity_name = rarityName(off.rarity),
			color = rarityColor(off.rarity),
			kind_text = if off.is_weapon then "WEAPON" else "MODIFIER",
			label = string.format("%s\n%s", off.name, group(off.cost) .. "g"),
		}
	end

	local rerollAffordable = eco.reroll_cost <= eco.gold
	local rerollAvailable = rerollAffordable and eco.rerolls_remaining > 0

	return {
		dead = dead,
		gold = eco.gold,
		gold_text = group(eco.gold) .. "g",
		offers = offers,
		reroll = {
			code = ShopUi.REROLL_CODE,
			cost = eco.reroll_cost,
			cost_text = group(eco.reroll_cost) .. "g",
			remaining = eco.rerolls_remaining,
			affordable = rerollAffordable,
			enabled = rerollAvailable and not dead,
			label = string.format("REROLL  %sg  (%d left)", group(eco.reroll_cost), eco.rerolls_remaining),
		},
		clear = {
			code = ShopUi.CLEAR_CODE,
			enabled = not dead,
			cooldown_known = false,
			label = "CLEAR",
		},
	}
end

--=============================================================================
-- Actions (pure) — the ONLY thing this module produces is an integer
--=============================================================================

export type Action = "buy" | "reroll" | "clear"

--[[
	Resolve a press into the code to send, or `nil` if the control is disabled in
	the last snapshot the server gave us.

	`slot` is required for `"buy"` and is the 0-BASED `RenderOffer.slot`. An
	unknown slot returns `nil` rather than fabricating a code — the offer grid can
	shrink between a render and a press (a round boundary regenerates offers), and
	an out-of-range buy would be a wasted round-trip the server has to reject.
]]
function ShopUi.press(model: ShopModel, action: Action, slot: number?): number?
	if action == "reroll" then
		if not model.reroll.enabled then
			return nil
		end
		return model.reroll.code
	elseif action == "clear" then
		if not model.clear.enabled then
			return nil
		end
		return model.clear.code
	elseif action == "buy" then
		if slot == nil then
			return nil
		end
		for i = 1, #model.offers do
			local o = model.offers[i]
			if o.slot == slot then
				if not o.enabled then
					return nil
				end
				return o.code
			end
		end
		return nil
	end
	return nil
end

--=============================================================================
-- Roblox shell
--=============================================================================

export type Handle = {
	update: (Handle, RenderView) -> ShopModel?,
	lastModel: (Handle) -> ShopModel?,
	instanceCount: (Handle) -> number,
	destroy: (Handle) -> (),
}

local ShellImpl = {}
ShellImpl.__index = ShellImpl

local DISABLED = { r = 0.34, g = 0.34, b = 0.38 }
local PANEL = { r = 0.09, g = 0.10, b = 0.13 }
local TEXT = { r = 0.90, g = 0.92, b = 0.96 }

local function button(parent: any, name: string): any
	local b = Env.Instance.new("TextButton")
	b.Name = name
	b.BorderSizePixel = 0
	b.BackgroundColor3 = Env.color(PANEL)
	b.Font = Env.Enum.Font.GothamMedium
	b.TextSize = 14
	b.TextColor3 = Env.color(TEXT)
	b.Text = ""
	b.AutoButtonColor = true
	b.Parent = parent

	local stroke = Env.Instance.new("UIStroke")
	stroke.Name = "Rarity"
	stroke.Thickness = 2
	stroke.Parent = b
	return b
end

--[[
	Build the shop panel under `parent`. `onInput` receives the SIM-SPEC §S6 code
	and nothing else — it is the whole client→server surface of this module, and
	the natural place to hand the code to `Wire` when that lands (P4).

	Returns `nil` off-Roblox.
]]
function ShopUi.new(parent: any, onInput: (number) -> ()): Handle?
	if not Env.available then
		return nil
	end

	local root = Env.Instance.new("Frame")
	root.Name = "Shop"
	root.BorderSizePixel = 0
	root.BackgroundColor3 = Env.color({ r = 0.05, g = 0.055, b = 0.07 })
	root.BackgroundTransparency = 0.25
	root.AnchorPoint = Env.Vector2.new(0.5, 1)
	root.Position = Env.UDim2.new(0.5, 0, 1, -12)
	root.Size = Env.UDim2.new(0, 8 * 118 + 9 * 8, 0, 96)
	root.Parent = parent

	local grid = Env.Instance.new("UIListLayout")
	grid.FillDirection = Env.Enum.FillDirection.Horizontal
	grid.Padding = Env.UDim.new(0, 8)
	grid.HorizontalAlignment = Env.Enum.HorizontalAlignment.Center
	grid.VerticalAlignment = Env.Enum.VerticalAlignment.Center
	grid.Parent = root

	-- The click handlers must resolve against the LATEST server snapshot, not one
	-- captured when the button was built, so the model lives in this shared box.
	-- (It cannot live on the `Instance`: Roblox instances reject arbitrary
	-- fields.) `update` writes it; every handler reads it.
	local box: { model: ShopModel? } = { model = nil }

	local function emit(action: Action, slot: number?)
		local m = box.model
		if m == nil then
			return
		end
		local code = ShopUi.press(m, action, slot)
		if code ~= nil then
			onInput(code)
		end
	end

	local slots = {}
	for i = 1, ShopUi.MAX_SLOTS do
		local b = button(root, "Slot" .. i)
		b.Size = Env.UDim2.fromOffset(118, 72)
		b.LayoutOrder = i
		b.Visible = false
		-- The button captures its 0-BASED slot, not its 1-based pool index. This
		-- is the off-by-one the encoding bug lived in; it is converted exactly
		-- once, here, and `RenderOffer.slot` is 0-based all the way from the sim.
		local slotIndex = i - 1
		b.Activated:Connect(function()
			emit("buy", slotIndex)
		end)
		slots[i] = b
	end

	local reroll = button(root, "Reroll")
	reroll.Size = Env.UDim2.fromOffset(160, 72)
	reroll.LayoutOrder = ShopUi.MAX_SLOTS + 1
	reroll.Activated:Connect(function()
		emit("reroll")
	end)

	local clear = button(root, "Clear")
	clear.Size = Env.UDim2.fromOffset(110, 72)
	clear.LayoutOrder = ShopUi.MAX_SLOTS + 2
	clear.Activated:Connect(function()
		emit("clear")
	end)

	local self = setmetatable({
		_root = root,
		_box = box,
		_slots = slots,
		_reroll = reroll,
		_clear = clear,
		_model = nil,
	}, ShellImpl)
	return (self :: any) :: Handle
end

function ShellImpl.update(self: any, view: RenderView): ShopModel
	local m = ShopUi.model(view)
	self._model = m
	self._box.model = m

	for i = 1, ShopUi.MAX_SLOTS do
		local b = self._slots[i]
		local o = m.offers[i]
		if o == nil then
			b.Visible = false
		else
			b.Visible = true
			b.Text = o.label
			b.TextColor3 = Env.color(if o.enabled then TEXT else DISABLED)
			b.Rarity.Color = Env.color(o.color)
			b.Rarity.Transparency = if o.enabled then 0 else 0.6
		end
	end

	self._reroll.Text = m.reroll.label
	self._reroll.TextColor3 = Env.color(if m.reroll.enabled then TEXT else DISABLED)
	self._clear.Text = m.clear.label
	self._clear.TextColor3 = Env.color(if m.clear.enabled then TEXT else DISABLED)

	return m
end

function ShellImpl.lastModel(self: any): ShopModel?
	return self._model
end

function ShellImpl.instanceCount(self: any): number
	-- root + list layout + (MAX_SLOTS + 2) buttons, each with one UIStroke.
	return 2 + (ShopUi.MAX_SLOTS + 2) * 2
end

function ShellImpl.destroy(self: any)
	self._root:Destroy()
	self._model = nil
end

return ShopUi
