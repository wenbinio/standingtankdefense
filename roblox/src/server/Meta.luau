--!strict
--[[
	Meta.luau — the meta-progression RULES (`docs/10` F3, R6).

	`Profile.luau` remembers; this module decides. XP from a run, what a given XP
	total entitles an account to, which content ids may be OFFERED, what a chassis
	is and how it is applied, and which cosmetics exist.

	WHAT F3 ACTUALLY BUYS US, AND WHAT IT DOES NOT
	----------------------------------------------
	F3 carries a correction found during the R3 transcription, and it governs this
	file: there is NO rarity/weighting system to reuse. `Shop.generateOffers`
	draws UNIFORMLY over the full pools into a fixed 4-weapon + 4-modifier layout
	(`sim/Shop.luau`); the rarity ladder, Black Market and copy-effects of
	`docs/02` §2.5/§2.8 are not implemented. So:

	  * pool RESTRICTION is trivial and is what this module does — draw from a
	    subset of ids;
	  * pool WEIGHTING does not exist and is not invented here. Rarity is used
	    only as the ORDER in which content unlocks, never as a draw weight. A
	    weighted draw would be new machinery inside the verified sim and is out of
	    scope for R6.

	NO GACHA, NO PAID POWER (a decision, not a default)
	---------------------------------------------------
	`docs/10` F3 declines the unit-roll / luck-boost model that the $2–5M/month
	Roblox TD games run on, accepting a materially lower revenue ceiling in
	exchange for not shipping a predatory loop to a majority-minor audience.
	Mechanically, in this file, that means:

	  * unlocks are a pure function of XP — there is no roll, no pity, no crate,
	    and nothing that converts money into offer-pool breadth;
	  * `Chassis` has NO price field, by construction. Chassis are the only
	    mechanically-loaded meta content and they are earned with XP only;
	  * `Cosmetic` is the only thing with a price, and no cosmetic touches the
	    sim — nothing in `applyChassis`/`pool` ever reads a cosmetic.

	Reversing this is a `docs/10` F3 decision, not an implementation detail. If a
	price ever needs to appear on a chassis, that is the flag.

	DETERMINISM
	-----------
	Nothing here draws RNG, and `applyChassis` is integer-only and runs strictly
	BEFORE tick 0 (it asserts that). A run is therefore still reproducible from
	`(master_seed, player_id, chassis_id, pool)` — those four are what a replay or
	an anti-cheat re-simulation needs, so the director must record the chassis id
	and the pool alongside the seed.

	PURITY
	------
	No Roblox globals. Requires only `./Profile` and — lazily, injectably — the
	shared content catalog and `sim/State`. Runs under the bare interpreter;
	covered by `test/meta_test.luau`.
]]

local Profile = require("./Profile")

local Meta = {}

export type Kind = string
local WEAPONS: Kind = "weapons"
local MODIFIERS: Kind = "modifiers"
local KINDS = { WEAPONS, MODIFIERS }
Meta.KIND = Profile.KIND

--=============================================================================
-- content / state binding
--=============================================================================
-- Same seam as `sim/Shop.luau` and `sim/Waves.luau`: injectable, with a lazy
-- default require, so a test can bind a trimmed catalog without a global.

local contentModule: any = nil
local stateModule: any = nil
local scheduleCache: any = nil

--- Inject the content catalog (`src/shared/Content.luau`, or any table with the
--- same `WEAPONS`/`MODIFIERS`/`constants` shape). Clears the derived schedule.
function Meta.setContent(c: any)
	contentModule = c
	scheduleCache = nil
end

--- Inject `sim/State.luau`. Only `applyChassis` needs it.
function Meta.setState(s: any)
	stateModule = s
end

local function content(): any
	if contentModule == nil then
		local ok, mod = pcall(require, "../shared/Content")
		if not ok then
			error("Meta: no content — require('../shared/Content') failed (" .. tostring(mod) .. "); call Meta.setContent(Content) first", 3)
		end
		contentModule = mod
	end
	return contentModule
end

local function stateMod(): any
	if stateModule == nil then
		local ok, mod = pcall(require, "../shared/sim/State")
		if not ok then
			error("Meta: no sim State — require('../shared/sim/State') failed (" .. tostring(mod) .. "); call Meta.setState(State) first", 3)
		end
		stateModule = mod
	end
	return stateModule
