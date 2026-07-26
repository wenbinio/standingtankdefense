--!strict
--[[
	Wire.luau — the arena replication codec (`PHASE2-CONTRACTS.md` P4).

	Packs a `View.snapshot(state)` `RenderView` into a `buffer` for transmission
	over `UnreliableRemoteEvent`, and unpacks it on the client. The server
	simulates every arena (`docs/10` F6) and each client renders exactly one
	view, so this is a **server -> client** channel: nothing decoded here is ever
	authoritative for the sim (P5). Decoding is nevertheless fully bounds-checked
	and errors loudly rather than producing a half-built view.

	PURITY. No Roblox global is referenced — no `game`, `workspace`, `task`,
	`Instance`, and no `require` of anything at all. The module runs unchanged
	under the standalone `luau` interpreter, which is how `test/wire_test.luau`
	exercises it. The `RemoteEvent` glue that actually pushes these buffers lives
	in server/client files that are not this one.

	WHY A DELTA CODEC.
	`UnreliableRemoteEvent` caps at ~900 bytes/payload (dropped above 1000) and a
	naive table of Luau numbers costs ~9 bytes per number. The measured peak is
	~67 enemies / ~84 projectiles / 20 weapons (`docs/09 §1.6`, `docs/10 §F8`) at
	10-15 Hz. A full keyframe of that arena does not fit one payload; a delta
	against the previous view does, several times over, because between two
	15 Hz frames an entity moves a handful of integer units and everything else
	is bit-identical. So:

	  * `packView`  — KEYFRAME. Self-contained. For a joining or desynced client.
	  * `packDelta` — DELTA against the view the client already holds, identified
	                  by its `tick`. Entities are matched by their stable `id`;
	                  spawn / move / despawn all fall out of that.
	  * `chunk` / `Reassembler` — framing so a message larger than one payload
	                  (any keyframe, in practice) survives the transport and a
	                  dropped fragment is *detected* rather than silently
	                  producing a corrupt view.

	INTEGER-ONLY. `View` has already run every world coordinate through
	`Fixed.floorToInt`, and every other field is an i64 carried as a plain Luau
	number (`SIM-SPEC` §S4). The wire keeps that property: it encodes integers
	only and raises on a non-integer, so a float leaking into the render path is
	a loud failure here instead of a silent rounding difference on one client.

	NUMBER ENCODINGS
	  uvarint   LEB128, 7 bits/byte, little-endian groups. 0 .. 2^53-1.
	  svarint   zigzag (n >= 0 -> 2n, n < 0 -> -2n-1) then uvarint.
	  Counts, lengths and table indices are uvarint; every *value* field is
	  svarint, so a negative anywhere in a view is a size cost and never a bug.

	VIEW MESSAGE LAYOUT
	  u8       MAGIC 0x53
	  u8       VERSION
	  u8       kind            0 = KEYFRAME, 1 = DELTA
	  uvarint  tick            the tick of the view being described
	  uvarint  baseTick        DELTA only: prev.tick. Mismatch => refuse to apply.
	  u8       sectionMask     which scalar sections are present (KEYFRAME: all)
	  ...      archetype table (enemy kind/name/base_hp/boss for records that need it)
	  ...      enemies, projectiles, minions   (always present)
	  ...      scalars, tank, economy, shop, arsenal, stats  (per sectionMask)

	A KEYFRAME is exactly a DELTA against an all-empty view with every section
	forced present — one encoder, one decoder, so the two paths cannot drift.

	CHUNK LAYOUT (6-byte header, payload <= maxChunkBytes - 6)
	  u8   MAGIC 0x43
	  u8   VERSION
	  u16  msgId       little-endian, wraps at 65536
	  u8   index       0-based
	  u8   count       total chunks in this message (1..255)
]]

local Wire = {}

-- ============================ constants ============================

Wire.VERSION = 1
Wire.MAGIC = 0x53 -- 'S' — view message
Wire.CHUNK_MAGIC = 0x43 -- 'C' — chunk frame

--- Target payload size. `UnreliableRemoteEvent` drops above ~1000 bytes; 900
--- is the documented safe ceiling (`PHASE2-CONTRACTS.md` P4).
Wire.MAX_CHUNK_BYTES = 900
--- The size at which the transport actually drops the packet. Asserted against.
Wire.HARD_CHUNK_LIMIT = 1000
Wire.CHUNK_HEADER_BYTES = 6

Wire.KIND_KEYFRAME = 0
Wire.KIND_DELTA = 1

-- Section mask bits (scalar / low-rate sections; entity arrays are always sent).
local SEC_SCALARS = 1 -- round, dead
local SEC_TANK = 2
local SEC_ECONOMY = 4
local SEC_SHOP = 8
local SEC_ARSENAL = 16
local SEC_STATS = 32
local SEC_ALL = 63

-- Per-entity control byte.
local XY_SAME = 0 -- position identical to the matched previous entity
local XY_NIB = 1 -- dx,dy both in [-8,7]; one packed byte
local XY_VAR = 2 -- dx,dy as svarints
local XY_ABS = 3 -- absolute x,y as svarints

local F_HP = 4 -- bit 2: hp field present
local F_NEW = 8 -- bit 3: entity not in prev — archetype + absolute fields follow
local F_ARCH = 16 -- bit 4: archetype changed for an entity that already existed
local F_KIND = 4 -- minions reuse bit 2 for "kind changed"
-- bits 5-7 of the control byte carry the small id step; 7 is the escape code.
local ID_ESCAPE = 7

-- Decode sanity caps. Nothing legitimate comes close; they exist so a corrupt
-- or hostile buffer cannot make the decoder allocate unboundedly.
local MAX_ENTITIES = 65535
local MAX_STRING_BYTES = 1024
local MAX_TABLE_ENTRIES = 4096

-- ============================ view types ============================
-- These MIRROR `src/shared/sim/View.luau` exactly. They are re-declared rather
-- than required so this module has zero dependencies (requiring `View` would
-- drag in `Content` and parse `content.json` just to obtain type names).
-- `test/wire_test.luau` asserts the two definitions stay structurally
-- assignable in both directions, so drift is a analyzer failure, not a runtime
-- surprise.

