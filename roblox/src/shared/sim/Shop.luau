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

-- ============================ offer pool (R6) ============================
-- Meta-progression restricts WHICH content ids the shop may draw (`docs/10` F3:
-- unlocks WIDEN the pool, they never reroll it). `nil` = unrestricted.
--
-- This has to live inside the draw and not outside it, for two reasons that
-- both bite silently:
--   * `Arena.step` generates the round's offers in phase 1 and applies input in
--     phase 2 of the SAME tick, so there is no point outside `step` at which a
--     caller could rewrite `s.shop.offers` before a `BuyOffer` reads them.
--     Post-step remapping is a race, not an integration.
--   * Restriction must happen in ID space, never by handing `Shop` a trimmed
--     catalog: the draw index is written straight into `offer.def`, and
--     `State.grantOffer` resolves that def against the FULL catalog. Subset
--     indices would grant the wrong weapon.
--
-- With `pool == nil` the branches below collapse to the pre-R6 code exactly --
-- same 8 draws, same ranges, same `rng_shop` cursor -- so the R3 oracle gate is
-- unaffected. Under F6 each arena is its own Actor (its own module instance),
-- so a per-arena pool does not leak across shards.
local pool = nil

--- Restrict the draw pool to these 0-based content ids. `nil` clears it.
--- Shape: `{ weapons = {id, ...}, modifiers = {id, ...} }`.
function Shop.setPool(p)
	pool = p
end

--- The active pool (`nil` when unrestricted).
function Shop.getPool()
	return pool
end


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
			-- `r` is a 0-based DRAW index; `def` is a 0-based content id. They are
			-- the same number only when the pool is unrestricted.
			local ids = pool and pool.weapons
			local r = Rng.below(s.rng_shop, if ids then #ids else nw)
			local def = if ids then ids[r + 1] else r
			offer = { kind = kind.Weapon, def = def, cost = weapons.get(def).cost }
		else
			local ids = pool and pool.modifiers
			local r = Rng.below(s.rng_shop, if ids then #ids else nm)
			local def = if ids then ids[r + 1] else r
			offer = { kind = kind.Modifier, def = def, cost = modifiers.get(def).cost }
		end
		offers[slot + 1] = offer
	end
	s.shop.offers = offers
	s.shop.shop_seq += 1
end

return Shop
