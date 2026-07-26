--[[
	View.luau — a transcription of `sim-core/crates/sim/src/view.rs`.

	Engine-agnostic VIEW-MODEL: a flat, owned snapshot of everything a renderer
	needs for one frame, derived from the public arena state. It contains no
	engine types and no `Fixed` — world coordinates are exposed as integer units
	(`Fixed.floorToInt`) so the boundary stays float-free on the sim side; the
	engine scales to pixels.

	This is the CONTRACT the front-end reads each frame, so the determinism core
	and the renderer can evolve independently. Building a view NEVER mutates the
	sim and NEVER feeds the checksum.

	Field names mirror `view.rs` exactly (snake_case), per `SIM-SPEC.md` §S3.

	No Roblox global is referenced; runs unchanged under a bare `luau`.
]]

local Fixed = require("../Fixed")
local Content = require("../Content")

local View = {}

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

-- ============================ catalog access ============================

-- Content ids are 0-based (`SIM-SPEC.md` §S3: array position in `content.json`
-- IS identity); Luau array storage is 1-indexed.
local function weaponDef(def: number)
	return Content.WEAPONS[def + 1]
end

local function enemyDef(def: number)
	return Content.ENEMIES[def + 1]
end

local function modifierDef(def: number)
	return Content.MODIFIERS[def + 1]
end

--- An i64 sim field as a plain Luau number. Fields the Rust types `i64` are held
--- as plain numbers (§S4); anything carried as an opaque `Fixed` i64 is
--- converted exactly (and errors loudly rather than rounding).
local function num(v)
	if type(v) == "table" then
		return Fixed.toInt(v)
	end
	return v
end

--- `OfferKind::Weapon` test, tolerant of the enum's Luau spelling.
local function isWeaponKind(k)
	return k == 0 or k == "Weapon" or (type(k) == "table" and k.code == 0)
end

-- ============================ snapshot ============================

--- Build a `RenderView` from current state. Read-only; allocation-light.
function View.snapshot(s): RenderView
	local tank = {
		x = Fixed.floorToInt(s.tank.pos.x),
		y = Fixed.floorToInt(s.tank.pos.y),
		hp = num(s.tank.hp),
		max_hp = num(s.tank.max_hp),
		revives = s.tank.revives,
	}

	local enemies = table.create(#s.enemies)
	for i = 1, #s.enemies do
		local e = s.enemies[i]
		local def = enemyDef(e.def)
		enemies[i] = {
			id = e.id,
			x = Fixed.floorToInt(e.pos.x),
			y = Fixed.floorToInt(e.pos.y),
			hp = num(e.hp),
			-- Catalog base HP — a denominator for an approximate HP bar.
			base_hp = num(def.base_hp),
			kind = e.def,
			boss = def.boss,
			name = def.name,
		}
	end

	local projectiles = table.create(#s.projectiles)
	for i = 1, #s.projectiles do
		local p = s.projectiles[i]
		projectiles[i] = { x = Fixed.floorToInt(p.pos.x), y = Fixed.floorToInt(p.pos.y) }
	end

	local minions = table.create(#s.minions)
	for i = 1, #s.minions do
		local m = s.minions[i]
		minions[i] = { x = Fixed.floorToInt(m.pos.x), y = Fixed.floorToInt(m.pos.y), kind = m.kind }
	end

	local gold = num(s.economy.gold)
	local economy = {
		gold = gold,
		-- Effective passive income per tick (base × income multiplier).
		income_per_tick = num(Fixed.scaleI64(s.economy.income_mult, s.economy.income_per_tick)),
		rerolls_remaining = s.economy.rerolls_remaining,
		reroll_cost = num(s.economy.reroll_cost),
	}

	local offers = s.shop.offers
	local shop = table.create(#offers)
	for i = 1, #offers do
		local off = offers[i]
		local name, is_weapon, rarity
		if isWeaponKind(off.kind) then
			local d = weaponDef(off.def)
			name, is_weapon, rarity = d.name, true, d.rarity
		else
			local d = modifierDef(off.def)
			name, is_weapon, rarity = d.name, false, d.rarity
		end
		local cost = num(off.cost)
		shop[i] = {
			-- `slot` is the 0-BASED shop index (it is what `Input.BuyOffer` takes).
			slot = i - 1,
			name = name,
			cost = cost,
			is_weapon = is_weapon,
			affordable = cost <= gold,
			-- 0 common · 1 uncommon · 2 rare · 3 epic (drives the shop frame colour).
			rarity = rarity,
		}
	end

	-- Owned weapons collapsed to `(name, count)`, sorted by name — the Rust uses a
	-- `BTreeMap<&str, u32>`, whose iteration order is byte-wise on the name.
	local counts = {}
	local names = {}
	for i = 1, #s.weapons do
		local n = weaponDef(s.weapons[i].def).name
		if counts[n] == nil then
			counts[n] = 1
			names[#names + 1] = n
		else
			counts[n] += 1
		end
	end
	table.sort(names)
	local arsenal = table.create(#names)
	for i = 1, #names do
		arsenal[i] = { name = names[i], count = counts[names[i]] }
	end

	return {
		tick = s.tick,
		round = s.round,
		dead = s.dead and true or false,
		tank = tank,
		enemies = enemies,
		projectiles = projectiles,
		minions = minions,
		economy = economy,
		shop = shop,
		arsenal = arsenal,
		stats = {
			damage_dealt = num(s.total_damage_dealt),
			gold_earned = num(s.total_gold_earned),
			-- Playstyle telemetry (cosmetic achievements) — never checksummed.
			bought_attack_mask = s.bought_attack_mask,
			weapons_bought = s.weapons_bought,
			economy_purchases = s.economy_purchases,
		},
	}
end

return View