export type RenderPoint = { x: number, y: number }

export type RenderTank = {
	x: number,
	y: number,
	hp: number,
	max_hp: number,
	revives: number,
}

export type RenderMinion = { x: number, y: number, kind: number }

export type RenderEnemy = {
	id: number,
	x: number,
	y: number,
	hp: number,
	base_hp: number,
	kind: number,
	boss: boolean,
	name: string,
}

export type RenderEconomy = {
	gold: number,
	income_per_tick: number,
	rerolls_remaining: number,
	reroll_cost: number,
}

export type RenderOffer = {
	slot: number,
	name: string,
	cost: number,
	is_weapon: boolean,
	affordable: boolean,
	rarity: number,
}

export type RenderArsenalEntry = { name: string, count: number }

export type RenderStats = {
	damage_dealt: number,
	gold_earned: number,
	bought_attack_mask: number,
	weapons_bought: number,
	economy_purchases: number,
}

export type RenderView = {
	tick: number,
	round: number,
	dead: boolean,
	tank: RenderTank,
	enemies: { RenderEnemy },
	projectiles: { RenderPoint },
	minions: { RenderMinion },
	economy: RenderEconomy,
	shop: { RenderOffer },
	arsenal: { RenderArsenalEntry },
	stats: RenderStats,
}

export type PackOpts = {
	--- Initial scratch buffer size, in bytes. Purely an allocation hint.
	capacity: number?,
}

-- ============================ integer guard ============================

local function int(v: number, what: string): number
	if type(v) ~= "number" or v ~= v or v == math.huge or v == -math.huge or v % 1 ~= 0 then
		error(`Wire: {what} must be a finite integer, got {tostring(v)}`, 0)
	end
	return v
end

-- `table.create` is typed `{unknown}` and Luau table types are invariant, so a
-- pre-sized array needs a cast at each use site. This wrapper keeps the cast to
-- one readable `:: { T }` instead of a double cast everywhere.
local function tcreate(n: number): any
	return table.create(n)
end

-- ============================ writer ============================

type Writer = { buf: buffer, len: number }

local function wNew(capacity: number?): Writer
	return { buf = buffer.create(math.max(capacity or 512, 16)), len = 0 }
end

local function wGrow(w: Writer, need: number)
	local cap = buffer.len(w.buf)
	local want = w.len + need
	if want <= cap then
		return
	end
	while cap < want do
		cap *= 2
	end
	local nb = buffer.create(cap)
	buffer.copy(nb, 0, w.buf, 0, w.len)
	w.buf = nb
end

local function wU8(w: Writer, v: number)
	wGrow(w, 1)
	buffer.writeu8(w.buf, w.len, v)
	w.len += 1
end

--- LEB128. `v` must be a non-negative integer below 2^53.
local function wUVar(w: Writer, v: number, what: string)
	int(v, what)
	if v < 0 then
		error(`Wire: {what} must be non-negative, got {v}`, 0)
	end
	if v > 9007199254740991 then
		error(`Wire: {what} exceeds the 2^53-1 wire range`, 0)
	end
	while v >= 128 do
		wU8(w, v % 128 + 128)
		v = math.floor(v / 128)
	end
	wU8(w, v)
end

--- Zigzag + LEB128.
local function wSVar(w: Writer, v: number, what: string)
	int(v, what)
	if v >= 0 then
		if v > 4503599627370495 then
			error(`Wire: {what} exceeds the signed wire range`, 0)
		end
		wUVar(w, v * 2, what)
	else
		if v < -4503599627370496 then
			error(`Wire: {what} exceeds the signed wire range`, 0)
		end
		wUVar(w, -v * 2 - 1, what)
	end
end

local function wStr(w: Writer, s: string, what: string)
	if type(s) ~= "string" then
		error(`Wire: {what} must be a string`, 0)
	end
	local n = #s
	if n > MAX_STRING_BYTES then
		error(`Wire: {what} is {n} bytes, over the {MAX_STRING_BYTES}-byte cap`, 0)
	end
	wUVar(w, n, what .. ".len")
	if n > 0 then
		wGrow(w, n)
		buffer.writestring(w.buf, w.len, s)
		w.len += n
	end
end

local function wFinish(w: Writer): buffer
	local out = buffer.create(w.len)
	buffer.copy(out, 0, w.buf, 0, w.len)
	return out
end

-- ============================ reader ============================

type Reader = { buf: buffer, pos: number, len: number }

local function rNew(b: buffer): Reader
	return { buf = b, pos = 0, len = buffer.len(b) }
end

local function rNeed(r: Reader, n: number, what: string)
	if r.pos + n > r.len then
		error(`Wire: truncated buffer reading {what} (need {n} at {r.pos}, have {r.len})`, 0)
	end
end

local function rU8(r: Reader, what: string): number
	rNeed(r, 1, what)
	local v = buffer.readu8(r.buf, r.pos)
	r.pos += 1
	return v
end

local function rUVar(r: Reader, what: string): number
	local result = 0
	local scale = 1
	for _ = 1, 8 do
		local byte = rU8(r, what)
		result += (byte % 128) * scale
		if byte < 128 then
			return result
		end
		scale *= 128
	end
	error(`Wire: malformed varint reading {what}`, 0)
end

local function rSVar(r: Reader, what: string): number
	local u = rUVar(r, what)
	if u % 2 == 0 then
		return u / 2
	end
	return -(u + 1) / 2
end

local function rCount(r: Reader, what: string, cap: number): number
	local n = rUVar(r, what)
	if n > cap then
		error(`Wire: {what} count {n} exceeds cap {cap}`, 0)
	end
	return n
end

local function rStr(r: Reader, what: string): string
	local n = rCount(r, what .. ".len", MAX_STRING_BYTES)
	if n == 0 then
		return ""
	end
	rNeed(r, n, what)
	local s = buffer.readstring(r.buf, r.pos, n)
	r.pos += n
	return s
end

-- ============================ empty view ============================

