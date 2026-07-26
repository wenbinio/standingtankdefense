--[[
	Shop.luau — transcription of `sim-core/crates/sim/src/shop.rs`.

	Shop / offers. Deterministic; randomness only from `s.rng_shop`.

	RARITY-WEIGHTED, ESCALATING OFFERS (`docs/02 §2.5`, `docs/11 §11.2`)
	-------------------------------------------------------------------
	Offers are **not** uniform over the catalog. Each slot is drawn in TWO stages:
	first a RARITY TIER from a round-dependent weight table, then a UNIFORM PICK
	INSIDE THAT TIER. Two-stage is the point — it makes the shown distribution
	exactly the designed one, independent of how many entries each tier happens to
	contain, so adding thirty Commons to the catalog does not flood the shop.

	RNG DRAW SEQUENCE (`rng_shop`), per call to `Shop.generateOffers` — exactly
	2 * `OFFER_SLOTS` = 16 draws, in slot order, no conditionals:
	    for slot = 0..7:
	      1. Rng.below(rng_shop, WEIGHT_TOTAL)   -- the tier roll; ALWAYS 40000,
	         never varying with round, kind, pool or catalog size. The walk over
	         the accumulated weights consumes no further randomness.
	      2. Rng.below(rng_shop, counts[t])      -- uniform inside the tier, where
	         `counts[t]` is the number of candidate entries of this slot's
	         EFFECTIVE rarity for this slot's kind (weapon for slots 0..3,
	         modifier for slots 4..7).
	The draw COUNT AND ORDER are a pure function of state — never of the values
	rolled. The tier → effective-tier fallback for an empty tier is a pure
	function of the candidate set and consumes no randomness, so an empty tier can
	never perturb the draw sequence.

	`Rng.below` is a rejection loop and consumes a VARIABLE number of u32 words
	per draw; `Rng.luau` reproduces the Rust loop word-for-word. Never substitute
	an "equivalent" unbiased method (`SIM-SPEC.md` §S4). `rng_shop` is drawn from
	HERE AND NOWHERE ELSE, matching the Rust.

	All of the weighting arithmetic is EXACT INTEGER arithmetic — per-mille weights
	scaled by the ramp length, never a float, never a division (§S4).

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
	  matching PURCHASE (free / extra copies). The rarity-WEIGHTED draw below is
	  the one `docs/02` §2.5 asked for and is now implemented; the Black Market
	  pool restriction is served by `Shop.setPool` (R6, Roblox-only, see below)
	  and "copy of the next Rare" remains a purchase-time effect. Do not add
	  anything else here; the oracle is generated from the Rust.
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
-- HOW IT COMPOSES WITH THE RARITY DRAW. The pool narrows the CANDIDATE SET, and
-- the two-stage draw then runs over that set: tier counts are counted over the
-- candidates, and the member roll picks the k-th candidate of the tier in POOL
-- ORDER. So restriction and weighting are orthogonal — a fresh profile still
-- sees the designed rarity mix, just drawn from fewer entries, and a tier the
-- pool cannot serve falls back exactly as the Rust's does (RNG-free).
--
-- With `pool == nil` the branches below collapse to the plain-catalog code
-- exactly — candidate i IS content id i, so the tier counts, the enumeration
-- order, all 16 draw ranges and the `rng_shop` cursor are identical — so the R3
-- oracle gate is unaffected. A pool listing every id in ascending order is the
-- same thing again, which is what `Meta.bindShop(Shop, Meta.fullPool())` relies
-- on. Under F6 each arena is its own Actor (its own module instance), so a
-- per-arena pool does not leak across shards.
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
--- round offers a steady spread of both, never all-of-one-kind. Weighting
--- changes *what* is drawn, never the layout.
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

-- ============================ rarity weighting ============================

--- Rarity tiers on the ladder: 0 Common, 1 Uncommon, 2 Rare, 3 Epic.
local TIERS = 4
Shop.TIERS = TIERS

-- The weight tables and every derived table below are indexed by the 0-BASED
-- tier (`[0]` … `[3]`), not by a 1-indexed Luau array position. Tier is a
-- content-space index like `def` is, and §S3 keeps those 0-based; making the
-- storage 0-based too removes the `+ 1` from every single site, which is the
-- only place this transcription could hide an off-by-one. Consequence: `#t` is
-- meaningless on these tables — always iterate `for t = 0, TIERS - 1`.

--- Tier weights, in per-mille, for the **first** shop of a run (round 0).
---
--- Common is the backdrop and Epic is an event: at 20/1000 per slot, an 8-slot
--- shop contains an Epic ~15% of the time, so you meet one every six or seven
--- shops — often enough to be a real hope, rare enough that hitting one (when
--- you almost certainly cannot afford its 5000 g) is worth bending the plan for.
--- Compare the previous uniform draw, which showed **15%** Epic *per slot*
--- (13/86 weapons) from the opening round — i.e. an Epic in essentially every
--- shop, which is why nothing ever felt like a hit.
local WEIGHTS_EARLY = table.freeze({ [0] = 600, [1] = 280, [2] = 100, [3] = 20 })

--- Tier weights, in per-mille, at `ESCALATION_ROUNDS` and after.
---
--- The inversion is the escalation: Common collapses from the backdrop to the
--- residue, and Epic goes from an event to the thing a late shop is *about*
--- (~1.6 Epic slots of 8). A minute-20 shop is materially a different object
--- from a minute-1 shop, which is precisely what `docs/11 §11.2` asks for.
local WEIGHTS_LATE = table.freeze({ [0] = 150, [1] = 300, [2] = 350, [3] = 200 })

--- Rounds over which the early table ramps linearly into the late table.
--- 40 rounds @ 30 s = **20 minutes** — the point `docs/11 §11.2` names as the
--- far end of the feel curve, and the sweep harness's cap. The last third of a
--- run (the 20→30 min boss run-up) sits at full escalation.
---
--- NOTE: this is a count of ROUNDS, so it is timeline-independent — it needs no
--- `round_ticks` rescaling for the Roblox fork (`docs/10` F1), unlike anything
--- expressed in ticks.
local ESCALATION_ROUNDS = 40

--- Total tier weight. Constant because both anchor tables sum to 1000 and the
--- interpolation is `EARLY*(N-l) + LATE*l`, so the sum is `1000*N` at every
--- round. A fixed `below` bound keeps the draw bound state-independent.
local WEIGHT_TOTAL = 1000 * ESCALATION_ROUNDS
Shop.WEIGHT_TOTAL = WEIGHT_TOTAL

do -- the Rust's compile-time asserts, at require time.
	local e, l = 0, 0
	for t = 0, TIERS - 1 do
		e += WEIGHTS_EARLY[t]
		l += WEIGHTS_LATE[t]
	end
	if e ~= 1000 or l ~= 1000 then
		error("Shop: anchor weight tables must each sum to 1000 (got " .. e .. " / " .. l .. ")")
	end
end

--- `u32::MAX` — `ArenaState.round`'s pre-first-round sentinel (`State.U32_MAX`,
--- duplicated as a literal only because `State.luau` requires the sim modules
--- and `Shop` must not require it back).
local U32_MAX = 4294967295

