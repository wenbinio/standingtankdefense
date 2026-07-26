--!strict
--[[
	Profile.luau — the account-persistent player record (`docs/10` F3, R6).

	WHAT THIS IS
	------------
	The *record*, not the rules. Everything here is storage, mutation and
	serialization of one account's persistent state:

	    xp | unlocked weapon ids | unlocked modifier ids | owned chassis
	       | owned + equipped cosmetics | lifetime stats

	Which id unlocks at which XP, which chassis exist, what a run is worth —
	all of that is `Meta.luau`. This module never reads the content catalog and
	never decides an unlock; it only remembers one.

	PURITY (this is load-bearing, `PHASE2-CONTRACTS.md` P6)
	------------------------------------------------------
	Zero requires, zero Roblox globals, no wall clock, no `math.random`. It runs
	under the bare `luau` interpreter, which is how `test/meta_test.luau` covers
	it. DataStore is reached through the ADAPTER SEAM at the bottom of this file
	(`Profile.Adapter` / `Profile.newRepo`): a ~30-line server file implements
	`get`/`update` against `DataStoreService` and hands it in. That file is the
	only thing in the meta layer that may touch a Roblox global.

	SERIALIZATION
	-------------
	`serialize` emits a plain table that survives `DataStore:SetAsync` unchanged:
	string keys only, dense arrays only, finite integers only, no cycles, no
	mixed key tables (Roblox's `JSONEncode` rejects those, and a rejected save is
	a lost account). `Profile.isDataStoreSafe` is that rule as an assertion — the
	repo runs it before every save so a malformed profile fails at the call site
	rather than inside `UpdateAsync`.

	In memory, unlock/ownership sets are SETS (`{[id] = true}`) because the hot
	path is a membership test; on the wire they are SORTED ARRAYS, because that
	is what JSON can carry and because a stable order makes stored blobs diffable.

	CONCURRENCY
	-----------
	`Profile.merge` is a conflict-free reconcile for the `UpdateAsync` callback:
	sets union, monotone counters take the max. Two servers writing the same
	account can therefore never destroy each other's unlocks — the worst case is
	a duplicated run being counted once. Non-monotone fields (the equipped
	cosmetics, the selected chassis) resolve to the newer `rev`.
]]

local Profile = {}

--- Bump ONLY with a matching branch in `migrate`. A stored blob whose `v` is
--- newer than this is refused rather than downgraded (see `tryDeserialize`) —
--- an old server must never overwrite a new server's account.
local SCHEMA_VERSION = 1
Profile.SCHEMA_VERSION = SCHEMA_VERSION

--- XP is clamped here so no arithmetic can push it out of exact-integer range
--- or negative. Far above anything the curve in `Meta.luau` reaches.
local XP_MAX = 1000000000
Profile.XP_MAX = XP_MAX

--- Catalog keys. These strings ARE the pool field names `Meta.pool` returns and
--- the ones a future `Shop.setPool` consumes, so they are shared vocabulary,
--- not private spelling.
export type Kind = string
Profile.KIND = { Weapon = "weapons", Modifier = "modifiers" }

local KINDS = { "weapons", "modifiers" }

--=============================================================================
-- types
--=============================================================================

export type IdSet = { [number]: boolean }
export type NameSet = { [string]: boolean }

--- Lifetime totals. Every field is monotone non-decreasing (`merge` takes the
--- max), which is what makes cross-server reconcile safe.
export type Stats = {
	runs: number, -- runs finished
	rounds_best: number, -- deepest round reached in one run
	rounds_total: number, -- rounds completed, all runs
	ticks_total: number, -- sim ticks survived, all runs (30/s)
	damage_total: number, -- `ArenaState.total_damage_dealt`, summed
	gold_total: number, -- `ArenaState.total_gold_earned`, summed
	last_stands: number, -- F2 Last Stand awards
	xp_total: number, -- XP ever earned (>= `xp`; `xp` is never spent, but this
	-- stays correct if a convenience item ever grants XP)
}