--- The base a KEYFRAME is encoded against. Never handed to a caller.
local function emptyView(): RenderView
	return {
		tick = 0,
		round = 0,
		dead = false,
		tank = { x = 0, y = 0, hp = 0, max_hp = 0, revives = 0 },
		enemies = {},
		projectiles = {},
		minions = {},
		economy = { gold = 0, income_per_tick = 0, rerolls_remaining = 0, reroll_cost = 0 },
		shop = {},
		arsenal = {},
		stats = {
			damage_dealt = 0,
			gold_earned = 0,
			bought_attack_mask = 0,
			weapons_bought = 0,
			economy_purchases = 0,
		},
	}
end

-- ============================ section comparison ============================

local function tankEq(a: RenderTank, b: RenderTank): boolean
	return a.x == b.x and a.y == b.y and a.hp == b.hp and a.max_hp == b.max_hp and a.revives == b.revives
end

local function economyEq(a: RenderEconomy, b: RenderEconomy): boolean
	return a.gold == b.gold
		and a.income_per_tick == b.income_per_tick
		and a.rerolls_remaining == b.rerolls_remaining
		and a.reroll_cost == b.reroll_cost
end

local function shopEq(a: { RenderOffer }, b: { RenderOffer }): boolean
	if #a ~= #b then
		return false
	end
	for i = 1, #a do
		local x, y = a[i], b[i]
		if
			x.slot ~= y.slot
			or x.name ~= y.name
			or x.cost ~= y.cost
			or x.is_weapon ~= y.is_weapon
			or x.affordable ~= y.affordable
			or x.rarity ~= y.rarity
		then
			return false
		end
	end
	return true
end

local function arsenalEq(a: { RenderArsenalEntry }, b: { RenderArsenalEntry }): boolean
	if #a ~= #b then
		return false
	end
	for i = 1, #a do
		if a[i].name ~= b[i].name or a[i].count ~= b[i].count then
			return false
		end
	end
	return true
end

local function statsEq(a: RenderStats, b: RenderStats): boolean
	return a.damage_dealt == b.damage_dealt
		and a.gold_earned == b.gold_earned
		and a.bought_attack_mask == b.bought_attack_mask
		and a.weapons_bought == b.weapons_bought
		and a.economy_purchases == b.economy_purchases
end