--- Tier weights for `round`, by exact integer interpolation between the two
--- anchor tables. No division, no floats: `w[t] = EARLY[t]*(N-l) + LATE[t]*l`
--- with `l = min(round, N)`. Sums to `WEIGHT_TOTAL` for every round.
---
--- `round` is `u32::MAX` before the first round starts (the `State.new`
--- sentinel); that is treated as round 0, so a shop generated before the clock
--- starts uses the opening table rather than the endgame one.
---
--- Returns a table indexed by the 0-based tier.
local function rarityWeights(round: number)
	local l = if round == U32_MAX then 0 else math.min(round, ESCALATION_ROUNDS)
	local e = ESCALATION_ROUNDS - l
	local w = {}
	for t = 0, TIERS - 1 do
		w[t] = WEIGHTS_EARLY[t] * e + WEIGHTS_LATE[t] * l
	end
	return w
end

--- `shop::rarity_weights`. Exposed for tests / tooling; the sim uses the local.
Shop.rarityWeights = rarityWeights

--- Roll a tier from `weights` (which must sum to `WEIGHT_TOTAL`).
--- ONE `Rng.below(rng, WEIGHT_TOTAL)` draw; the walk consumes no further
--- randomness. Returns a 0-based tier.
local function drawTier(rng, weights): number
	local r = Rng.below(rng, WEIGHT_TOTAL)
	for t = 0, TIERS - 2 do
		if r < weights[t] then
			return t
		end
		r -= weights[t]
	end
	return TIERS - 1
end

--- Map a rolled tier onto one the candidate set can actually serve: prefer the
--- tier itself, else the nearest **lower** tier, else the nearest higher one.
--- Deterministic, RNG-free, and a pure function of the candidate set — so an
--- empty tier can never perturb the draw sequence. Downward-first keeps a
--- missing Epic pool from silently inflating the perceived rarity of what is
--- offered.
local function effectiveTier(counts, tier: number): number
	if counts[tier] > 0 then
		return tier
	end
	for d = 1, TIERS - 1 do
		if d <= tier and counts[tier - d] > 0 then
			return tier - d
		end
		if tier + d < TIERS and counts[tier + d] > 0 then
			return tier + d
		end
	end
	return tier
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