export type Profile = {
	v: number,
	user_id: string,
	rev: number, -- bumped on every local mutation; breaks `merge` ties
	xp: number,
	unlocked: { [Kind]: IdSet }, -- 0-based CONTENT ids (`SIM-SPEC` §S3)
	chassis: NameSet, -- owned chassis ids
	selected_chassis: string, -- "" = use `Meta`'s default
	cosmetics: NameSet, -- owned cosmetic ids
	equipped: { [string]: string }, -- cosmetic slot -> cosmetic id
	stats: Stats,
}

--- What `serialize` emits / `deserialize` accepts. Deliberately untyped in its
--- leaves: it crosses a storage boundary and arrives as whatever was stored.
export type Serialized = { [string]: any }

--=============================================================================
-- construction
--=============================================================================

local function newStats(): Stats
	return {
		runs = 0,
		rounds_best = 0,
		rounds_total = 0,
		ticks_total = 0,
		damage_total = 0,
		gold_total = 0,
		last_stands = 0,
		xp_total = 0,
	}
end

--- A brand-new account. Note what is NOT here: no unlocks are materialized. A
--- fresh profile's offer pool is whatever `Meta` says 0 XP entitles it to.
--- `Meta.syncProfile` writes those ids in on the first award, and from then on
--- they are permanent — retuning the schedule upward can never take back an
--- unlock a live account already had.
function Profile.new(user_id: string): Profile
	return {
		v = SCHEMA_VERSION,
		user_id = user_id,
		rev = 0,
		xp = 0,
		unlocked = { weapons = {}, modifiers = {} },
		chassis = {},
		selected_chassis = "",
		cosmetics = {},
		equipped = {},
		stats = newStats(),
	}
end

local function copySet<K>(t: { [K]: boolean }): { [K]: boolean }
	local out: { [K]: boolean } = {}
	for k, v in t do
		if v then
			out[k] = true
		end
	end
	return out
end

function Profile.clone(p: Profile): Profile
	local stats = newStats()
	for k, v in p.stats :: any do
		(stats :: any)[k] = v
	end
	local equipped: { [string]: string } = {}
	for slot, id in p.equipped do
		equipped[slot] = id
	end
	return {
		v = p.v,
		user_id = p.user_id,
		rev = p.rev,
		xp = p.xp,
		unlocked = { weapons = copySet(p.unlocked.weapons), modifiers = copySet(p.unlocked.modifiers) },
		chassis = copySet(p.chassis),
		selected_chassis = p.selected_chassis,
		cosmetics = copySet(p.cosmetics),
		equipped = equipped,
		stats = stats,
	}
end

--=============================================================================
-- mutation
--=============================================================================

local function touch(p: Profile)
	p.rev += 1
end

local function intClamp(n: number, lo: number, hi: number): number
	if n ~= n then -- NaN
		return lo
	end
	n = math.floor(n)
	if n < lo then
		return lo
	elseif n > hi then
		return hi
	end
	return n
end

--- Add XP. `amount` is floored and clamped at 0 (XP is never spent or lost —
--- there is no currency here, deliberately: see the no-paid-power note in
--- `Meta.luau`). Returns the new total.
function Profile.addXp(p: Profile, amount: number): number
	local add = intClamp(amount, 0, XP_MAX)
	p.xp = intClamp(p.xp + add, 0, XP_MAX)
	p.stats.xp_total = intClamp(p.stats.xp_total + add, 0, XP_MAX)
	touch(p)
	return p.xp
end

--- Record a content id as unlocked. Returns true if this was NEW.
--- `id` is a 0-based content id; the caller (`Meta`) is responsible for it being
--- in range for the current catalog.
function Profile.unlock(p: Profile, kind: Kind, id: number): boolean
	local set = p.unlocked[kind]
	if set == nil then
		error("Profile.unlock: unknown kind '" .. tostring(kind) .. "'", 2)
	end
	if id ~= math.floor(id) or id < 0 then
		error("Profile.unlock: id must be a non-negative integer, got " .. tostring(id), 2)
	end
	if set[id] then
		return false
	end
	set[id] = true
	touch(p)
	return true