-- ============================ section copy ============================
-- Sections absent from a delta are carried forward, deep-copied so the applied
-- view never aliases the one it was based on (a renderer that mutates its view
-- must not corrupt the server's delta base).

local function copyTank(t: RenderTank): RenderTank
	return { x = t.x, y = t.y, hp = t.hp, max_hp = t.max_hp, revives = t.revives }
end

local function copyEconomy(e: RenderEconomy): RenderEconomy
	return {
		gold = e.gold,
		income_per_tick = e.income_per_tick,
		rerolls_remaining = e.rerolls_remaining,
		reroll_cost = e.reroll_cost,
	}
end

local function copyShop(s: { RenderOffer }): { RenderOffer }
	local out = tcreate(#s) :: { RenderOffer }
	for i = 1, #s do
		local o = s[i]
		out[i] = {
			slot = o.slot,
			name = o.name,
			cost = o.cost,
			is_weapon = o.is_weapon,
			affordable = o.affordable,
			rarity = o.rarity,
		}
	end
	return out
end

local function copyArsenal(a: { RenderArsenalEntry }): { RenderArsenalEntry }
	local out = tcreate(#a) :: { RenderArsenalEntry }
	for i = 1, #a do
		out[i] = { name = a[i].name, count = a[i].count }
	end
	return out
end

local function copyStats(s: RenderStats): RenderStats
	return {
		damage_dealt = s.damage_dealt,
		gold_earned = s.gold_earned,
		bought_attack_mask = s.bought_attack_mask,
		weapons_bought = s.weapons_bought,
		economy_purchases = s.economy_purchases,
	}
end

-- ============================ position coding ============================

local function nibFits(d: number): boolean
	return d >= -8 and d <= 7
end

--- Choose the cheapest XY mode for one entity and write nothing yet.
local function xyMode(px: number?, py: number?, x: number, y: number): number
	if px == nil then
		return XY_ABS
	end
	local dx, dy = x - (px :: number), y - (py :: number)
	if dx == 0 and dy == 0 then
		return XY_SAME
	end
	if nibFits(dx) and nibFits(dy) then
		return XY_NIB
	end
	-- An svarint pair costs >= 2 bytes; absolute costs >= 2 as well but is
	-- immune to a huge delta, so prefer whichever encodes smaller. Coordinates
	-- live inside roughly +/-1800 units, so both are 1-2 bytes; the delta wins
	-- whenever it is small, which is the common case.
	return XY_VAR
end

local function wXY(w: Writer, mode: number, px: number?, py: number?, x: number, y: number, what: string)
	if mode == XY_SAME then
		return
	elseif mode == XY_NIB then
		local dx, dy = x - (px :: number), y - (py :: number)
		wU8(w, (dx + 8) + (dy + 8) * 16)
	elseif mode == XY_VAR then
		wSVar(w, x - (px :: number), what .. ".dx")
		wSVar(w, y - (py :: number), what .. ".dy")
	else
		wSVar(w, x, what .. ".x")
		wSVar(w, y, what .. ".y")
	end
end

local function rXY(r: Reader, mode: number, px: number, py: number, what: string): (number, number)
	if mode == XY_SAME then
		return px, py
	elseif mode == XY_NIB then
		local b = rU8(r, what .. ".nib")
		local dx = b % 16 - 8
		local dy = math.floor(b / 16) - 8
		return px + dx, py + dy
	elseif mode == XY_VAR then
		local dx = rSVar(r, what .. ".dx")
		local dy = rSVar(r, what .. ".dy")
		return px + dx, py + dy
	end
	local x = rSVar(r, what .. ".x")
	local y = rSVar(r, what .. ".y")
	return x, y
end

-- ============================ enemy archetypes ============================
-- `name`, `base_hp`, `boss` and `kind` are catalog-derived and constant for the
-- life of an enemy, so they are interned once per message and referenced by
-- index. Only enemies that are NEW in this message (or whose archetype somehow
-- changed) pay for the reference; a persisting enemy inherits all four fields
-- from the view the client already holds. That is what gets a moving enemy down
-- to three bytes.

type Archetype = { kind: number, name: string, base_hp: number, boss: boolean }

local function archKey(kind: number, name: string, base_hp: number, boss: boolean): string
	return `{kind}\0{base_hp}\0{boss and 1 or 0}\0{name}`
end

-- ============================ encode ============================

local function encodeArchetypes(w: Writer, list: { Archetype })
	wUVar(w, #list, "archetypes")
	for i = 1, #list do
		local a = list[i]
		wSVar(w, a.kind, "archetype.kind")
		wSVar(w, a.base_hp, "archetype.base_hp")
		wU8(w, a.boss and 1 or 0)
		wStr(w, a.name, "archetype.name")
	end
end

local function decodeArchetypes(r: Reader): { Archetype }
	local n = rCount(r, "archetypes", MAX_TABLE_ENTRIES)
	local out = tcreate(n) :: { Archetype }
	for i = 1, n do
		local kind = rSVar(r, "archetype.kind")
		local base_hp = rSVar(r, "archetype.base_hp")
		local boss = rU8(r, "archetype.boss") ~= 0
		local name = rStr(r, "archetype.name")
		out[i] = { kind = kind, name = name, base_hp = base_hp, boss = boss }
	end
	return out
end

local function encodeEnemies(
	w: Writer,
	prev: { RenderEnemy },
	next_: { RenderEnemy },
	archIndex: { [string]: number }
)
	local byId: { [number]: RenderEnemy } = {}
	for i = 1, #prev do
		byId[prev[i].id] = prev[i]
	end

	wUVar(w, #next_, "enemies")
	local lastId = -1
	for i = 1, #next_ do
		local e = next_[i]
		local id = int(e.id, "enemy.id")
		local p = byId[id]

		local isNew = p == nil
		local archChanged = not isNew
			and (p.kind ~= e.kind or p.name ~= e.name or p.base_hp ~= e.base_hp or p.boss ~= e.boss)

		local mode: number
		local hpPresent: boolean
		if isNew then
			mode = XY_ABS
			hpPresent = true
		else
			mode = xyMode(p.x, p.y, int(e.x, "enemy.x"), int(e.y, "enemy.y"))
			hpPresent = p.hp ~= e.hp
		end

		local step = id - lastId - 1
		local ctrl = mode
		if hpPresent then
			ctrl += F_HP
		end
		if isNew then
			ctrl += F_NEW
		elseif archChanged then
			ctrl += F_ARCH
		end
		if step >= 0 and step < ID_ESCAPE then
			ctrl += step * 32
		else
			ctrl += ID_ESCAPE * 32
		end
		wU8(w, ctrl)
		if step < 0 or step >= ID_ESCAPE then
			wSVar(w, step, "enemy.idStep")
		end

		if isNew or archChanged then
			local key = archKey(e.kind, e.name, e.base_hp, e.boss)
			local idx = archIndex[key]
			if idx == nil then
				error("Wire: internal — enemy archetype missing from the message table", 0)
			end
			wUVar(w, idx - 1, "enemy.arch")
		end

		wXY(w, mode, p and p.x, p and p.y, int(e.x, "enemy.x"), int(e.y, "enemy.y"), "enemy")

		if hpPresent then
			if isNew then
				wSVar(w, e.hp, "enemy.hp")
			else
				wSVar(w, e.hp - p.hp, "enemy.dhp")
			end
		end

		lastId = id
	end
end

local function decodeEnemies(r: Reader, prev: { RenderEnemy }, archetypes: { Archetype }): { RenderEnemy }
	local byId: { [number]: RenderEnemy } = {}
	for i = 1, #prev do
		byId[prev[i].id] = prev[i]
	end

	local n = rCount(r, "enemies", MAX_ENTITIES)
	local out = tcreate(n) :: { RenderEnemy }
	local lastId = -1
	for i = 1, n do
		local ctrl = rU8(r, "enemy.ctrl")
		local mode = ctrl % 4
		local hpPresent = math.floor(ctrl / F_HP) % 2 == 1
		local isNew = math.floor(ctrl / F_NEW) % 2 == 1
		local archChanged = math.floor(ctrl / F_ARCH) % 2 == 1
		local code = math.floor(ctrl / 32)

		local step: number
		if code == ID_ESCAPE then
			step = rSVar(r, "enemy.idStep")
		else
			step = code
		end
		local id = lastId + 1 + step
		lastId = id

		local p = byId[id]
		if isNew then
			if mode ~= XY_ABS or not hpPresent then
				error("Wire: malformed new-enemy record", 0)
			end
		elseif p == nil then
			error(`Wire: delta references unknown enemy id {id} — base view mismatch`, 0)
		end

		local kind, name, base_hp, boss
		if isNew or archChanged then
			local ai = rUVar(r, "enemy.arch") + 1
			local a = archetypes[ai]
			if a == nil then
				error(`Wire: enemy archetype index {ai} out of range`, 0)
			end
			kind, name, base_hp, boss = a.kind, a.name, a.base_hp, a.boss
		else
			kind, name, base_hp, boss = p.kind, p.name, p.base_hp, p.boss
		end

		local x, y = rXY(r, mode, p and p.x or 0, p and p.y or 0, "enemy")

		local hp: number
		if hpPresent then
			if isNew then
				hp = rSVar(r, "enemy.hp")
			else
				hp = p.hp + rSVar(r, "enemy.dhp")
			end
		else
			hp = p.hp
		end

		out[i] = { id = id, x = x, y = y, hp = hp, base_hp = base_hp, kind = kind, boss = boss, name = name }
	end
	return out
end

-- Projectiles carry no id in `RenderView`, and the sim compacts them in place
-- preserving relative order (`Combat.advanceProjectiles` rebuilds `survivors`
-- in iteration order). So they are matched by array index, with a per-index
-- 2-bit mode packed four to a byte; an index whose occupant was replaced by a
-- removal shift simply selects the absolute mode and costs a couple of bytes.

local function encodePoints(w: Writer, prev: { RenderPoint }, next_: { RenderPoint })
	local n = #next_
	wUVar(w, n, "projectiles")
	if n == 0 then
		return
	end
	local modes = tcreate(n) :: { number }
	for i = 1, n do
		local p = prev[i]
		modes[i] = xyMode(p and p.x, p and p.y, int(next_[i].x, "projectile.x"), int(next_[i].y, "projectile.y"))
	end
	-- 2-bit mode plane, four entries per byte.
	local planeBytes = math.ceil(n / 4)
	for b = 0, planeBytes - 1 do
		local v = 0
		for k = 0, 3 do
			local idx = b * 4 + k + 1
			if idx <= n then
				v += modes[idx] * (4 ^ k)
			end
		end
		wU8(w, v)
	end
	for i = 1, n do
		local p = prev[i]
		wXY(w, modes[i], p and p.x, p and p.y, next_[i].x, next_[i].y, "projectile")
	end
end

local function decodePoints(r: Reader, prev: { RenderPoint }): { RenderPoint }
	local n = rCount(r, "projectiles", MAX_ENTITIES)
	local out = tcreate(n) :: { RenderPoint }
	if n == 0 then
		return out
	end
	local planeBytes = math.ceil(n / 4)
	local modes: { number } = tcreate(n)
	for b = 0, planeBytes - 1 do
		local v = rU8(r, "projectile.modes")
		for k = 0, 3 do
			local idx = b * 4 + k + 1
			if idx <= n then
				modes[idx] = math.floor(v / (4 ^ k)) % 4
			end
		end
	end
	for i = 1, n do
		local p = prev[i]
		if modes[i] ~= XY_ABS and p == nil then
			error(`Wire: projectile {i} coded as a delta with no base — base view mismatch`, 0)
		end
		local x, y = rXY(r, modes[i], p and p.x or 0, p and p.y or 0, "projectile")
		out[i] = { x = x, y = y }
	end
	return out
end

local function encodeMinions(w: Writer, prev: { RenderMinion }, next_: { RenderMinion })
	local n = #next_
	wUVar(w, n, "minions")
	for i = 1, n do
		local m = next_[i]
		local p = prev[i]
		local mode = xyMode(p and p.x, p and p.y, int(m.x, "minion.x"), int(m.y, "minion.y"))
		local kindChanged = p == nil or p.kind ~= m.kind
		local ctrl = mode + (kindChanged and F_KIND or 0)
		wU8(w, ctrl)
		if kindChanged then
			wSVar(w, m.kind, "minion.kind")
		end
		wXY(w, mode, p and p.x, p and p.y, m.x, m.y, "minion")
	end
end

local function decodeMinions(r: Reader, prev: { RenderMinion }): { RenderMinion }
	local n = rCount(r, "minions", MAX_ENTITIES)
	local out = tcreate(n) :: { RenderMinion }
	for i = 1, n do
		local ctrl = rU8(r, "minion.ctrl")
		local mode = ctrl % 4
		local kindChanged = math.floor(ctrl / F_KIND) % 2 == 1
		local p = prev[i]
		if (mode ~= XY_ABS or not kindChanged) and p == nil then
			error(`Wire: minion {i} coded as a delta with no base — base view mismatch`, 0)
		end
		local kind: number
		if kindChanged then
			kind = rSVar(r, "minion.kind")
		else
			kind = (p :: RenderMinion).kind
		end
		local x, y = rXY(r, mode, p and p.x or 0, p and p.y or 0, "minion")
		out[i] = { x = x, y = y, kind = kind }
	end
	return out
end

-- ============================ message assembly ============================

--- Gather the archetypes this message must carry: one per enemy that is new to
--- `next_` relative to `prev`, or whose catalog fields changed.
local function collectArchetypes(
	prev: { RenderEnemy },
	next_: { RenderEnemy }
): ({ Archetype }, { [string]: number })
	local byId: { [number]: RenderEnemy } = {}
	for i = 1, #prev do
		byId[prev[i].id] = prev[i]
	end
	local list: { Archetype } = {}
	local index: { [string]: number } = {}
	for i = 1, #next_ do
		local e = next_[i]
		local p = byId[e.id]
		if p == nil or p.kind ~= e.kind or p.name ~= e.name or p.base_hp ~= e.base_hp or p.boss ~= e.boss then
			local key = archKey(e.kind, e.name, e.base_hp, e.boss)
			if index[key] == nil then
				list[#list + 1] = { kind = e.kind, name = e.name, base_hp = e.base_hp, boss = e.boss }
				index[key] = #list
			end
		end
	end
	return list, index
end

local function encodeBody(w: Writer, prev: RenderView, next_: RenderView, mask: number)
	local archList, archIndex = collectArchetypes(prev.enemies, next_.enemies)
	encodeArchetypes(w, archList)
	encodeEnemies(w, prev.enemies, next_.enemies, archIndex)
	encodePoints(w, prev.projectiles, next_.projectiles)
	encodeMinions(w, prev.minions, next_.minions)

	if mask % 2 == 1 then
		wSVar(w, next_.round, "round")
		wU8(w, next_.dead and 1 or 0)
	end
	if math.floor(mask / SEC_TANK) % 2 == 1 then
		local t = next_.tank
		wSVar(w, t.x, "tank.x")
		wSVar(w, t.y, "tank.y")
		wSVar(w, t.hp, "tank.hp")
		wSVar(w, t.max_hp, "tank.max_hp")
		wSVar(w, t.revives, "tank.revives")
	end
	if math.floor(mask / SEC_ECONOMY) % 2 == 1 then
		local e = next_.economy
		wSVar(w, e.gold, "economy.gold")
		wSVar(w, e.income_per_tick, "economy.income_per_tick")
		wSVar(w, e.rerolls_remaining, "economy.rerolls_remaining")
		wSVar(w, e.reroll_cost, "economy.reroll_cost")
	end
	if math.floor(mask / SEC_SHOP) % 2 == 1 then
		local shop = next_.shop
		wUVar(w, #shop, "shop")
		for i = 1, #shop do
			local o = shop[i]
			wSVar(w, o.slot, "offer.slot")
			wSVar(w, o.cost, "offer.cost")
			wSVar(w, o.rarity, "offer.rarity")
			wU8(w, (o.is_weapon and 1 or 0) + (o.affordable and 2 or 0))
			wStr(w, o.name, "offer.name")
		end
	end
	if math.floor(mask / SEC_ARSENAL) % 2 == 1 then
		local ars = next_.arsenal
		wUVar(w, #ars, "arsenal")
		for i = 1, #ars do
			wSVar(w, ars[i].count, "arsenal.count")
			wStr(w, ars[i].name, "arsenal.name")
		end
	end
	if math.floor(mask / SEC_STATS) % 2 == 1 then
		local s = next_.stats
		wSVar(w, s.damage_dealt, "stats.damage_dealt")
		wSVar(w, s.gold_earned, "stats.gold_earned")
		wSVar(w, s.bought_attack_mask, "stats.bought_attack_mask")
		wSVar(w, s.weapons_bought, "stats.weapons_bought")
		wSVar(w, s.economy_purchases, "stats.economy_purchases")
	end
end

local function decodeBody(r: Reader, prev: RenderView, tick: number, mask: number): RenderView
	local archetypes = decodeArchetypes(r)
	local enemies = decodeEnemies(r, prev.enemies, archetypes)
	local projectiles = decodePoints(r, prev.projectiles)
	local minions = decodeMinions(r, prev.minions)

	local round, dead
	if mask % 2 == 1 then
		round = rSVar(r, "round")
		dead = rU8(r, "dead") ~= 0
	else
		round, dead = prev.round, prev.dead
	end

	local tank: RenderTank
	if math.floor(mask / SEC_TANK) % 2 == 1 then
		tank = {
			x = rSVar(r, "tank.x"),
			y = rSVar(r, "tank.y"),
			hp = rSVar(r, "tank.hp"),
			max_hp = rSVar(r, "tank.max_hp"),
			revives = rSVar(r, "tank.revives"),
		}
	else
		tank = copyTank(prev.tank)
	end

	local economy: RenderEconomy
	if math.floor(mask / SEC_ECONOMY) % 2 == 1 then
		economy = {
			gold = rSVar(r, "economy.gold"),
			income_per_tick = rSVar(r, "economy.income_per_tick"),
			rerolls_remaining = rSVar(r, "economy.rerolls_remaining"),
			reroll_cost = rSVar(r, "economy.reroll_cost"),
		}
	else
		economy = copyEconomy(prev.economy)
	end

	local shop: { RenderOffer }
	if math.floor(mask / SEC_SHOP) % 2 == 1 then
		local n = rCount(r, "shop", MAX_TABLE_ENTRIES)
		shop = tcreate(n) :: { RenderOffer }
		for i = 1, n do
			local slot = rSVar(r, "offer.slot")
			local cost = rSVar(r, "offer.cost")
			local rarity = rSVar(r, "offer.rarity")
			local flags = rU8(r, "offer.flags")
			local name = rStr(r, "offer.name")
			shop[i] = {
				slot = slot,
				name = name,
				cost = cost,
				is_weapon = flags % 2 == 1,
				affordable = math.floor(flags / 2) % 2 == 1,
				rarity = rarity,
			}
		end
	else
		shop = copyShop(prev.shop)
	end

	local arsenal: { RenderArsenalEntry }
	if math.floor(mask / SEC_ARSENAL) % 2 == 1 then
		local n = rCount(r, "arsenal", MAX_TABLE_ENTRIES)
		arsenal = tcreate(n) :: { RenderArsenalEntry }
		for i = 1, n do
			local count = rSVar(r, "arsenal.count")
			arsenal[i] = { name = rStr(r, "arsenal.name"), count = count }
		end
	else
		arsenal = copyArsenal(prev.arsenal)
	end

	local stats: RenderStats
	if math.floor(mask / SEC_STATS) % 2 == 1 then
		stats = {
			damage_dealt = rSVar(r, "stats.damage_dealt"),
			gold_earned = rSVar(r, "stats.gold_earned"),
			bought_attack_mask = rSVar(r, "stats.bought_attack_mask"),
			weapons_bought = rSVar(r, "stats.weapons_bought"),
			economy_purchases = rSVar(r, "stats.economy_purchases"),
		}
	else
		stats = copyStats(prev.stats)
	end

	if r.pos ~= r.len then
		error(`Wire: {r.len - r.pos} trailing byte(s) after the view message`, 0)
	end

	return {
		tick = tick,
		round = round,
		dead = dead,
		tank = tank,
		enemies = enemies,
		projectiles = projectiles,
		minions = minions,
		economy = economy,
		shop = shop,
		arsenal = arsenal,
		stats = stats,
	}
end

-- ============================ public: view codec ============================

--- KEYFRAME. Self-contained: a client with no history at all can decode this
--- into a complete `RenderView`. Send one on join, on respawn/spectate switch,
--- and whenever `applyDelta` rejects a base (see `Wire.baseTick`).
function Wire.packView(view: RenderView, opts: PackOpts?): buffer
	local w = wNew(opts and opts.capacity or 2048)
	wU8(w, Wire.MAGIC)
	wU8(w, Wire.VERSION)
	wU8(w, Wire.KIND_KEYFRAME)
	wUVar(w, view.tick, "tick")
	wU8(w, SEC_ALL)
	encodeBody(w, emptyView(), view, SEC_ALL)
	return wFinish(w)
end

--- DELTA of `next_` against `prev`. The receiver must be holding exactly
--- `prev` — its `tick` is written into the message and checked on apply.
function Wire.packDelta(prev: RenderView, next_: RenderView, opts: PackOpts?): buffer
	local mask = 0
	if prev.round ~= next_.round or prev.dead ~= next_.dead then
		mask += SEC_SCALARS
	end
	if not tankEq(prev.tank, next_.tank) then
		mask += SEC_TANK
	end
	if not economyEq(prev.economy, next_.economy) then
		mask += SEC_ECONOMY
	end
	if not shopEq(prev.shop, next_.shop) then
		mask += SEC_SHOP
	end
	if not arsenalEq(prev.arsenal, next_.arsenal) then
		mask += SEC_ARSENAL
	end
	if not statsEq(prev.stats, next_.stats) then
		mask += SEC_STATS
	end

	local w = wNew(opts and opts.capacity or 1024)
	wU8(w, Wire.MAGIC)
	wU8(w, Wire.VERSION)
	wU8(w, Wire.KIND_DELTA)
	wUVar(w, next_.tick, "tick")
	wUVar(w, prev.tick, "baseTick")
	wU8(w, mask)
	encodeBody(w, prev, next_, mask)
	return wFinish(w)
end

local function readHeader(r: Reader): (number, number, number?, number)
	local magic = rU8(r, "magic")
	if magic ~= Wire.MAGIC then
		error(`Wire: bad magic 0x{string.format("%02x", magic)}`, 0)
	end
	local version = rU8(r, "version")
	if version ~= Wire.VERSION then
		error(`Wire: unsupported wire version {version} (this build speaks {Wire.VERSION})`, 0)
	end
	local kind = rU8(r, "kind")
	if kind ~= Wire.KIND_KEYFRAME and kind ~= Wire.KIND_DELTA then
		error(`Wire: unknown message kind {kind}`, 0)
	end
	local tick = rUVar(r, "tick")
	local baseTick: number? = nil
	if kind == Wire.KIND_DELTA then
		baseTick = rUVar(r, "baseTick")
	end
	local mask = rU8(r, "mask")
	return kind, tick, baseTick, mask
end

--- True when `buf` is a keyframe and can be decoded with no prior view.
function Wire.isKeyframe(buf: buffer): boolean
	local r = rNew(buf)
	local kind = select(1, readHeader(r))
	return kind == Wire.KIND_KEYFRAME
end

--- The tick of the view a delta is based on, or `nil` for a keyframe. A client
--- compares this with the tick it holds and asks for a keyframe on mismatch.
function Wire.baseTick(buf: buffer): number?
	local r = rNew(buf)
	local _, _, baseTick = readHeader(r)
	return baseTick
end

--- Decode a KEYFRAME. Errors on a delta — use `applyDelta`/`unpack`.
function Wire.unpackView(buf: buffer): RenderView
	local r = rNew(buf)
	local kind, tick, _, mask = readHeader(r)
	if kind ~= Wire.KIND_KEYFRAME then
		error("Wire: unpackView called on a delta message", 0)
	end
	if mask ~= SEC_ALL then
		error("Wire: keyframe with an incomplete section mask", 0)
	end
	return decodeBody(r, emptyView(), tick, mask)
end

--- Decode a DELTA on top of the view the client currently holds.
--- Errors if `prev.tick` is not the tick the delta was built against — that is
--- the desync signal, and the answer to it is a keyframe.
function Wire.applyDelta(prev: RenderView, buf: buffer): RenderView
	local r = rNew(buf)
	local kind, tick, baseTick, mask = readHeader(r)
	if kind ~= Wire.KIND_DELTA then
		error("Wire: applyDelta called on a keyframe message", 0)
	end
	if baseTick ~= prev.tick then
		error(`Wire: delta base tick {tostring(baseTick)} does not match held view tick {prev.tick}`, 0)
	end
	return decodeBody(r, prev, tick, mask)
end

--- Encode whichever form applies: keyframe when there is no base, delta
--- otherwise. The server-side counterpart to `Wire.unpack`.
function Wire.pack(prev: RenderView?, next_: RenderView, opts: PackOpts?): buffer
	if prev == nil then
		return Wire.packView(next_, opts)
	end
	return Wire.packDelta(prev, next_, opts)
end

--- Decode whichever form arrived. `prev` may be `nil`; a delta then errors,
--- which is exactly the "I need a keyframe" condition.
function Wire.unpack(prev: RenderView?, buf: buffer): RenderView
	local r = rNew(buf)
	local kind, tick, baseTick, mask = readHeader(r)
	if kind == Wire.KIND_KEYFRAME then
		if mask ~= SEC_ALL then
			error("Wire: keyframe with an incomplete section mask", 0)
		end
		return decodeBody(r, emptyView(), tick, mask)
	end
	if prev == nil then
		error("Wire: received a delta with no base view — request a keyframe", 0)
	end
	if baseTick ~= prev.tick then
		error(`Wire: delta base tick {tostring(baseTick)} does not match held view tick {prev.tick}`, 0)
	end
	return decodeBody(r, prev, tick, mask)
end

-- ============================ public: chunking ============================

export type Chunking = {
	--- Bytes per chunk INCLUDING the 6-byte header. Must not exceed
	--- `Wire.HARD_CHUNK_LIMIT`; defaults to `Wire.MAX_CHUNK_BYTES`.
	maxChunkBytes: number?,
}

--- Split a message into transport-sized frames. `msgId` identifies the message
--- so the receiver can group fragments and spot a missing one; the caller
--- increments it per message (it wraps at 65536).
function Wire.chunk(buf: buffer, msgId: number, opts: Chunking?): { buffer }
	local maxChunk = (opts and opts.maxChunkBytes) or Wire.MAX_CHUNK_BYTES
	if maxChunk > Wire.HARD_CHUNK_LIMIT then
		error(`Wire: maxChunkBytes {maxChunk} exceeds the {Wire.HARD_CHUNK_LIMIT}-byte transport limit`, 0)
	end
	local payloadMax = maxChunk - Wire.CHUNK_HEADER_BYTES
	if payloadMax < 1 then
		error("Wire: maxChunkBytes leaves no room for a payload", 0)
	end
	local total = buffer.len(buf)
	local count = math.max(1, math.ceil(total / payloadMax))
	if count > 255 then
		error(`Wire: message of {total} bytes needs {count} chunks, over the 255 the framing allows`, 0)
	end
	local id = msgId % 65536

	local out = tcreate(count) :: { buffer }
	for i = 0, count - 1 do
		local offset = i * payloadMax
		local size = math.min(payloadMax, total - offset)
		local c = buffer.create(Wire.CHUNK_HEADER_BYTES + size)
		buffer.writeu8(c, 0, Wire.CHUNK_MAGIC)
		buffer.writeu8(c, 1, Wire.VERSION)
		buffer.writeu16(c, 2, id)
		buffer.writeu8(c, 4, i)
		buffer.writeu8(c, 5, count)
		if size > 0 then
			buffer.copy(c, Wire.CHUNK_HEADER_BYTES, buf, offset, size)
		end
		out[i + 1] = c
	end
	return out
end

--- Pack and frame in one call.
function Wire.packChunked(prev: RenderView?, next_: RenderView, msgId: number, opts: PackOpts?): { buffer }
	return Wire.chunk(Wire.pack(prev, next_, opts), msgId)
end

type Partial = {
	id: number,
	count: number,
	received: number,
	seq: number,
	parts: { buffer },
}

export type Reassembler = {
	--- Messages abandoned incomplete — i.e. observed packet loss.
	dropped: number,
	--- Chunks discarded because their message had already been completed or
	--- evicted (a late duplicate).
	stale: number,
	--- Reason the most recent `push` returned `nil` without completing.
	lastError: string?,
	push: (self: Reassembler, chunk: buffer) -> buffer?,
	pending: (self: Reassembler) -> number,
	reset: (self: Reassembler) -> (),
}

export type ReassemblerOpts = {
	--- How many partially-received messages to hold at once. `UnreliableRemote`
	--- delivery is unordered, so a small window absorbs interleaving; when it
	--- overflows the oldest partial is abandoned and counted in `dropped`.
	window: number?,
}

local ReassemblerImpl = {}
ReassemblerImpl.__index = ReassemblerImpl

--- Regroup chunks into whole messages.
---
--- Returns the completed message `buffer`, or `nil` when this chunk did not
--- complete one. `self.lastError` explains a `nil` that was caused by a bad
--- frame rather than by simply waiting for more chunks; `self.dropped` counts
--- messages that were abandoned because a fragment never arrived, which is the
--- "detect a missing chunk" signal the caller acts on by requesting a keyframe.
function Wire.newReassembler(opts: ReassemblerOpts?): Reassembler
	local self = setmetatable({
		dropped = 0,
		stale = 0,
		lastError = nil :: string?,
		_window = math.max(1, (opts and opts.window) or 4),
		_partials = {} :: { [number]: Partial },
		_pending = 0,
		_seq = 0,
		_done = {} :: { [number]: boolean },
		_doneList = {} :: { number },
	}, ReassemblerImpl)
	return (self :: any) :: Reassembler
end

function ReassemblerImpl.reset(self: any)
	self._partials = {}
	self._pending = 0
	self._done = {}
	self._doneList = {}
	self.lastError = nil
end

function ReassemblerImpl.pending(self: any): number
	return self._pending
end

--- Remember a completed message id so late duplicates are swallowed rather
--- than starting a phantom partial. The set is bounded; ids wrap at 65536.
local function markDone(self: any, id: number)
	if self._done[id] then
		return
	end
	self._done[id] = true
	table.insert(self._doneList, id)
	if #self._doneList > 512 then
		self._done[table.remove(self._doneList, 1)] = nil
	end
end

local function evictOldest(self: any)
	local oldestId, oldestSeq = nil, math.huge
	for id, p in self._partials do
		if p.seq < oldestSeq then
			oldestId, oldestSeq = id, p.seq
		end
	end
	if oldestId ~= nil then
		self._partials[oldestId] = nil
		self._pending -= 1
		self.dropped += 1
	end
end

function ReassemblerImpl.push(self: any, chunk: buffer): buffer?
	self.lastError = nil
	local n = buffer.len(chunk)
	if n < Wire.CHUNK_HEADER_BYTES then
		self.lastError = "chunk shorter than its header"
		return nil
	end
	if buffer.readu8(chunk, 0) ~= Wire.CHUNK_MAGIC then
		self.lastError = "bad chunk magic"
		return nil
	end
	if buffer.readu8(chunk, 1) ~= Wire.VERSION then
		self.lastError = "unsupported chunk version"
		return nil
	end
	local id = buffer.readu16(chunk, 2)
	local index = buffer.readu8(chunk, 4)
	local count = buffer.readu8(chunk, 5)
	if count == 0 or index >= count then
		self.lastError = "chunk index out of range"
		return nil
	end

	if self._done[id] then
		self.stale += 1
		return nil
	end

	local size = n - Wire.CHUNK_HEADER_BYTES
	local payload = buffer.create(size)
	if size > 0 then
		buffer.copy(payload, 0, chunk, Wire.CHUNK_HEADER_BYTES, size)
	end

	-- Single-chunk messages are the common case and never enter the window.
	if count == 1 then
		markDone(self, id)
		return payload
	end

	local p: Partial? = self._partials[id]
	if p == nil then
		while self._pending >= self._window do
			evictOldest(self)
		end
		self._seq += 1
		p = { id = id, count = count, received = 0, seq = self._seq, parts = tcreate(count) :: { buffer } }
		self._partials[id] = p
		self._pending += 1
	end
	local part = p :: Partial
	if part.count ~= count then
		self.lastError = "conflicting chunk count for message " .. tostring(id)
		return nil
	end
	if part.parts[index + 1] ~= nil then
		self.stale += 1
		return nil
	end
	part.parts[index + 1] = payload
	part.received += 1
	if part.received < part.count then
		return nil
	end

	local total = 0
	for i = 1, part.count do
		total += buffer.len(part.parts[i])
	end
	local out = buffer.create(total)
	local at = 0
	for i = 1, part.count do
		local piece = part.parts[i]
		local len = buffer.len(piece)
		buffer.copy(out, at, piece, 0, len)
		at += len
	end
	self._partials[id] = nil
	self._pending -= 1
	markDone(self, id)
	return out
end

return Wire