end

--- The catalog for a kind, as a 1-indexed array of defs (each carrying its own
--- 0-based `id`, `rarity` and `cost`).
local function catalog(kind: Kind): { any }
	local c = content()
	if kind == WEAPONS then
		return c.WEAPONS or c.weapons
	elseif kind == MODIFIERS then
		return c.MODIFIERS or c.modifiers
	end
	error("Meta: unknown kind '" .. tostring(kind) .. "'", 3)
end

--=============================================================================
-- XP
--=============================================================================

--- Every knob that converts a run into XP. Integer arithmetic throughout — not
--- for determinism (this never touches the sim) but so a displayed number and a
--- stored number can never disagree by a float epsilon.
export type XpRates = {
	per_round: number, -- per round COMPLETED
	ticks_per_xp: number, -- 30 ticks = 1 s, so 90 = 1 XP / 3 s survived
	damage_per_xp: number,
	gold_per_xp: number,
	last_stand: number, -- F2's Last Stand award
	completion: number, -- flat, for finishing a run at all
	max_per_run: number, -- hard cap: bounds any single-run exploit
}

local XP_RATES: XpRates = {
	per_round = 40,
	ticks_per_xp = 90,
	damage_per_xp = 25000,
	gold_per_xp = 2000,
	last_stand = 250,
	completion = 25,
	max_per_run = 6000,
}
Meta.XP = XP_RATES

--- One finished run, as the meta layer sees it. Produced by
--- `Meta.runResultFromState`, or assembled by the director.
export type RunResult = {
	rounds: number, -- rounds COMPLETED (0 = died in the first round)
	ticks: number, -- sim ticks survived
	damage: number, -- `ArenaState.total_damage_dealt`
	gold: number, -- `ArenaState.total_gold_earned`
	last_stand: boolean?,
}

local U32_MAX = 4294967295

local function nonNegInt(n: any): number
	if type(n) ~= "number" or n ~= n then
		return 0
	end
	n = math.floor(n)
	if n < 0 then
		return 0
	end
	return n
end

--- Read a finished arena into a `RunResult`. READ-ONLY — it never mutates `s`.
--- `last_stand` comes from the director (F2: only it knows the cohort), not from
--- the arena, which by design knows nothing about other players.
function Meta.runResultFromState(s: any, last_stand: boolean?): RunResult
	local round = s.round
	-- `round` is `U32_MAX` before the first round boundary; being IN round n
	-- means n rounds were completed.
	local rounds = if round == nil or round == U32_MAX then 0 else nonNegInt(round)
	return {
		rounds = rounds,
		ticks = nonNegInt(s.tick),
		damage = nonNegInt(s.total_damage_dealt),
		gold = nonNegInt(s.total_gold_earned),
		last_stand = last_stand == true,
	}
end

--- XP for one run. Deterministic, integer, capped.
function Meta.xpForRun(r: RunResult): number
	local x = XP_RATES
	local xp = x.completion
	xp += nonNegInt(r.rounds) * x.per_round
	xp += nonNegInt(r.ticks) // x.ticks_per_xp
	xp += nonNegInt(r.damage) // x.damage_per_xp
	xp += nonNegInt(r.gold) // x.gold_per_xp
	if r.last_stand then
		xp += x.last_stand
	end
	if xp > x.max_per_run then
		xp = x.max_per_run
	end
	return xp
end

--=============================================================================
-- the unlock schedule
--=============================================================================

--- The pool starts small and widens (F3). The schedule is DERIVED FROM THE
--- CATALOG rather than hand-listed, so adding content to `content.rs` extends
--- progression automatically and no id can be forgotten or duplicated:
---
---   * items are grouped into BANDS by their `rarity` field (0 Common .. 3 Epic,
---     clamped like `Content.rarityScopeId`);
---   * within a band the order is (cost asc, id asc) — stable and total;
---   * band 0's first `starter` entries are free at 0 XP; the rest of band 0
---     unlocks one per `step`; every other band opens at `open` and then
---     unlocks one per `step`. Bands overlap on purpose, so there is always a
---     next unlock in sight.
---
--- Rarity is used HERE and only here — as unlock ORDER. It is never a draw
--- weight; see the header.
export type Band = { open: number, step: number }