end

function Profile.isUnlocked(p: Profile, kind: Kind, id: number): boolean
	local set = p.unlocked[kind]
	return set ~= nil and set[id] == true
end

function Profile.grantChassis(p: Profile, id: string): boolean
	if p.chassis[id] then
		return false
	end
	p.chassis[id] = true
	touch(p)
	return true
end

function Profile.hasChassis(p: Profile, id: string): boolean
	return p.chassis[id] == true
end

--- Select a chassis. Refuses one the account does not own — ownership is the
--- profile's business; whether the id EXISTS is `Meta.selectChassis`'s.
--- `""` clears the selection back to `Meta`'s default.
function Profile.selectChassis(p: Profile, id: string): boolean
	if id ~= "" and not p.chassis[id] then
		return false
	end
	p.selected_chassis = id
	touch(p)
	return true
end

function Profile.grantCosmetic(p: Profile, id: string): boolean
	if p.cosmetics[id] then
		return false
	end
	p.cosmetics[id] = true
	touch(p)
	return true
end

function Profile.hasCosmetic(p: Profile, id: string): boolean
	return p.cosmetics[id] == true
end

--- Equip an owned cosmetic into a slot (`""` clears the slot). Slot validity is
--- `Meta`'s business; ownership is this module's.
function Profile.equip(p: Profile, slot: string, id: string): boolean
	if id ~= "" and not p.cosmetics[id] then
		return false
	end
	if id == "" then
		p.equipped[slot] = nil
	else
		p.equipped[slot] = id
	end
	touch(p)
	return true
end

function Profile.equipped(p: Profile, slot: string): string?
	return p.equipped[slot]
end

--- One finished run's contribution to the lifetime totals. `Meta.awardRun` calls
--- this; it is separate so a caller can replay stats without re-awarding XP.
--- `rounds` is rounds COMPLETED (0 for a first-round death).
export type RunTotals = {
	rounds: number,
	ticks: number,
	damage: number,
	gold: number,
	last_stand: boolean?,
}

function Profile.recordRun(p: Profile, r: RunTotals)
	local st = p.stats
	local rounds = intClamp(r.rounds, 0, XP_MAX)
	st.runs += 1
	st.rounds_total = intClamp(st.rounds_total + rounds, 0, XP_MAX)
	if rounds > st.rounds_best then
		st.rounds_best = rounds
	end
	st.ticks_total = intClamp(st.ticks_total + intClamp(r.ticks, 0, XP_MAX), 0, XP_MAX)
	st.damage_total = intClamp(st.damage_total + intClamp(r.damage, 0, XP_MAX), 0, XP_MAX)
	st.gold_total = intClamp(st.gold_total + intClamp(r.gold, 0, XP_MAX), 0, XP_MAX)
	if r.last_stand then
		st.last_stands += 1
	end
	touch(p)
end

--=============================================================================
-- serialization
--=============================================================================

