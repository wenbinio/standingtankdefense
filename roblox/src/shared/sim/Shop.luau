--[[
	Shop.luau — transcription of `sim-core/crates/sim/src/shop.rs`.

	Shop / offers. Deterministic; randomness only from `s.rng_shop`.

	RNG DRAW SEQUENCE (`rng_shop`), per call to `Shop.generateOffers` — exactly
	`OFFER_SLOTS` (8) draws, in slot order, no conditionals:
	    slot 0..3 : Rng.below(rng_shop, #WEAPONS)     (86)
	    slot 4..7 : Rng.below(rng_shop, #MODIFIERS)   (91)
	`Rng.below` is a rejection loop and consumes a VARIABLE number of u32 words
	per draw; `Rng.luau` reproduces the Rust loop word-for-word. Never substitute
	an "equivalent" unbiased method (`SIM-SPEC.md` §S4). `rng_shop` is drawn from
	HERE AND NOWHERE ELSE, matching the Rust.

	`rng_reroll` is NOT touched here — nor anywhere else in the Rust sim. It is
	derived in `ArenaState::new` and checksummed, but never drawn from: reroll
	cost escalation is pure arithmetic (`input.rs: REROLL_COST_STEP`). Any Luau
	draw from `rng_reroll` would desynchronize the checksum immediately.

	WHERE THE REROLL / PURCHASE PATHS LIVE. In the Rust they are in `input.rs`
	(`Input::Reroll`, `Input::BuyOffer`), not in `shop.rs`, so by SIM-SPEC §S2
	they belong in `Input.luau`. They are NOT duplicated here. For the record,
	so the `Input.luau` author and the integrator can check the seam:

	  Reroll — free rerolls first, then escalating gold:
	    if rerolls_remaining > 0 then rerolls_remaining -= 1; generateOffers(s)
	    elseif gold >= reroll_cost then gold -= reroll_cost; generateOffers(s);
	                                    reroll_cost += 100
	    else ignore (NO generateOffers ⇒ NO rng_shop draws — the ignored branch
	         must not advance the stream, or a rejected reroll becomes a
	         save-scum oracle)
	  Starting values (`state.rs`): rerolls_remaining = 5, reroll_cost = 100.

	  BuyOffer{slot} — the stateful meta-item machinery (`docs/02` §2.8):
	    perk = offer_is_meta(offer) and nil or pending_perk-if-it-matches-rarity
	    cost = (perk and perk.free) and 0 or offer.cost
	    if gold >= cost then gold -= cost; grant_offer(offer)
	       if perk then repeat grant_offer(offer) perk.extra_copies times
	                   pending_perk = nil end
	  `pending_perk` is ARMED by `ArenaState::buy_modifier` (state.rs) on
	  `GrantDuplicator(rarity, copies)` / `GrantVoucher(rarity)`. It carries NO
	  RNG, and — this is the part worth stating plainly — in the implemented Rust
	  it does **not** alter future OFFER DRAWS at all: it alters the next
	  matching PURCHASE (free / extra copies). See the report's spec-gap note
	  versus `docs/02` §2.5–2.8 (Black Market pool restriction, rarity-weighted
	  draws, "copy of the next Rare"), which the Rust does not implement.
	  Do not add them here; the oracle is generated from the Rust.
]]

local Rng = require("../Rng")

local Shop = {}

--- Number of purchasable slots presented each round / reroll.
local OFFER_SLOTS = 8
--- Of those, the first `WEAPON_SLOTS` are always weapons; the remainder are
--- always modifiers (economy / passives / spikes). A fixed layout so every
--- round offers a steady spread of both, never all-of-one-kind.
local WEAPON_SLOTS = 4

Shop.OFFER_SLOTS = OFFER_SLOTS
Shop.WEAPON_SLOTS = WEAPON_SLOTS

--- `OfferKind`. The values are the ones `lib.rs::checksum` writes
--- (`Weapon => 0`, `Modifier => 1`), so they are load-bearing, not cosmetic.
--- INTEGRATION NOTE: `State.luau` owns this enum (`state.rs`); replace this
--- table with `State.OfferKind` once it lands, or override via
--- `Shop.setOfferKind(State.OfferKind)`.
Shop.OfferKind = { Weapon = 0, Modifier = 1 }

function Shop.setOfferKind(t)
	Shop.OfferKind = t
end

-- ============================ content access ============================
-- INTEGRATION NOTE: see `Waves.luau`. Export names AND index base are resolved
-- once, at content-binding time, and cross-checked against `constants.num_*`.
-- The index base is load-bearing here: `Rng.below(rng_shop, n)` with the wrong
-- `n` desynchronizes `rng_shop` permanently and invisibly.

local contentModule = nil
local view = nil

--- Inject the content module (tests, or an explicit wiring from `Arena`).
function Shop.setContent(c)
	contentModule = c
	view = nil
end

local function pick(t, names, what)
	for _, n in names do
		local v = t[n]
		if v ~= nil then
			return v
		end
	end
	error("Shop: Content is missing " .. what .. " (tried: " .. table.concat(names, ", ") .. ")", 3)
end

--- Bind a catalog indexed by 0-based content ids (`SIM-SPEC.md` §S3) to a
--- uniform accessor, whichever index base the table actually uses.
local function bindCatalog(t, expected, what)
	local zeroBased = t[0] ~= nil
	local n = if zeroBased then #t + 1 else #t
	if expected ~= nil and n ~= expected then
		error(string.format("Shop: %s has %d entries, content says %d", what, n, expected), 3)
	end
	local base = if zeroBased then 0 else 1
	return {
		n = n,
		--- `id` is a 0-based content id.
		get = function(id)
			return t[id + base]
		end,
	}
end

local function content()
	if view ~= nil then
		return view
	end
	if contentModule == nil then
		local ok, mod = pcall(require, "../Content")
		if not ok then
			error(
				"Shop: no content — require('../Content') failed ("
					.. tostring(mod)
					.. "); call Shop.setContent(Content) first",
				3
			)
		end
		contentModule = mod
	end
	local c = contentModule
	local constants = pick(c, { "CONSTANTS", "constants" }, "the constants table")
	view = {
		weapons = bindCatalog(
			pick(c, { "WEAPONS", "weapons" }, "the weapon catalog"),
			constants.num_weapons,
			"the weapon catalog"
		),
		modifiers = bindCatalog(
			pick(c, { "MODIFIERS", "modifiers" }, "the modifier catalog"),
			constants.num_modifiers,
			"the modifier catalog"
		),
	}
	return view
end

-- ============================ generate_offers ============================

--- Generate a fresh set of offers: the first `WEAPON_SLOTS` drawn uniformly
--- from the weapon pool, the rest from the modifier pool, via `s.rng_shop`.
--- Replaces `s.shop.offers`, increments `s.shop.shop_seq`. Called at the round
--- boundary (`lib.rs::step` phase 1) and on every reroll (`input.rs`).
function Shop.generateOffers(s)
	local c = content()
	local weapons, modifiers = c.weapons, c.modifiers
	local nw = weapons.n
	local nm = modifiers.n
	local kind = Shop.OfferKind
	local offers = table.create(OFFER_SLOTS)
	-- `slot` is 0-based (mirrors the Rust `0..OFFER_SLOTS`); `offers` is a
	-- 1-indexed Luau array, so it is written at `slot + 1` (SIM-SPEC §S3).
	for slot = 0, OFFER_SLOTS - 1 do
		local offer
		if slot < WEAPON_SLOTS then
			-- `r` is a 0-based content id.
			local r = Rng.below(s.rng_shop, nw)
			offer = { kind = kind.Weapon, def = r, cost = weapons.get(r).cost }
		else
			local r = Rng.below(s.rng_shop, nm)
			offer = { kind = kind.Modifier, def = r, cost = modifiers.get(r).cost }
		end
		offers[slot + 1] = offer
	end
	s.shop.offers = offers
	s.shop.shop_seq += 1
end

return Shop