local BANDS_T: { [number]: Band } = {
	[0] = { open = 0, step = 250 },
	[1] = { open = 2500, step = 400 },
	[2] = { open = 12000, step = 900 },
	[3] = { open = 30000, step = 2500 },
}
Meta.BANDS = BANDS_T

--- How many band-0 entries are free on a brand-new account, per kind.
local STARTER_T: { [Kind]: number } = { weapons = 8, modifiers = 10 }
Meta.STARTER = STARTER_T

local function bandOf(rarity: any): number
	local r = if type(rarity) == "number" then math.floor(rarity) else 0
	if r < 0 then
		r = 0
	elseif r > 3 then
		r = 3
	end
	return r
end

--- `{ [kind] = { [id] = xp } }`, memoized against the bound catalog.
local function schedule(): { [Kind]: { [number]: number } }
	if scheduleCache ~= nil then
		return scheduleCache
	end
	local out: { [Kind]: { [number]: number } } = {}
	for _, kind in KINDS do
		local defs = catalog(kind)
		local bands: { [number]: { any } } = {}
		for i = 1, #defs do
			local d = defs[i]
			local b = bandOf(d.rarity)
			local list = bands[b]
			if list == nil then
				list = {}
				bands[b] = list
			end
			list[#list + 1] = d
		end
		local xpById: { [number]: number } = {}
		local starter = STARTER_T[kind] or 0
		for b = 0, 3 do
			local list = bands[b]
			if list ~= nil then
				table.sort(list, function(a: any, c: any): boolean
					if a.cost ~= c.cost then
						return a.cost < c.cost
					end
					return a.id < c.id
				end)
				local cfg = BANDS_T[b] or { open = 0, step = 1000 }
				local free = if b == 0 then starter else 0
				for i = 1, #list do
					local id = list[i].id
					if i <= free then
						xpById[id] = 0
					elseif b == 0 then
						xpById[id] = cfg.open + (i - free) * cfg.step
					else
						xpById[id] = cfg.open + (i - 1) * cfg.step
					end
				end
			end
		end
		out[kind] = xpById
	end
	scheduleCache = out
	return out
end

--- The XP at which `id` becomes offerable, or `nil` if the id is not in the
--- catalog.
function Meta.unlockXp(kind: Kind, id: number): number?
	return schedule()[kind][id]
end

--- Every id a `xp` total entitles an account to, ascending. Pure function of the
--- catalog and the schedule — no profile involved.
function Meta.entitledIds(kind: Kind, xp: number): { number }
	local sched = schedule()[kind]
	local out: { number } = {}
	for id, need in sched do
		if xp >= need then
			out[#out + 1] = id
		end
	end
	table.sort(out)
	return out
end

--- The full XP an account needs before nothing is locked. Useful for UI and for
--- sanity-checking a retune.
function Meta.maxUnlockXp(kind: Kind): number
	local top = 0
	for _, need in schedule()[kind] do
		if need > top then
			top = need
		end
	end
	return top
end


--=============================================================================
-- the offer pool  (the F3 deliverable)
--=============================================================================

--- Exactly the shape a pool-aware `Shop` consumes: per kind, an ASCENDING array
--- of 0-based content ids. `pool.weapons[r + 1]` is the def for draw index `r`.
export type Pool = { weapons: { number }, modifiers: { number } }

--- Which content ids this profile may be OFFERED. The union of what its XP
--- entitles it to and what it has stored (a materialized or separately granted
--- unlock is never taken away). Ids outside the current catalog are dropped, so
--- a profile written against a larger catalog cannot inject a bad def.
---
--- The starting weapon (`constants.starting_weapon`) is force-included: it is
--- what `State.new` puts in slot 0, so a pool without it would be incoherent.
function Meta.pool(p: Profile.Profile): Pool
	local sched = schedule()
	local pool: Pool = { weapons = {}, modifiers = {} }
	for _, kind in KINDS do
		local xpById = sched[kind]
		local stored = p.unlocked[kind] :: { [number]: boolean }
		local ids = (pool :: any)[kind] :: { number }
		for id, need in xpById do
			if p.xp >= need or stored[id] then
				ids[#ids + 1] = id
			end
		end
		table.sort(ids)
	end
	local starting = content().constants.starting_weapon
	if type(starting) == "number" and not table.find(pool.weapons, starting) then
		table.insert(pool.weapons, 1, starting)
		table.sort(pool.weapons)
	end
	if #pool.weapons == 0 or #pool.modifiers == 0 then
		error("Meta.pool: empty pool — the unlock schedule or the catalog is broken")
	end
	return pool
end

--- The whole catalog, as a `Pool`. This is the UNRESTRICTED draw — identical to
--- what `Shop.generateOffers` does today, and therefore the pool the R3 oracle
--- path must use.
function Meta.fullPool(): Pool
	local pool: Pool = { weapons = {}, modifiers = {} }
	for _, kind in KINDS do
		local defs = catalog(kind)
		local ids = (pool :: any)[kind] :: { number }
		for i = 1, #defs do
			ids[i] = defs[i].id
		end
		table.sort(ids)
	end
	return pool
end

--- Is `id` offerable to `p`? O(1) — does not build the pool.
function Meta.isOffered(p: Profile.Profile, kind: Kind, id: number): boolean
	local need = schedule()[kind][id]
	if need == nil then
		return false
	end
	return p.xp >= need or Profile.isUnlocked(p, kind, id)
end

--- The def id for draw index `r` (0-based, `r < #pool[kind]`). THIS IS THE
--- WHOLE INTEGRATION: a pool-aware shop draws `r = Rng.below(rng_shop, n)` with
--- `n = #pool[kind]` instead of the catalog size, then maps `r` through here.
--- Exactly 8 draws per `generateOffers` either way — the draw COUNT is
--- unchanged, only its range.
function Meta.defForDraw(pool: Pool, kind: Kind, r: number): number
	local ids = (pool :: any)[kind] :: { number }?
	if ids == nil then
		error("Meta.defForDraw: unknown kind '" .. tostring(kind) .. "'", 2)
	end
	local def = ids[r + 1]
	if def == nil then
		error(string.format("Meta.defForDraw: draw %d out of range for %d %s", r, #ids, kind), 2)
	end
	return def
end

--- Is `pool` the entire catalog? Then restriction is a no-op and the arena can
--- run the stock, oracle-identical draw.
function Meta.isFullPool(pool: Pool): boolean
	for _, kind in KINDS do
		if #((pool :: any)[kind] :: { number }) ~= #catalog(kind) then
			return false
		end
	end
	return true
end

--[[
	Hand `pool` to the shop for one arena.

	`sim/Shop.luau` is VERIFIED CODE (R0–R3, bit-identical to the Rust for 55,200
	ticks) and is not this task's to edit, so this function feature-detects the
	hook rather than assuming it. The hook it wants is three lines inside
	`Shop.generateOffers`:

	    local pool = nil                       -- module upvalue
	    function Shop.setPool(p) pool = p end  -- nil = unrestricted

	    -- inside the slot loop, weapon branch (modifier branch is the mirror):
	    local ids = pool and pool.weapons
	    local n   = if ids then #ids else nw
	    local r   = Rng.below(s.rng_shop, n)
	    local def = if ids then ids[r + 1] else r
	    offer = { kind = kind.Weapon, def = def, cost = weapons.get(def).cost }

	Why it must live there and not out here: `Arena.step` generates the round's
	offers in phase 1 and applies the player's input in phase 2 OF THE SAME TICK,
	so there is no point outside `step` at which a caller could rewrite
	`s.shop.offers` before a `BuyOffer` could read them. Remapping after the step
	would be a race, not an integration.

	Why it is safe: with `pool == nil` the branch collapses to today's code
	exactly — same draw count, same range, same stream — so the R3 oracle gate is
	untouched. Under Roblox each arena is its own `Actor`, hence its own module
	instance, so a per-arena `setPool` does not leak across arenas. Under the bare
	interpreter one process is one arena, which is how the tests run.
]]
function Meta.bindShop(Shop: any, pool: Pool)
	local setPool = (Shop :: any).setPool
	if type(setPool) == "function" then
		(setPool :: (Pool) -> ())(pool)
		return
	end
	if Meta.isFullPool(pool) then
		return -- unrestricted: the stock draw already does exactly this
	end
	error(
		"Meta.bindShop: this Shop has no `setPool` hook, so the offer pool cannot be "
			.. "restricted (Shop writes the draw index straight into `offer.def`). "
			.. "Add the three-line hook documented above `Meta.bindShop`, or run the "
			.. "arena with `Meta.fullPool()`.",
		2
	)
end

--=============================================================================
-- chassis
--=============================================================================

--- A starting loadout: one starting weapon and one passive modifier. Light
--- mechanical variety, strong identity hook (F3 #2).
---
--- THERE IS NO PRICE FIELD, and that is deliberate — chassis are the only
--- mechanically-loaded meta content, so they are XP-earned only. See the header.
export type Chassis = {
	id: string,
	name: string,
	blurb: string,
	starting_weapon: number, -- 0-based weapon id, replaces the default Bow
	passive: number?, -- 0-based modifier id, applied once at tick 0
	unlock_xp: number,
}

--- All passives here are rarity-0 commons: a chassis is meant to angle a run,
--- not to start it ahead. Ids are 0-based content ids (`src/shared/content.json`).
local CHASSIS_T: { Chassis } = {
	{
		id = "ranger",
		name = "Ranger",
		blurb = "The stock tank. Bow, and it fires faster.",
		starting_weapon = 0, -- Bow
		passive = 5, -- Rapidfire (+attack speed)
		unlock_xp = 0,
	},
	{
		id = "bulwark",
		name = "Bulwark",
		blurb = "Heavier hull, slower opener.",
		starting_weapon = 12, -- Boulder
		passive = 57, -- Improved Masonry (+max HP)
		unlock_xp = 4000,
	},
	{
		id = "alchemist",
		name = "Alchemist",
		blurb = "Poison that actually bites.",
		starting_weapon = 3, -- Poison Bow
		passive = 31, -- Potent Poison
		unlock_xp = 10000,
	},
	{
		id = "prospector",
		name = "Prospector",
		blurb = "Starts poor, ends rich.",
		starting_weapon = 1, -- Mortar Launcher
		passive = 73, -- Gold Mine
		unlock_xp = 18000,
	},
	{
		id = "arcanist",
		name = "Arcanist",
		blurb = "Magic damage, straight through armor.",
		starting_weapon = 11, -- Magic Missile
		passive = 3, -- Improved Magic Attacks
		unlock_xp = 28000,
	},
}
Meta.CHASSIS = CHASSIS_T

Meta.DEFAULT_CHASSIS = "ranger"

local chassisById: { [string]: Chassis }? = nil

function Meta.chassis(id: string): Chassis?
	if chassisById == nil then
		local m: { [string]: Chassis } = {}
		for _, c in CHASSIS_T do
			m[c.id] = c
		end
		chassisById = m
	end
	return (chassisById :: { [string]: Chassis })[id]
end

--- Does `p` own `id`? Ownership is stored, but the default chassis and anything
--- the current XP entitles the account to also count, so a profile that has
--- never been synced still behaves correctly.
function Meta.ownsChassis(p: Profile.Profile, id: string): boolean
	local c = Meta.chassis(id)
	if c == nil then
		return false
	end
	return c.unlock_xp <= p.xp or Profile.hasChassis(p, id)
end

function Meta.ownedChassis(p: Profile.Profile): { string }
	local out: { string } = {}
	for _, c in CHASSIS_T do
		if Meta.ownsChassis(p, c.id) then
			out[#out + 1] = c.id
		end
	end
	return out
end

--- The chassis a run for `p` should start with: the selection if it is real and
--- owned, else the default. Never errors — a stale selection silently falls back.
function Meta.selectedChassis(p: Profile.Profile): Chassis
	local id = p.selected_chassis
	if id ~= "" and Meta.ownsChassis(p, id) then
		local c = Meta.chassis(id)
		if c ~= nil then
			return c
		end
	end
	return (Meta.chassis(Meta.DEFAULT_CHASSIS) :: any) :: Chassis
end

--- Record a chassis selection. Unlike `Profile.selectChassis` this also checks
--- the id EXISTS. Returns false (and changes nothing) if unknown or unowned.
function Meta.selectChassis(p: Profile.Profile, id: string): boolean
	if id ~= "" and not Meta.ownsChassis(p, id) then
		return false
	end
	if id ~= "" then
		Profile.grantChassis(p, id) -- materialize, so the selection is stable
	end
	return Profile.selectChassis(p, id)
end

--- Apply a chassis to a FRESH arena state, before the first `Arena.step`.
---
--- Determinism: integer-only, no RNG, no wall clock. It is a pre-tick-0 edit of
--- the initial state — the same class of thing as `State.new` itself — so a run
--- stays reproducible from `(master_seed, player_id, chassis_id)`. Applying it
--- mid-run would silently fork the checksum, so that is refused outright.
---
--- The starting weapon REPLACES the default instance (same `instance_id`, so
--- entity ids are untouched); the passive goes through `State.buyModifier`, the
--- same path a purchase takes, minus the gold.
function Meta.applyChassis(s: any, id: string)
	local c = Meta.chassis(id)
	if c == nil then
		error("Meta.applyChassis: unknown chassis '" .. tostring(id) .. "'", 2)
	end
	if s.tick ~= 0 or s.shop == nil or s.shop.shop_seq ~= 0 then
		error("Meta.applyChassis: must be applied to a fresh arena (tick 0, before the first step)", 2)
	end
	local defs = catalog(WEAPONS)
	if defs[c.starting_weapon + 1] == nil then
		error("Meta.applyChassis: chassis '" .. id .. "' names weapon " .. c.starting_weapon .. ", which is not in the catalog", 2)
	end
	if s.weapons[1] == nil then
		error("Meta.applyChassis: arena has no starting weapon instance to replace", 2)
	end
	s.weapons[1].def = c.starting_weapon
	if c.passive ~= nil then
		if catalog(MODIFIERS)[c.passive + 1] == nil then
			error("Meta.applyChassis: chassis '" .. id .. "' names modifier " .. c.passive .. ", which is not in the catalog", 2)
		end
		stateMod().buyModifier(s, c.passive)
	end
end

--- `applyChassis` with the chassis this profile should be running. Returns the
--- id applied — the director MUST record it next to the seed, or the run is not
--- reproducible.
function Meta.applyProfile(s: any, p: Profile.Profile): string
	local c = Meta.selectedChassis(p)
	Meta.applyChassis(s, c.id)
	return c.id
end

--=============================================================================
-- cosmetics
--=============================================================================

--- The monetization surface, and the ONLY priced content in this module
--- (F3 #3). Nothing here is read by the sim, by `pool`, or by `applyChassis`:
--- a cosmetic cannot affect a run, which is what makes selling it non-predatory.
--- `price` is in Robux; 0 with `default = true` means everyone has it.
export type Cosmetic = {
	id: string,
	slot: string, -- "skin" | "trail" | "arena"
	name: string,
	price: number,
	default: boolean?,
}

Meta.COSMETIC_SLOTS = { "skin", "trail", "arena" }

local COSMETICS_T: { Cosmetic } = {
	{ id = "skin_stock", slot = "skin", name = "Stock Plating", price = 0, default = true },
	{ id = "skin_verdigris", slot = "skin", name = "Verdigris", price = 75 },
	{ id = "skin_obsidian", slot = "skin", name = "Obsidian", price = 150 },
	{ id = "trail_none", slot = "trail", name = "No Trail", price = 0, default = true },
	{ id = "trail_ember", slot = "trail", name = "Ember Trail", price = 50 },
	{ id = "trail_aurora", slot = "trail", name = "Aurora Trail", price = 120 },
	{ id = "arena_classic", slot = "arena", name = "Classic Field", price = 0, default = true },
	{ id = "arena_duskmoor", slot = "arena", name = "Duskmoor", price = 100 },
}
Meta.COSMETICS = COSMETICS_T

local cosmeticById: { [string]: Cosmetic }? = nil

function Meta.cosmetic(id: string): Cosmetic?
	if cosmeticById == nil then
		local m: { [string]: Cosmetic } = {}
		for _, c in COSMETICS_T do
			m[c.id] = c
		end
		cosmeticById = m
	end
	return (cosmeticById :: { [string]: Cosmetic })[id]
end

function Meta.ownsCosmetic(p: Profile.Profile, id: string): boolean
	local c = Meta.cosmetic(id)
	if c == nil then
		return false
	end
	return c.default == true or Profile.hasCosmetic(p, id)
end

function Meta.ownedCosmetics(p: Profile.Profile): { string }
	local out: { string } = {}
	for _, c in COSMETICS_T do
		if Meta.ownsCosmetic(p, c.id) then
			out[#out + 1] = c.id
		end
	end
	return out
end

--- Grant an owned cosmetic (a completed purchase, or a promo). The purchase
--- itself — `MarketplaceService`, receipt processing, idempotency by receipt id
--- — lives in the Roblox glue behind `Profile.Adapter`, never here.
function Meta.grantCosmetic(p: Profile.Profile, id: string): boolean
	if Meta.cosmetic(id) == nil then
		return false
	end
	return Profile.grantCosmetic(p, id)
end

--- Equip an owned cosmetic into its own slot. `""` is not accepted here — clear
--- a slot with `Meta.unequip`.
function Meta.equipCosmetic(p: Profile.Profile, id: string): boolean
	local c = Meta.cosmetic(id)
	if c == nil or not Meta.ownsCosmetic(p, id) then
		return false
	end
	Profile.grantCosmetic(p, id) -- materialize defaults so `equipped` stays valid
	return Profile.equip(p, c.slot, id)
end

function Meta.unequip(p: Profile.Profile, slot: string): boolean
	return Profile.equip(p, slot, "")
end

--- What the client should render for this account: slot -> cosmetic id, with
--- every slot filled (falling back to that slot's default).
function Meta.loadout(p: Profile.Profile): { [string]: string }
	local out: { [string]: string } = {}
	for _, slot in Meta.COSMETIC_SLOTS do
		local id = Profile.equipped(p, slot)
		if id ~= nil and Meta.ownsCosmetic(p, id) then
			out[slot] = id
		else
			for _, c in COSMETICS_T do
				if c.slot == slot and c.default then
					out[slot] = c.id
					break
				end
			end
		end
	end
	return out
end

--- Materialize everything `p` is currently entitled to INTO the profile, so an
--- unlock, once earned, survives a later retune of the schedule. Live games do
--- not take things back. Returns the newly-granted ids per kind (drive the
--- "New!" UI off this), plus newly-granted chassis.
export type SyncResult = { weapons: { number }, modifiers: { number }, chassis: { string } }

function Meta.syncProfile(p: Profile.Profile): SyncResult
	local res: SyncResult = { weapons = {}, modifiers = {}, chassis = {} }
	for _, kind in KINDS do
		local gained = (res :: any)[kind] :: { number }
		for _, id in Meta.entitledIds(kind, p.xp) do
			if Profile.unlock(p, kind, id) then
				gained[#gained + 1] = id
			end
		end
	end
	for _, ch in CHASSIS_T do
		if p.xp >= ch.unlock_xp and Profile.grantChassis(p, ch.id) then
			res.chassis[#res.chassis + 1] = ch.id
		end
	end
	for _, cos in COSMETICS_T do
		if cos.default then
			Profile.grantCosmetic(p, cos.id)
		end
	end
	return res
end

--=============================================================================
-- levels, progress, and the post-run transaction
--=============================================================================

--- A shallow, purely cosmetic level curve over the same XP: level L needs
--- `500 * (L-1) * L / 2` XP. It exists for the UI; nothing gates on it.
local function xpForLevel(level: number): number
	local n = level - 1
	return 250 * n * (level)
end

function Meta.levelForXp(xp: number): number
	local level = 1
	while level < 200 and xp >= xpForLevel(level + 1) do
		level += 1
	end
	return level
end

export type Progress = {
	xp: number,
	level: number,
	xp_into_level: number,
	xp_for_next: number,
	pool_weapons: number,
	pool_modifiers: number,
	catalog_weapons: number,
	catalog_modifiers: number,
	next_unlock_xp: number?, -- XP total at which the pool next widens
}

--- Everything the progression UI needs, in one read.
function Meta.progress(p: Profile.Profile): Progress
	local level = Meta.levelForXp(p.xp)
	local base = xpForLevel(level)
	local nextXp = xpForLevel(level + 1)
	local pool = Meta.pool(p)
	local nextUnlock: number? = nil
	for _, kind in KINDS do
		for _, need in schedule()[kind] do
			if need > p.xp and (nextUnlock == nil or need < (nextUnlock :: number)) then
				nextUnlock = need
			end
		end
	end
	return {
		xp = p.xp,
		level = level,
		xp_into_level = p.xp - base,
		xp_for_next = nextXp - base,
		pool_weapons = #pool.weapons,
		pool_modifiers = #pool.modifiers,
		catalog_weapons = #catalog(WEAPONS),
		catalog_modifiers = #catalog(MODIFIERS),
		next_unlock_xp = nextUnlock,
	}
end

export type Award = {
	xp_gained: number,
	xp_total: number,
	level_before: number,
	level_after: number,
	unlocked: SyncResult,
}

--- The whole end-of-run meta transaction: stats, XP, and the unlocks that XP
--- just bought. Call it INSIDE `Repo:mutate` so the profile is re-read under the
--- DataStore's atomic update rather than written from a stale copy.
function Meta.awardRun(p: Profile.Profile, r: RunResult): Award
	local before = Meta.levelForXp(p.xp)
	local gained = Meta.xpForRun(r)
	Profile.recordRun(p, {
		rounds = r.rounds,
		ticks = r.ticks,
		damage = r.damage,
		gold = r.gold,
		last_stand = r.last_stand,
	})
	local total = Profile.addXp(p, gained)
	local unlocked = Meta.syncProfile(p)
	return {
		xp_gained = gained,
		xp_total = total,
		level_before = before,
		level_after = Meta.levelForXp(total),
		unlocked = unlocked,
	}
end

--=============================================================================
-- self-check
--=============================================================================

--- Assert this module's hand-written tables agree with the bound catalog: every
--- chassis id/weapon/modifier resolves, every catalog entry is on the schedule
--- exactly once, the starting weapon is free, and no chassis is priced.
--- Returns `(ok, { problems })`. Cheap; the tests run it, and so should a
--- server boot.
function Meta.validate(): (boolean, { string })
	local problems: { string } = {}
	local sched = schedule()
	for _, kind in KINDS do
		local defs = catalog(kind)
		local n = 0
		for _ in sched[kind] do
			n += 1
		end
		if n ~= #defs then
			problems[#problems + 1] = string.format("%s: schedule covers %d of %d entries", kind, n, #defs)
		end
		for i = 1, #defs do
			if sched[kind][defs[i].id] == nil then
				problems[#problems + 1] = string.format("%s: id %d is not on the schedule", kind, defs[i].id)
			end
		end
	end
	local starting = content().constants.starting_weapon
	if sched[WEAPONS][starting] ~= 0 then
		problems[#problems + 1] = "the starting weapon (" .. tostring(starting) .. ") is not free at 0 XP"
	end
	local seen: { [string]: boolean } = {}
	for _, c in CHASSIS_T do
		if seen[c.id] then
			problems[#problems + 1] = "duplicate chassis id '" .. c.id .. "'"
		end
		seen[c.id] = true
		if (c :: any).price ~= nil then
			problems[#problems + 1] = "chassis '" .. c.id .. "' has a price — F3 forbids paid power"
		end
		if catalog(WEAPONS)[c.starting_weapon + 1] == nil then
			problems[#problems + 1] = "chassis '" .. c.id .. "' names a missing weapon"
		end
		if c.passive ~= nil and catalog(MODIFIERS)[c.passive + 1] == nil then
			problems[#problems + 1] = "chassis '" .. c.id .. "' names a missing modifier"
		end
	end
	if Meta.chassis(Meta.DEFAULT_CHASSIS) == nil then
		problems[#problems + 1] = "the default chassis does not exist"
	end
	local slots: { [string]: boolean } = {}
	for _, s in Meta.COSMETIC_SLOTS do
		slots[s] = true
	end
	local defaults: { [string]: boolean } = {}
	local seenCos: { [string]: boolean } = {}
	for _, c in COSMETICS_T do
		if seenCos[c.id] then
			problems[#problems + 1] = "duplicate cosmetic id '" .. c.id .. "'"
		end
		seenCos[c.id] = true
		if not slots[c.slot] then
			problems[#problems + 1] = "cosmetic '" .. c.id .. "' has unknown slot '" .. c.slot .. "'"
		end
		if c.default then
			if defaults[c.slot] then
				problems[#problems + 1] = "slot '" .. c.slot .. "' has more than one default"
			end
			defaults[c.slot] = true
			if c.price ~= 0 then
				problems[#problems + 1] = "default cosmetic '" .. c.id .. "' is priced"
			end
		end
	end
	for _, s in Meta.COSMETIC_SLOTS do
		if not defaults[s] then
			problems[#problems + 1] = "slot '" .. s .. "' has no default cosmetic"
		end
	end
	return #problems == 0, problems
end

return Meta