local function sortedNumbers(set: IdSet): { number }
	local out: { number } = {}
	for id, v in set do
		if v then
			out[#out + 1] = id
		end
	end
	table.sort(out)
	return out
end

local function sortedStrings(set: NameSet): { string }
	local out: { string } = {}
	for id, v in set do
		if v then
			out[#out + 1] = id
		end
	end
	table.sort(out)
	return out
end

--- Profile -> plain DataStore-safe table. Stable output: the same profile always
--- serializes to the same shape, and every array is sorted.
function Profile.serialize(p: Profile): Serialized
	local stats: { [string]: number } = {}
	for k, v in p.stats :: any do
		stats[k] = v
	end
	local equipped: { [string]: string } = {}
	for slot, id in p.equipped do
		equipped[slot] = id
	end
	return {
		v = SCHEMA_VERSION,
		user_id = p.user_id,
		rev = p.rev,
		xp = p.xp,
		unlocked_weapons = sortedNumbers(p.unlocked.weapons),
		unlocked_modifiers = sortedNumbers(p.unlocked.modifiers),
		chassis = sortedStrings(p.chassis),
		selected_chassis = p.selected_chassis,
		cosmetics = sortedStrings(p.cosmetics),
		equipped = equipped,
		stats = stats,
	}
end

local function readNumber(v: any, default: number): number
	if type(v) ~= "number" or v ~= v or v == math.huge or v == -math.huge then
		return default
	end
	return intClamp(v, 0, XP_MAX)
end

local function readString(v: any, default: string): string
	if type(v) ~= "string" then
		return default
	end
	return v
end

local function readIdSet(v: any): IdSet
	local out: IdSet = {}
	if type(v) ~= "table" then
		return out
	end
	for _, id in v :: { any } do
		if type(id) == "number" and id == math.floor(id) and id >= 0 and id < 65536 then
			out[id] = true
		end
	end
	return out
end

local function readNameSet(v: any): NameSet
	local out: NameSet = {}
	if type(v) ~= "table" then
		return out
	end
	for _, id in v :: { any } do
		if type(id) == "string" and id ~= "" then
			out[id] = true
		end
	end
	return out
end

--- Storage -> profile. TOLERANT by construction: a corrupt or partially written
--- blob yields a valid profile with defaults for the broken fields, because the
--- alternative — erroring on load — locks a player out of the game. The one
--- thing it refuses is a blob written by a NEWER schema (see `SCHEMA_VERSION`).
--- Returns `nil, reason` in that case.
function Profile.tryDeserialize(raw: any, user_id: string?): (Profile?, string?)
	if type(raw) ~= "table" then
		return nil, "not a table"
	end
	local t = raw :: { [string]: any }
	local v = readNumber(t.v, SCHEMA_VERSION)
	if v > SCHEMA_VERSION then
		return nil, string.format("stored schema v%d is newer than this server's v%d", v, SCHEMA_VERSION)
	end
	local p = Profile.new(readString(t.user_id, user_id or ""))
	if user_id ~= nil then
		-- The key the blob was loaded under always wins over the one inside it.
		p.user_id = user_id
	end
	p.rev = readNumber(t.rev, 0)
	p.xp = readNumber(t.xp, 0)
	p.unlocked.weapons = readIdSet(t.unlocked_weapons)
	p.unlocked.modifiers = readIdSet(t.unlocked_modifiers)
	p.chassis = readNameSet(t.chassis)
	p.cosmetics = readNameSet(t.cosmetics)
	p.selected_chassis = readString(t.selected_chassis, "")
	if p.selected_chassis ~= "" and not p.chassis[p.selected_chassis] then
		-- A chassis that is no longer owned (revoked, or a corrupt blob) must not
		-- silently keep applying.
		p.selected_chassis = ""
	end
	if type(t.equipped) == "table" then
		for slot, id in t.equipped :: { [any]: any } do
			if type(slot) == "string" and type(id) == "string" and p.cosmetics[id] then
				p.equipped[slot] = id
			end
		end
	end
	if type(t.stats) == "table" then
		local src = t.stats :: { [string]: any }
		for k in p.stats :: any do
			(p.stats :: any)[k] = readNumber(src[k], 0)
		end
	end
	return p, nil
end

--- `tryDeserialize`, erroring instead of returning a reason.
function Profile.deserialize(raw: any, user_id: string?): Profile
	local p, err = Profile.tryDeserialize(raw, user_id)
	if p == nil then
		error("Profile.deserialize: " .. tostring(err), 2)
	end
	return p
end

--- Is `t` something Roblox's `JSONEncode` (and therefore DataStore) will accept?
--- Rejects: cycles, mixed/sparse tables, non-string non-array keys, NaN/inf,
--- functions/userdata, and anything nested more than `MAX_DEPTH` deep.
--- Returns `(ok, reason)`.
local MAX_DEPTH = 8
function Profile.isDataStoreSafe(t: any, _depth: number?, _seen: { [any]: boolean }?): (boolean, string?)
	local depth = _depth or 1
	local seen = _seen or {}
	if depth > MAX_DEPTH then
		return false, "nested deeper than " .. MAX_DEPTH
	end
	if type(t) ~= "table" then
		return false, "not a table"
	end
	if seen[t] then
		return false, "cyclic reference"
	end
	seen[t] = true

	local nStr, nNum, maxIdx = 0, 0, 0
	for k, v in t :: { [any]: any } do
		local kt = type(k)
		if kt == "string" then
			nStr += 1
		elseif kt == "number" then
			if k ~= math.floor(k) or k < 1 then
				return false, "numeric key " .. tostring(k) .. " is not a positive integer"
			end
			nNum += 1
			if k > maxIdx then
				maxIdx = k
			end
		else
			return false, "key of type " .. kt
		end
		local vt = type(v)
		if vt == "number" then
			if v ~= v or v == math.huge or v == -math.huge then
				return false, "non-finite number at key " .. tostring(k)
			end
		elseif vt == "table" then
			local ok, why = Profile.isDataStoreSafe(v, depth + 1, seen)
			if not ok then
				return false, why
			end
		elseif vt ~= "string" and vt ~= "boolean" then
			return false, "value of type " .. vt .. " at key " .. tostring(k)
		end
	end
	if nStr > 0 and nNum > 0 then
		return false, "mixed string and numeric keys"
	end
	if nNum > 0 and maxIdx ~= nNum then
		return false, "sparse array (max index " .. maxIdx .. ", " .. nNum .. " entries)"
	end
	seen[t] = nil
	return true, nil
end

--=============================================================================
-- reconcile
--=============================================================================

local function unionInto(dst: { [any]: boolean }, src: { [any]: boolean })
	for k, v in src do
		if v then
			dst[k] = true
		end
	end
end

--- Conflict-free merge of two views of the SAME account, for the `UpdateAsync`
--- transform: `merge(stored, mine)`. Sets union; every counter takes the max;
--- the newer `rev` wins the two fields that are genuinely last-write-wins.
--- Returns a NEW profile — neither input is mutated.
function Profile.merge(a: Profile, b: Profile): Profile
	local out = Profile.clone(a)
	for _, kind in KINDS do
		unionInto(out.unlocked[kind] :: any, b.unlocked[kind] :: any)
	end
	unionInto(out.chassis :: any, b.chassis :: any)
	unionInto(out.cosmetics :: any, b.cosmetics :: any)
	if b.xp > out.xp then
		out.xp = b.xp
	end
	for k, v in b.stats :: any do
		if type(v) == "number" and v > ((out.stats :: any)[k] or 0) then
			(out.stats :: any)[k] = v
		end
	end
	if b.rev > out.rev then
		out.rev = b.rev
		out.selected_chassis = b.selected_chassis
		out.equipped = {}
		for slot, id in b.equipped do
			out.equipped[slot] = id
		end
	end
	if out.selected_chassis ~= "" and not out.chassis[out.selected_chassis] then
		out.selected_chassis = ""
	end
	if out.user_id == "" then
		out.user_id = b.user_id
	end
	return out
end

--=============================================================================
-- adapter seam  (the ONLY place Roblox is allowed to enter the meta layer)
--=============================================================================

--- What a storage backend must provide. The Roblox implementation is a thin
--- server file (not this one — this one must stay require-free and testable):
---
---     local store = DataStoreService:GetDataStore("Profiles_v1")
---     local adapter = {
---         get = function(key) return store:GetAsync(key) end,
---         update = function(key, transform) return store:UpdateAsync(key, transform) end,
---     }
---     local repo = Profile.newRepo(adapter)
---
--- `update` MUST be atomic read-modify-write (that is exactly `UpdateAsync`) and
--- MUST hand the transform the currently stored raw value (or `nil`). Both
--- functions may yield; nothing in this module does.
export type Adapter = {
	get: (key: string) -> any,
	update: (key: string, transform: (any) -> any) -> any,
}

export type Repo = {
	adapter: Adapter,
	key: (self: Repo, user_id: string) -> string,
	load: (self: Repo, user_id: string) -> Profile,
	save: (self: Repo, p: Profile) -> Profile,
	mutate: (self: Repo, user_id: string, fn: (Profile) -> ()) -> Profile,
}

local Repo = {}
Repo.__index = Repo

function Repo.key(_self: Repo, user_id: string): string
	return "u_" .. user_id
end

--- Load, or a fresh profile if the account has never been stored. Never errors
--- on corrupt data (`tryDeserialize` is tolerant); DOES propagate a
--- newer-schema refusal, because overwriting a newer profile is data loss.
function Repo.load(self: Repo, user_id: string): Profile
	local raw = self.adapter.get(self:key(user_id))
	if raw == nil then
		return Profile.new(user_id)
	end
	local p, err = Profile.tryDeserialize(raw, user_id)
	if p == nil then
		error("Profile.Repo:load(" .. user_id .. "): " .. tostring(err), 2)
	end
	return p
end

--- Write `p`, merging against whatever is stored (another server may have
--- written since we loaded). Returns the merged profile that was actually
--- stored, which the caller should adopt as its in-memory copy.
function Repo.save(self: Repo, p: Profile): Profile
	local merged: Profile = p
	self.adapter.update(self:key(p.user_id), function(raw: any): any
		if raw ~= nil then
			local stored = Profile.tryDeserialize(raw, p.user_id)
			if stored ~= nil then
				merged = Profile.merge(stored, p)
			end
		end
		local payload = Profile.serialize(merged)
		local ok, why = Profile.isDataStoreSafe(payload)
		if not ok then
			error("Profile.Repo:save: refusing to store an unsafe payload: " .. tostring(why))
		end
		return payload
	end)
	return merged
end

--- Read-modify-write in one atomic transaction: the safe way to award XP, since
--- the profile is re-read inside the transform.
function Repo.mutate(self: Repo, user_id: string, fn: (Profile) -> ()): Profile
	local result: Profile = Profile.new(user_id)
	self.adapter.update(self:key(user_id), function(raw: any): any
		local p: Profile
		if raw == nil then
			p = Profile.new(user_id)
		else
			local loaded, err = Profile.tryDeserialize(raw, user_id)
			if loaded == nil then
				error("Profile.Repo:mutate(" .. user_id .. "): " .. tostring(err))
			end
			p = loaded :: Profile
		end
		fn(p)
		result = p
		local payload = Profile.serialize(p)
		local ok, why = Profile.isDataStoreSafe(payload)
		if not ok then
			error("Profile.Repo:mutate: refusing to store an unsafe payload: " .. tostring(why))
		end
		return payload
	end)
	return result
end

function Profile.newRepo(adapter: Adapter): Repo
	return (setmetatable({ adapter = adapter }, Repo) :: any) :: Repo
end

--- An in-memory `Adapter`: for headless tests, for Studio play without a
--- published place, and as the fallback when `DataStoreService` is unavailable
--- (in which case progression is session-local and the shell should SAY SO
--- rather than silently discard the player's run).
function Profile.memoryAdapter(): Adapter
	local store: { [string]: any } = {}
	return {
		get = function(key: string): any
			return store[key]
		end,
		update = function(key: string, transform: (any) -> any): any
			local next_ = transform(store[key])
			store[key] = next_
			return next_
		end,
	}
end

return Profile
