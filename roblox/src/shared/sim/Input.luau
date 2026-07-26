--[[
	Input.luau — transcription of `sim-core/crates/sim/src/input.rs` (phase 2 of
	the tick: apply one player action) plus the `ids::Input` enum itself.
	Spec: `roblox/SIM-SPEC.md` §S2, §S3, §S6.

	THE INPUT ENCODING
	------------------
	Rust's `ids::Input` is a payload-carrying enum. Luau has none, so an input is
	carried as the single non-negative integer `code` that `SIM-SPEC` §S6 already
	puts in the trace log — no wrapper table, no allocation per tick, and the
	trace's `code` field is directly applicable:

	    code = kind * 256 + slot          (kind = code // 256, slot = code % 256)

	    kind 0  Noop
	    kind 1  BuyOffer(slot)   slot is a u8, 0-BASED (shop.offers[slot + 1])
	    kind 2  Reroll
	    kind 3  Clear

	The `kind` numbers are NOT invented: they are `net::wire::InputCode`'s
	on-the-wire tags (`wire.rs` / `replay.rs` encode/decode `0/1/2/3`, with
	`BuyOffer`'s slot as the following byte). This module packs those same two
	bytes into one integer, little-endian, which is the only part §S6 left open.
	If the Rust trace exporter picks a different packing, THIS is the file to
	change — nothing else knows the layout.

	ENGINE INDEPENDENCE: no Roblox global is referenced.
]]

local Content = require("../Content")
local State = require("./State")
local Shop = require("./Shop")

local Input = {}

--=============================================================================
-- ids::Input
--=============================================================================

--- Action kinds — `net::wire::InputCode`'s wire tags.
Input.Kind = table.freeze({
	Noop = 0,
	BuyOffer = 1,
	Reroll = 2,
	Clear = 3,
})

--- Ready-made codes for the payload-free actions.
Input.NOOP = 0
Input.REROLL = 2 * 256
Input.CLEAR = 3 * 256

--- `Input::BuyOffer { slot }` — `slot` is a 0-BASED u8 shop slot.
function Input.buyOffer(slot: number): number
	return 256 + slot
end

--- The action kind of a code (`Input.Kind.*`).
function Input.kind(code: number): number
	return (code - code % 256) / 256
end

--- The `BuyOffer` slot carried by a code (0-BASED; 0 for every other kind).
function Input.slot(code: number): number
	return code % 256
end

--=============================================================================
-- input.rs
--=============================================================================

--- `CLEAR_DAMAGE`, `BOSS_CLEAR_DAMAGE` and `CLEAR_COOLDOWN_TICKS` all live in
--- `content.rs` now (next to the boss plate constants they are balanced against)
--- and ship in the C1 artifact, so they are READ, never pinned here — a local copy
--- of a balance number drifts silently the first time it is retuned. `Content.luau`
--- exposes them both as flattened UPPER_SNAKE names and under `constants`; hedge
--- the same way the sim modules do, and error loudly if neither is present.
local function contentConst(name: string, snake: string): number
	local v = Content[name]
	if v == nil and Content.constants ~= nil then
		v = Content.constants[snake]
	end
	if v == nil then
		error("Input: Content is missing " .. name, 2)
	end
	return v
end

--- Damage dealt to every NON-boss enemy by a `Clear`. Large but FINITE: it wipes
--- the board instantly.
local CLEAR_DAMAGE = contentConst("CLEAR_DAMAGE", "clear_damage")
--- Damage a `Clear` deals to a BOSS — a deliberately MINORITY share of its bar
--- (280k against 6.3M HP; the 11 Clears a clean ~110 s fight affords come to 3.08M),
--- because `Clear` is no longer the only thing that can hurt it. Its real boss value
--- is now the BREACH it opens (`Combat`'s `bossBreached`, derived from
--- `clear_cooldown_end` — nothing extra is stored here for it).
local BOSS_CLEAR_DAMAGE = contentConst("BOSS_CLEAR_DAMAGE", "boss_clear_damage")
--- Cooldown (in ticks) imposed after a `Clear`. Read by `Combat` as well: the
--- breach window is `clear_cooldown_end` minus this constant.
local CLEAR_COOLDOWN_TICKS = contentConst("CLEAR_COOLDOWN_TICKS", "clear_cooldown_ticks")
--- Gold increment added to `reroll_cost` after a paid reroll.
local REROLL_COST_STEP = 100

--- `input::apply` — phase 2: apply `inp`.
--- - `Noop`: nothing.
--- - `BuyOffer{slot}`: if the slot is valid and `gold >= cost`, deduct gold and
---   grant the offer; otherwise ignore (illegal/insufficient).
--- - `Reroll`: if `rerolls_remaining > 0`, decrement then `shop::generate_offers`;
---   else if `gold >= reroll_cost`, deduct, regenerate, and raise `reroll_cost`.
---   Otherwise ignore.
--- - `Clear`: if `tick >= tank.clear_cooldown_end`, damage ALL enemies —
---   `CLEAR_DAMAGE` for the board, the much smaller `BOSS_CLEAR_DAMAGE` for a boss
---   (killed ones push their `def` to `pending_kills` and are removed) — and set
---   `clear_cooldown_end = tick + CLEAR_COOLDOWN_TICKS`. Else ignore. That cooldown
---   field is ALSO what `Combat`'s breach window is derived from, so a `Clear`
---   opens the boss's armor plates without storing any new state.
---
--- Consumes NO RNG directly. `Reroll` draws only through `Shop.generateOffers`
--- (`s.rng_shop`), and only on the branches where the Rust calls it — an ignored
--- reroll must not regenerate, or `rng_shop` desynchronizes (§S4).
function Input.apply(s: State.ArenaState, inp: number)
	local kind = (inp - inp % 256) / 256

	if kind == 0 then -- Input::Noop
		return
	elseif kind == 1 then -- Input::BuyOffer { slot }
		local slot = inp % 256
		-- `s.shop.offers.get(idx)`: `slot` is 0-based, Luau arrays are 1-indexed.
		local offer = s.shop.offers[slot + 1]
		if offer == nil then
			return
		end
		-- A pending meta perk (`docs/06` #5) may apply to this purchase, but never
		-- to another META item (no self-duplication / chaining).
		--
		-- The Rust perk is `Copy`, so the fields are snapshotted here rather than
		-- held as a live reference into `s.pending_perk`.
		local perk_armed, perk_free, perk_extra = false, false, 0
		if not State.offerIsMeta(offer) then
			local rarity = State.offerRarity(offer)
			local pp = s.pending_perk
			if pp ~= nil and State.PendingPerk.matches(pp, rarity) then
				perk_armed, perk_free, perk_extra = true, pp.free, pp.extra_copies
			end
		end
		-- Black Market voucher makes the matching purchase free.
		local cost = offer.cost
		if perk_armed and perk_free then
			cost = 0
		end
		if s.economy.gold >= cost then
			s.economy.gold -= cost
			State.grantOffer(s, offer)
			-- Duplicator: grant the extra free copies, then consume the perk.
			if perk_armed then
				for _ = 1, perk_extra do
					State.grantOffer(s, offer)
				end
				s.pending_perk = nil
			end
		end
	elseif kind == 2 then -- Input::Reroll
		if s.economy.rerolls_remaining > 0 then
			s.economy.rerolls_remaining -= 1
			Shop.generateOffers(s)
		elseif s.economy.gold >= s.economy.reroll_cost then
			s.economy.gold -= s.economy.reroll_cost
			Shop.generateOffers(s)
			s.economy.reroll_cost += REROLL_COST_STEP
		end
	elseif kind == 3 then -- Input::Clear
		if s.tick >= s.tank.clear_cooldown_end then
			local enemies = s.enemies
			local survivors = {}
			local pending = s.pending_kills
			local cleared_damage = 0
			for i = 1, #enemies do
				local e = enemies[i]
				-- The boss takes a much smaller chunk than the board does: 280k vs
				-- 3M. `Clear`'s real boss value is now the BREACH it opens
				-- (`Combat`'s `bossBreached`), not this chip.
				local chunk = if Content.ENEMIES[e.def + 1].boss then BOSS_CLEAR_DAMAGE else CLEAR_DAMAGE
				local before = if e.hp > 0 then e.hp else 0
				e.hp -= chunk -- `saturating_sub`; see the i64 note in State.luau
				local after = if e.hp > 0 then e.hp else 0
				-- Actual HP removed (clamped: no credit past the kill).
				cleared_damage += before - after
				if e.hp <= 0 then
					pending[#pending + 1] = e.def
				else
					survivors[#survivors + 1] = e
				end
			end
			s.enemies = survivors
			State.recordPlayerDamage(s, cleared_damage)
			s.tank.clear_cooldown_end = s.tick + CLEAR_COOLDOWN_TICKS
		end
	else
		error(`Input.apply: unknown input kind {kind} (code {inp})`, 2)
	end
end

return Input