-- ============================ candidate sets ============================
-- The Rust draws over the whole catalog, addressed by 0-based content id. The
-- Roblox build additionally supports a pool restriction, so the draw is
-- expressed over a CANDIDATE SET: `n` entries, candidate position `i` (0-based)
-- mapping to content id `idAt(i)`. Unrestricted, `idAt` is the identity and
-- every step below is byte-for-byte the Rust.

--- `cat` is a bound catalog; `ids` is a pool listing of 0-based content ids, or
--- `nil` for unrestricted. An EMPTY listing is treated as unrestricted: a pool
--- that can serve no id of a kind cannot fill that kind's slots at all, and
--- degrading to the full catalog is strictly better than erroring mid-match.
local function candidates(cat, ids)
	if ids == nil or #ids == 0 then
		return {
			cat = cat,
			n = cat.n,
			idAt = function(i: number): number
				return i
			end,
		}
	end
	return {
		cat = cat,
		n = #ids,
		idAt = function(i: number): number
			-- `i` is a 0-based candidate position; `ids` is a 1-indexed Luau
			-- array of 0-based content ids (§S3).
			return ids[i + 1]
		end,
	}
end

--- `shop::tier_counts` — how many candidate entries of each rarity exist, for
--- one kind. Pure function of the (static) catalog and the pool; recomputed per
--- shop rather than cached so the sim keeps no hidden state. ~180 integer
--- compares per shop — free at this cadence.
--- Returns a table indexed by the 0-based tier.
local function tierCounts(cand)
	local c = {}
	for t = 0, TIERS - 1 do
		c[t] = 0
	end
	local get, idAt = cand.cat.get, cand.idAt
	for i = 0, cand.n - 1 do
		local r = get(idAt(i)).rarity
		if r >= 0 and r < TIERS then
			c[r] += 1
		end
	end
	return c
end

--- `shop::nth_of_tier` — content id of the `k`-th (0-based) candidate entry
--- whose rarity is `tier`, enumerated in candidate order. `k` must be
--- `< counts[tier]`.
local function nthOfTier(cand, tier: number, k: number): number
	local get, idAt = cand.cat.get, cand.idAt
	local seen = 0
	for i = 0, cand.n - 1 do
		local id = idAt(i)
		if get(id).rarity == tier then
			if seen == k then
				return id
			end
			seen += 1
		end
	end
	-- Unreachable for `k < counts[tier]`; fall back to the last candidate rather
	-- than erroring so a malformed catalog degrades instead of killing a match.
	return idAt(math.max(cand.n - 1, 0))
end

-- ============================ generate_offers ============================

--- Generate a fresh set of offers: the first `WEAPON_SLOTS` from the weapon
--- pool, the rest from the modifier pool, each **rarity-weighted by the current
--- round** (see the header for the exact draw sequence). Replaces
--- `s.shop.offers`, increments `s.shop.shop_seq`. Called at the round boundary
--- (`lib.rs::step` phase 1) and on every reroll (`input.rs`).
function Shop.generateOffers(s)
	local c = content()
	local kind = Shop.OfferKind
	local weights = rarityWeights(s.round)
	local wcand = candidates(c.weapons, if pool then pool.weapons else nil)
	local mcand = candidates(c.modifiers, if pool then pool.modifiers else nil)
	local wcounts = tierCounts(wcand)
	local mcounts = tierCounts(mcand)

	local offers = table.create(OFFER_SLOTS)
	-- `slot` is 0-based (mirrors the Rust `0..OFFER_SLOTS`); `offers` is a
	-- 1-indexed Luau array, so it is written at `slot + 1` (SIM-SPEC §S3).
	for slot = 0, OFFER_SLOTS - 1 do
		-- Draw 1 of 2: the tier. Same bound for every slot, every round.
		local tier = drawTier(s.rng_shop, weights)
		local offer
		if slot < WEAPON_SLOTS then
			local t = effectiveTier(wcounts, tier)
			-- Draw 2 of 2: uniform inside the tier.
			local k = Rng.below(s.rng_shop, wcounts[t])
			-- `def` is a 0-based content id, resolved against the FULL catalog.
			local def = nthOfTier(wcand, t, k)
			offer = { kind = kind.Weapon, def = def, cost = wcand.cat.get(def).cost }
		else
			local t = effectiveTier(mcounts, tier)
			local k = Rng.below(s.rng_shop, mcounts[t])
			local def = nthOfTier(mcand, t, k)
			offer = { kind = kind.Modifier, def = def, cost = mcand.cat.get(def).cost }
		end
		offers[slot + 1] = offer
	end
	s.shop.offers = offers
	s.shop.shop_seq += 1
end

return Shop
