--!nonstrict
--[[
	content_test.luau — proves `src/shared/Content.luau` exposes `content.json`
	(`CONTRACTS.md` C1) with the values `sim-core/crates/sim/src/content.rs`
	defines, and that its embedded copy of the artifact has not gone stale.

	HOW TO RUN. Standalone Luau has no file I/O, so `content.json` is handed to
	the runner on the command line (the `test/parity.luau` pattern — every
	argument after `-a` is concatenated). From `roblox/test`:

	    luau content_test.luau -a "$(cat ../src/shared/content.json)"

	WHAT IT CHECKS
	  1. `Content.RAW_JSON` is BYTE-IDENTICAL to `src/shared/content.json`. The
	     module embeds the artifact because this layer cannot read files; this is
	     the check that makes the copy safe rather than a second source of truth.
	  2. Every entry in the artifact round-trips into the accessors — name and
	     every scalar, every `{tag,a,b,c}` payload, every wave row, every ring
	     point — compared against an independent `Json.decode` of the same text.
	  3. Spot checks against `content.rs` itself: values transcribed by hand out
	     of the Rust for weapons, enemies and modifiers of each shape, so a bug
	     that corrupted BOTH the JSON and the reader would still be caught.
	  4. `enemyHpMult`, on BOTH timelines, against digests PRODUCED BY RUNNING
	     THE RUST (`content::enemy_hp_mult`) — never hand-derived. The Roblox
	     column is the Rust evaluated at `6 * tick`: the F1 rescale is an exact
	     /6 of every gate, and `Fixed::from_ratio(6a, 6b) == from_ratio(a, b)`
	     (equal rationals floor alike), so the Rust IS the oracle for both.
	  5. `damageMultiplier` raws and its index normalisation (`% 5`, clamp).
	  6. `is_meta` / `is_economy` over the whole modifier catalog, against the id
	     sets the Rust reported.
]]

local Fixed = require("../src/shared/Fixed")
local Json = require("../src/shared/Json")
local Content = require("../src/shared/Content")

local failures = 0
local checks = 0

local function fail(msg: string)
	failures += 1
	print("FAIL  " .. msg)
end

local function eq(got, want, msg: string)
	checks += 1
	if got ~= want then
		fail(msg .. ": got " .. tostring(got) .. ", want " .. tostring(want))
	end
end

local function eqFixed(got, wantHex: string, msg: string)
	checks += 1
	local g = Fixed.toHex(got)
	if g ~= wantHex then
		fail(msg .. ": got " .. g .. ", want " .. wantHex)
	end
end

-- ============================ 1. the embedded copy ============================

local fileText = table.concat({ ... })
if #fileText == 0 then
	error(
		"content_test: pass src/shared/content.json on the command line:\n"
			.. '  luau content_test.luau -a "$(cat ../src/shared/content.json)"',
		0
	)
end
-- Shell command substitution (`-a "$(cat ...)"`) strips trailing newlines, so
-- the comparison is made modulo a trailing newline on either side. Nothing else
-- is normalised: every other byte must match.
local function stripTrailingNewlines(s: string): string
	while #s > 0 and string.sub(s, -1) == "\n" do
		s = string.sub(s, 1, -2)
	end
	return s
end
local embedded = stripTrailingNewlines(Content.RAW_JSON)
local onDisk = stripTrailingNewlines(fileText)
checks += 1
if embedded ~= onDisk then
	local at = 0
	for i = 1, math.min(#embedded, #onDisk) do
		if string.sub(embedded, i, i) ~= string.sub(onDisk, i, i) then
			at = i
			break
		end
	end
	fail(
		"Content.RAW_JSON is NOT identical to src/shared/content.json ("
			.. #embedded
			.. " vs "
			.. #onDisk
			.. " bytes, first difference at byte "
			.. (if at > 0 then tostring(at) else "end-of-file")
			.. ") — the embedded catalog is stale, re-embed it"
	)
end

-- An INDEPENDENT decode of the artifact, used as the round-trip reference.
local doc = Json.decode(fileText)

-- ============================ 2. round-trip ============================

eq(Content.SCHEMA_VERSION, doc.schema_version, "schema_version")
eq(Content.CONTENT_HASH, doc.content_hash, "content_hash")
eq(Content.CONTENT_HASH, doc.constants and doc.content_hash, "content_hash present")
eq(#Content.WEAPONS, doc.constants.num_weapons, "num_weapons")
eq(#Content.ENEMIES, doc.constants.num_enemies, "num_enemies")
eq(#Content.MODIFIERS, doc.constants.num_modifiers, "num_modifiers")
eq(#Content.WAVES, #doc.waves, "num_waves")
eq(#Content.BOSS_ESCORT, #doc.boss_escort, "num_boss_escort")
eq(#Content.SPAWN_RING, #doc.spawn_ring, "num_spawn_ring")

--- Assert a decoded payload still carries its own `words()` encoding and names a
--- known variant.
local function checkWords(got, want, msg: string)
	eq(got.tag, want.tag, msg .. ".tag")
	eq(got.a, want.a, msg .. ".a")
	eq(got.b, want.b, msg .. ".b")
	eq(got.c, want.c, msg .. ".c")
	checks += 1
	if type(got.variant) ~= "string" then
		fail(msg .. ": decoded payload has no `variant` name")
	end
end

for i, w in ipairs(doc.weapons) do
	local g = Content.WEAPONS[i]
	local m = "weapons[" .. (i - 1) .. "]"
	eq(g.id, i - 1, m .. ".id")
	eq(g.name, w.name, m .. ".name")
	eq(g.rarity, w.rarity, m .. ".rarity")
	eq(g.cost, w.cost, m .. ".cost")
	eq(g.damage, w.damage, m .. ".damage")
	eq(g.damage_type, w.damage_type, m .. ".damage_type")
	eq(g.cooldown_ticks, w.cooldown_ticks, m .. ".cooldown_ticks")
	eq(g.range, w.range, m .. ".range")
	eq(g.proj_speed, w.proj_speed, m .. ".proj_speed")
	eq(g.on_hit.poison_dps, w.on_hit.poison_dps, m .. ".on_hit.poison_dps")
	eq(g.on_hit.poison_ticks, w.on_hit.poison_ticks, m .. ".on_hit.poison_ticks")
	eq(g.on_hit.frost_stacks, w.on_hit.frost_stacks, m .. ".on_hit.frost_stacks")
	eq(g.on_hit.fire_stacks, w.on_hit.fire_stacks, m .. ".on_hit.fire_stacks")
	eq(g.on_hit.stun_ticks, w.on_hit.stun_ticks, m .. ".on_hit.stun_ticks")
	checkWords(g.attack, w.attack, m .. ".attack")
	checkWords(g.ability, w.ability, m .. ".ability")
	-- C1: the Attack tag IS the attack scope id.
	eq(Content.attackScopeId(g.attack), w.attack.tag, m .. " attackScopeId == attack.tag")
end

for i, e in ipairs(doc.enemies) do
	local g = Content.ENEMIES[i]
	local m = "enemies[" .. (i - 1) .. "]"
	eq(g.id, i - 1, m .. ".id")
	eq(g.name, e.name, m .. ".name")
	eq(g.base_hp, e.base_hp, m .. ".base_hp")
	eq(g.move_speed, e.move_speed, m .. ".move_speed")
	eq(g.contact_damage, e.contact_damage, m .. ".contact_damage")
	eq(g.bounty, e.bounty, m .. ".bounty")
	eq(g.armor_class, e.armor_class, m .. ".armor_class")
	eq(g.archetype, e.archetype, m .. ".archetype")
	eq(g.boss, e.boss, m .. ".boss")
	checkWords(g.ability, e.ability, m .. ".ability")
	-- C1 addendum: `b = cooldown_ticks | (damage_type << 32)`.
	if g.ability.variant == "RangedAttack" then
		eq(g.ability.range, e.ability.a, m .. ".ability.range")
		eq(g.ability.damage, e.ability.c, m .. ".ability.damage")
		eq(
			g.ability.cooldown_ticks + g.ability.damage_type * 4294967296,
			e.ability.b,
			m .. ".ability repacks to b"
		)
	end
end

local metaIds, economyIds = {}, {}
for i, mo in ipairs(doc.modifiers) do
	local g = Content.MODIFIERS[i]
	local m = "modifiers[" .. (i - 1) .. "]"
	eq(g.id, i - 1, m .. ".id")
	eq(g.name, mo.name, m .. ".name")
	eq(g.rarity, mo.rarity, m .. ".rarity")
	eq(g.cost, mo.cost, m .. ".cost")
	eq(#g.effects, #mo.effects, m .. ".effects length")
	for j, ef in ipairs(mo.effects) do
		checkWords(g.effects[j], ef, m .. ".effects[" .. j .. "]")
	end
	if mo.ramp == nil or mo.ramp == Json.null then
		eq(g.ramp, nil, m .. ".ramp is nil")
	else
		checks += 1
		if g.ramp == nil then
			fail(m .. ".ramp: expected a ramp")
		else
			checkWords(g.ramp.effect, mo.ramp.effect, m .. ".ramp.effect")
			eq(g.ramp.interval_ticks, mo.ramp.interval_ticks, m .. ".ramp.interval_ticks")
		end
	end
	if g.is_meta then
		metaIds[#metaIds + 1] = i - 1
	end
	if g.is_economy then
		economyIds[#economyIds + 1] = i - 1
	end
end

local function checkWaveTable(got, want, what: string)
	for i, w in ipairs(want) do
		local g = got[i]
		local m = what .. "[" .. (i - 1) .. "]"
		eq(g.enemy, w.enemy, m .. ".enemy")
		eq(g.cadence_ticks, w.cadence_ticks, m .. ".cadence_ticks")
		eq(g.start_tick, w.start_tick, m .. ".start_tick")
	end
end
checkWaveTable(Content.WAVES, doc.waves, "waves")
checkWaveTable(Content.BOSS_ESCORT, doc.boss_escort, "boss_escort")

for i, p in ipairs(doc.spawn_ring) do
	local g = Content.SPAWN_RING[i]
	eq(Fixed.raw(g.x), p.x, "spawn_ring[" .. (i - 1) .. "].x")
	eq(Fixed.raw(g.y), p.y, "spawn_ring[" .. (i - 1) .. "].y")
end

for dt = 0, 4 do
	for ac = 0, 2 do
		eq(
			Fixed.raw(Content.damageMultiplier(dt, ac)),
			doc.damage_matrix[dt + 1][ac + 1],
			"damage_matrix[" .. dt .. "][" .. ac .. "]"
		)
	end
end

-- Both timelines round-trip, and neither is silently substituted for the other.
for _, name in ipairs({ "steam", "roblox" }) do
	for key, value in pairs(doc.timeline[name]) do
		eq(Content.TIMELINE[name][key], value, "timeline." .. name .. "." .. key)
	end
end
checks += 1
if Content.TIMELINE.steam.boss_spawn_tick == Content.TIMELINE.roblox.boss_spawn_tick then
	fail("the two timelines must not be the same table")
end

-- The flat UPPER_SNAKE names are derived from `constants`, so every one must
-- agree with its passthrough.
for key, value in pairs(doc.constants) do
	eq(Content[string.upper(key)], value, "flat constant " .. string.upper(key))
	eq(Content.constants[key], value, "constants." .. key)
end
-- ... and the Rust's own module constants (the STEAM gates) are flat too.
for key, value in pairs(doc.timeline.steam) do
	eq(Content[string.upper(key)], value, "flat steam constant " .. string.upper(key))
end

-- lowerCamel aliases are the SAME objects, not copies.
eq(Content.weapons, Content.WEAPONS, "weapons alias")
eq(Content.enemies, Content.ENEMIES, "enemies alias")
eq(Content.modifiers, Content.MODIFIERS, "modifiers alias")
eq(Content.waves, Content.WAVES, "waves alias")
eq(Content.WAVE_M0, Content.WAVES, "WAVE_M0 alias")
eq(Content.bossEscort, Content.BOSS_ESCORT, "bossEscort alias")
eq(Content.spawnRing, Content.SPAWN_RING, "spawnRing alias")
eq(Content.timeline, Content.TIMELINE, "timeline alias")
eq(Content.CONSTANTS, Content.constants, "constants alias")

-- Content is DATA: the catalog must not be mutable from a behavior module.
checks += 1
if pcall(function()
	Content.WEAPONS[1].damage = 1
end) then
	fail("weapon defs must be frozen")
end
checks += 1
if pcall(function()
	Content.SPAWN_RING[1] = nil
end) then
	fail("SPAWN_RING must be frozen")
end

-- ============================ 3. spot checks vs content.rs ============================
-- Values transcribed BY HAND from `sim-core/crates/sim/src/content.rs`.

do
	local w = Content.WEAPONS[0 + 1] -- content.rs: WEAPONS[0], the starting weapon
	eq(w.name, "Bow", "WEAPONS[0].name")
	eq(w.rarity, 0, "Bow.rarity")
	eq(w.cost, 500, "Bow.cost")
	eq(w.damage, 75, "Bow.damage")
	eq(w.damage_type, Content.DMG_PIERCING, "Bow.damage_type == DMG_PIERCING")
	eq(w.attack.variant, "SingleTarget", "Bow.attack")
	eq(w.cooldown_ticks, 30, "Bow.cooldown_ticks")
	eq(w.range, 900, "Bow.range")
	eq(w.proj_speed, 45, "Bow.proj_speed")
	eq(w.ability.variant, "None", "Bow.ability")
	eq(w.on_hit.poison_dps, 0, "Bow.on_hit is NONE")
	eq(Content.STARTING_WEAPON, 0, "STARTING_WEAPON is Bow")
end
do
	local w = Content.WEAPONS[1 + 1] -- Mortar Launcher: Splash
	eq(w.name, "Mortar Launcher", "WEAPONS[1].name")
	eq(w.attack.variant, "Splash", "Mortar.attack.variant")
	eq(w.attack.radius, 300, "Mortar.attack.radius")
	eq(w.damage_type, Content.DMG_SIEGE, "Mortar.damage_type")
	eq(w.damage, 340, "Mortar.damage")
	eq(w.cooldown_ticks, 48, "Mortar.cooldown_ticks")
	eq(w.proj_speed, 30, "Mortar.proj_speed")
end
do
	local w = Content.WEAPONS[6 + 1] -- Ballista: Barrage(4)
	eq(w.name, "Ballista", "WEAPONS[6].name")
	eq(w.attack.variant, "Barrage", "Ballista.attack.variant")
	eq(w.attack.n, 4, "Ballista.attack.n")
end
do
	local w = Content.WEAPONS[7 + 1] -- Immolation: Area(300)
	eq(w.attack.variant, "Area", "Immolation.attack.variant")
	eq(w.attack.radius, 300, "Immolation.attack.radius")
end
do
	local w = Content.WEAPONS[8 + 1] -- Shockwave Axe: Wave(300)
	eq(w.attack.variant, "Wave", "ShockwaveAxe.attack.variant")
	eq(w.attack.extra, 300, "ShockwaveAxe.attack.extra")
end
do
	local w = Content.WEAPONS[9 + 1] -- Moon Glaive: Bounce(4)
	eq(w.attack.variant, "Bounce", "MoonGlaive.attack.variant")
	eq(w.attack.n, 4, "MoonGlaive.attack.n")
end
do
	local w = Content.WEAPONS[10 + 1]
	eq(w.name, "Death Engine", "WEAPONS[10].name")
	eq(Content.DEATH_ENGINE, 10, "DEATH_ENGINE id")
end
do
	local w = Content.WEAPONS[17 + 1] -- Suckula: LifeDrain { per_hit: 40 }
	eq(w.name, "Suckula", "WEAPONS[17].name")
	eq(w.rarity, 2, "Suckula.rarity")
	eq(w.cost, 3000, "Suckula.cost")
	eq(w.damage, 1100, "Suckula.damage")
	eq(w.damage_type, Content.DMG_CHAOS, "Suckula.damage_type")
	eq(w.ability.variant, "LifeDrain", "Suckula.ability.variant")
	eq(w.ability.per_hit, 40, "Suckula.ability.per_hit")
end
do
	local w = Content.WEAPONS[25 + 1] -- Slap: Knockback { dist: 300 }
	eq(w.name, "Slap", "WEAPONS[25].name")
	eq(w.damage, 2600, "Slap.damage")
	eq(w.ability.variant, "Knockback", "Slap.ability.variant")
	eq(w.ability.dist, 300, "Slap.ability.dist")
end
do
	local w = Content.WEAPONS[21 + 1] -- Demon Eye: VulnOnHit { stacks: 10 }
	eq(w.name, "Demon Eye", "WEAPONS[21].name")
	eq(w.ability.variant, "VulnOnHit", "DemonEye.ability.variant")
	eq(w.ability.stacks, 10, "DemonEye.ability.stacks")
end
do
	local w = Content.WEAPONS[50 + 1] -- Magic Claw: ManaDrain { per_hit: 20 }
	eq(w.name, "Magic Claw", "WEAPONS[50].name")
	eq(w.ability.variant, "ManaDrain", "MagicClaw.ability.variant")
	eq(w.ability.per_hit, 20, "MagicClaw.ability.per_hit")
end
do
	local w = Content.WEAPONS[54 + 1] -- Shroom Doom: Summon { kind: 1, hp: 1500, damage: 600 }
	eq(w.name, "Shroom Doom", "WEAPONS[54].name")
	eq(w.rarity, 3, "ShroomDoom.rarity")
	eq(w.cost, 5000, "ShroomDoom.cost")
	eq(w.damage, 3200, "ShroomDoom.damage")
	eq(w.attack.variant, "Area", "ShroomDoom.attack.variant")
	eq(w.attack.radius, 300, "ShroomDoom.attack.radius")
	eq(w.cooldown_ticks, 90, "ShroomDoom.cooldown_ticks")
	eq(w.on_hit.fire_stacks, 60, "ShroomDoom.on_hit.fire_stacks")
	eq(w.on_hit.stun_ticks, 60, "ShroomDoom.on_hit.stun_ticks")
	eq(w.ability.variant, "Summon", "ShroomDoom.ability.variant")
	eq(w.ability.kind, 1, "ShroomDoom.ability.kind")
	eq(w.ability.hp, 1500, "ShroomDoom.ability.hp")
	eq(w.ability.damage, 600, "ShroomDoom.ability.damage")
end

do
	local e = Content.ENEMIES[0 + 1] -- Squeakzilla
	eq(e.name, "Squeakzilla", "ENEMIES[0].name")
	eq(e.base_hp, 200, "Squeakzilla.base_hp")
	eq(e.move_speed, 8, "Squeakzilla.move_speed")
	eq(e.contact_damage, 500, "Squeakzilla.contact_damage")
	eq(e.bounty, 10, "Squeakzilla.bounty")
	eq(e.armor_class, Content.ARMOR_LIGHT, "Squeakzilla.armor_class")
	eq(e.archetype, Content.ARCHETYPE_SWARM, "Squeakzilla.archetype")
	eq(e.ability.variant, "None", "Squeakzilla.ability")
	eq(e.boss, false, "Squeakzilla.boss")
end
do
	local e = Content.ENEMIES[1 + 1] -- Fanged Death
	eq(e.name, "Fanged Death", "ENEMIES[1].name")
	eq(e.base_hp, 1200, "FangedDeath.base_hp")
	eq(e.move_speed, 4, "FangedDeath.move_speed")
	eq(e.contact_damage, 1500, "FangedDeath.contact_damage")
	eq(e.bounty, 40, "FangedDeath.bounty")
	eq(e.armor_class, Content.ARMOR_MEDIUM, "FangedDeath.armor_class")
	eq(e.archetype, Content.ARCHETYPE_TANK, "FangedDeath.archetype")
end
do
	local e = Content.ENEMIES[Content.BOSS + 1] -- The Hippocrate
	eq(e.boss, true, "ENEMIES[BOSS].boss")
	eq(e.archetype, Content.ARCHETYPE_BOSS, "boss.archetype")
	eq(Content.BOSS, 2, "BOSS id")
end
do
	-- content.rs: EnemyAbility::RangedAttack { range: 900, cooldown_ticks: 45,
	--                                          damage: 250, damage_type: DMG_MAGIC }
	local e = Content.ENEMIES[7 + 1]
	eq(e.name, "Nope Rope", "ENEMIES[7].name")
	eq(e.base_hp, 320, "NopeRope.base_hp")
	eq(e.move_speed, 6, "NopeRope.move_speed")
	eq(e.contact_damage, 200, "NopeRope.contact_damage")
	eq(e.bounty, 30, "NopeRope.bounty")
	eq(e.archetype, Content.ARCHETYPE_CASTER, "NopeRope.archetype")
	eq(e.ability.variant, "RangedAttack", "NopeRope.ability.variant")
	eq(e.ability.range, 900, "NopeRope.ability.range")
	eq(e.ability.cooldown_ticks, 45, "NopeRope.ability.cooldown_ticks")
	eq(e.ability.damage, 250, "NopeRope.ability.damage")
	eq(e.ability.damage_type, Content.DMG_MAGIC, "NopeRope.ability.damage_type")
end
do
	-- The three other RangedAttack enemies, checking the packed `b` unpacks to
	-- distinct (cooldown, damage_type) pairs — the whole point of the packing.
	local expect = {
		[8] = { name = "Croak", range = 700, cd = 30, dtype = Content.DMG_PIERCING, damage = 180 },
		[9] = { name = "Spicy", range = 600, cd = 36, dtype = Content.DMG_CHAOS, damage = 420 },
		[10] = { name = "Popsicle", range = 650, cd = 48, dtype = Content.DMG_SIEGE, damage = 500 },
	}
	for id, x in pairs(expect) do
		local a = Content.ENEMIES[id + 1].ability
		eq(Content.ENEMIES[id + 1].name, x.name, "ENEMIES[" .. id .. "].name")
		eq(a.variant, "RangedAttack", x.name .. ".ability.variant")
		eq(a.range, x.range, x.name .. ".ability.range")
		eq(a.cooldown_ticks, x.cd, x.name .. ".ability.cooldown_ticks")
		eq(a.damage_type, x.dtype, x.name .. ".ability.damage_type")
		eq(a.damage, x.damage, x.name .. ".ability.damage")
	end
end

do
	local m = Content.MODIFIERS[0 + 1] -- content.rs: DamageGlobalPct(1, 10)
	eq(m.name, "+10% Damage", "MODIFIERS[0].name")
	eq(m.rarity, 0, "MODIFIERS[0].rarity")
	eq(m.cost, 500, "MODIFIERS[0].cost")
	eq(#m.effects, 1, "MODIFIERS[0].effects length")
	eq(m.effects[1].variant, "DamageGlobalPct", "MODIFIERS[0].effects[0].variant")
	eq(m.effects[1].num, 1, "MODIFIERS[0].num")
	eq(m.effects[1].den, 10, "MODIFIERS[0].den")
	eq(m.ramp, nil, "MODIFIERS[0].ramp")
	eq(m.is_meta, false, "MODIFIERS[0].is_meta")
	eq(m.is_economy, false, "MODIFIERS[0].is_economy")
end
do
	local m = Content.MODIFIERS[1 + 1] -- DamageTypePct(DMG_PIERCING, 1, 10)
	eq(m.name, "Improved Piercing Attacks", "MODIFIERS[1].name")
	eq(m.effects[1].variant, "DamageTypePct", "MODIFIERS[1].effects[0].variant")
	eq(m.effects[1].damage_type, Content.DMG_PIERCING, "MODIFIERS[1].damage_type")
	eq(m.effects[1].num, 1, "MODIFIERS[1].num")
	eq(m.effects[1].den, 10, "MODIFIERS[1].den")
end
do
	local m = Content.MODIFIERS[11 + 1] -- MaxHp(2000) then MaxHpPct(25, 100)
	eq(m.name, "Imbued Masonry", "MODIFIERS[11].name")
	eq(#m.effects, 2, "ImbuedMasonry effect count")
	eq(m.effects[1].variant, "MaxHp", "ImbuedMasonry.effects[0].variant")
	eq(m.effects[1].flat, 2000, "ImbuedMasonry.effects[0].flat")
	eq(m.effects[2].variant, "MaxHpPct", "ImbuedMasonry.effects[1].variant")
	eq(m.effects[2].num, 25, "ImbuedMasonry.effects[1].num")
	eq(m.effects[2].den, 100, "ImbuedMasonry.effects[1].den")
end
do
	local m = Content.MODIFIERS[16 + 1] -- ramp: DamageGlobalPct(1, 100) every 900
	eq(m.name, "Power Generator", "MODIFIERS[16].name")
	eq(m.ramp.effect.variant, "DamageGlobalPct", "PowerGenerator.ramp.effect.variant")
	eq(m.ramp.effect.num, 1, "PowerGenerator.ramp.num")
	eq(m.ramp.effect.den, 100, "PowerGenerator.ramp.den")
	eq(m.ramp.interval_ticks, Content.RAMP_PER_ROUND, "PowerGenerator.ramp.interval_ticks")
end
do
	local m = Content.MODIFIERS[44 + 1] -- Magic Treasure: META *and* economy
	eq(m.name, "Magic Treasure", "MODIFIERS[44].name")
	eq(m.is_meta, true, "MagicTreasure.is_meta")
	eq(m.is_economy, true, "MagicTreasure.is_economy")
	eq(m.effects[1].variant, "GrantGold", "MagicTreasure.effects[0].variant")
	eq(m.ramp.effect.variant, "IncomeFlat", "MagicTreasure.ramp.effect.variant")
	eq(m.ramp.effect.flat, 5, "MagicTreasure.ramp.effect.flat")
end
do
	local m = Content.MODIFIERS[56 + 1] -- ManaShield(400, 4) ramp
	eq(m.ramp.effect.variant, "ManaShield", "MODIFIERS[56].ramp.effect.variant")
	eq(m.ramp.effect.pool, 400, "MODIFIERS[56].ramp.pool")
	eq(m.ramp.effect.regen, 4, "MODIFIERS[56].ramp.regen")
end

-- ============================ scope ids ============================
-- `{ weapon id, attack scope, range scope, rarity scope }`, from running the Rust.
local SCOPES = {
	{ 0, 0, 7, 8 },
	{ 1, 1, 7, 8 },
	{ 7, 3, 6, 9 },
	{ 8, 4, 6, 10 },
	{ 9, 5, 6, 9 },
	{ 10, 0, 7, 10 },
	{ 17, 4, 6, 10 },
	{ 24, 0, 6, 9 },
}
for _, s in ipairs(SCOPES) do
	local w = Content.WEAPONS[s[1] + 1]
	eq(Content.attackScopeId(w.attack), s[2], "attackScopeId(WEAPONS[" .. s[1] .. "])")
	eq(Content.rangeScopeId(w.range), s[3], "rangeScopeId(WEAPONS[" .. s[1] .. "])")
	eq(Content.rarityScopeId(w.rarity), s[4], "rarityScopeId(WEAPONS[" .. s[1] .. "])")
end
-- `range_scope_id`'s boundary is `<= 600`, and `rarity_scope_id` clamps at 3.
eq(Content.rangeScopeId(600), 6, "rangeScopeId(600) is short")
eq(Content.rangeScopeId(601), 7, "rangeScopeId(601) is long")
eq(Content.rarityScopeId(0), 8, "rarityScopeId(0)")
eq(Content.rarityScopeId(3), 11, "rarityScopeId(3)")
eq(Content.rarityScopeId(9), 11, "rarityScopeId clamps at 3")
-- Every scope id a weapon can produce stays inside NUM_SCOPES.
for i = 1, #Content.WEAPONS do
	local w = Content.WEAPONS[i]
	for _, sid in ipairs({
		Content.attackScopeId(w.attack),
		Content.rangeScopeId(w.range),
		Content.rarityScopeId(w.rarity),
	}) do
		checks += 1
		if sid < 0 or sid >= Content.NUM_SCOPES then
			fail("scope id " .. sid .. " out of range for WEAPONS[" .. (i - 1) .. "]")
		end
	end
end

-- ============================ is_meta / is_economy ============================
-- The exact id sets `ModifierDef::is_meta` / `is_economy` report in the Rust.
local META_IDS = { 41, 42, 43, 44, 51, 52 }
local ECONOMY_IDS = { 7, 8, 9, 18, 44, 67, 72, 73 }
eq(table.concat(metaIds, ","), table.concat(META_IDS, ","), "is_meta id set")
eq(table.concat(economyIds, ","), table.concat(ECONOMY_IDS, ","), "is_economy id set")

-- ============================ damageMultiplier ============================
-- Raws from `content::damage_multiplier`. Rows = Normal, Piercing, Magic, Siege,
-- Chaos; columns = Light, Medium, Fortified.
local DMG_MATRIX = {
	{ 65536, 65536, 32768 },
	{ 131072, 65536, 22937 },
	{ 65536, 131072, 32768 },
	{ 65536, 32768, 98304 },
	{ 65536, 65536, 65536 },
}
for dt = 0, 4 do
	for ac = 0, 2 do
		eq(
			Fixed.raw(Content.damageMultiplier(dt, ac)),
			DMG_MATRIX[dt + 1][ac + 1],
			"damageMultiplier(" .. dt .. ", " .. ac .. ")"
		)
	end
end
-- The Rust normalises its indices: `damage_type % 5`, `min(armor_class, 2)`.
eq(
	Fixed.raw(Content.damageMultiplier(5, 0)),
	Fixed.raw(Content.damageMultiplier(0, 0)),
	"damageMultiplier wraps damage_type at 5"
)
eq(
	Fixed.raw(Content.damageMultiplier(1, 9)),
	Fixed.raw(Content.damageMultiplier(1, 2)),
	"damageMultiplier clamps armor_class"
)

-- ============================ waves / ring ============================
do
	local w0 = Content.WAVES[0 + 1] -- content.rs WAVE_M0[0]: Squeakzilla @ EARLY_GRUNT_CADENCE
	eq(w0.enemy, 0, "WAVE_M0[0].enemy")
	eq(w0.cadence_ticks, Content.EARLY_GRUNT_CADENCE, "WAVE_M0[0].cadence_ticks")
	eq(w0.start_tick, 0, "WAVE_M0[0].start_tick")
	local w1 = Content.WAVES[1 + 1] -- Fanged Death, cadence 95, ungated
	eq(w1.enemy, 1, "WAVE_M0[1].enemy")
	eq(w1.cadence_ticks, 95, "WAVE_M0[1].cadence_ticks")
	eq(w1.start_tick, 0, "WAVE_M0[1].start_tick")
	local b0 = Content.BOSS_ESCORT[0 + 1] -- Squeakzilla @ 4, gated on the boss tick
	eq(b0.enemy, 0, "BOSS_ESCORT[0].enemy")
	eq(b0.cadence_ticks, 4, "BOSS_ESCORT[0].cadence_ticks")
	eq(b0.start_tick, Content.BOSS_SPAWN_TICK, "BOSS_ESCORT[0].start_tick")
end
do
	-- content.rs: SPAWN_RING[0] == v(1200, 0), i.e. Fixed::from_int(1200).
	local p = Content.SPAWN_RING[0 + 1]
	eq(Fixed.raw(p.x), 1200 * 65536, "SPAWN_RING[0].x")
	eq(Fixed.raw(p.y), 0, "SPAWN_RING[0].y")
	local q = Content.SPAWN_RING[3 + 1] -- v(-849, 849)
	eq(Fixed.raw(q.x), -849 * 65536, "SPAWN_RING[3].x")
	eq(Fixed.raw(q.y), 849 * 65536, "SPAWN_RING[3].y")
end

-- ============================ enemyHpMult ============================
-- Raws PRODUCED BY RUNNING `content::enemy_hp_mult` (see the header).
local HP_STEAM = {
	{ 0, "0000000000010000" },
	{ 1, "0000000000010000" },
	{ 2, "0000000000010000" },
	{ 100, "0000000000010024" },
	{ 1000, "000000000001016b" },
	{ 4499, "0000000000010665" },
	{ 4500, "0000000000010666" },
	{ 4501, "0000000000010667" },
	{ 5399, "0000000000010a96" },
	{ 5400, "0000000000012fea" },
	{ 5401, "0000000000012fea" },
	{ 10799, "0000000000013c7b" },
	{ 10800, "00000000000168cb" },
	{ 16200, "000000000001ac51" },
	{ 18000, "000000000001b098" },
	{ 21600, "000000000001fc7b" },
	{ 27000, "0000000000025ba6" },
	{ 32400, "000000000002cca1" },
	{ 36000, "000000000002daf4" },
	{ 37800, "00000000000352c3" },
	{ 43200, "000000000003f1ff" },
	{ 45000, "000000000003fc17" },
	{ 48600, "000000000004af0b" },
	{ 53999, "000000000004e0a4" },
	{ 54000, "0000000000058f79" },
	{ 54001, "0000000000058f79" },
	{ 60000, "0000000000058f79" },
	{ 100000, "0000000000058f79" },}
local HP_ROBLOX = {
	{ 0, "0000000000010000" },
	{ 1, "0000000000010002" },
	{ 2, "0000000000010004" },
	{ 100, "00000000000100da" },
	{ 300, "000000000001028f" },
	{ 749, "0000000000010663" },
	{ 750, "0000000000010666" },
	{ 751, "000000000001066d" },
	{ 899, "0000000000010a90" },
	{ 900, "0000000000012fea" },
	{ 901, "0000000000012fec" },
	{ 1799, "0000000000013c74" },
	{ 1800, "00000000000168cb" },
	{ 2700, "000000000001ac51" },
	{ 3000, "000000000001b098" },
	{ 3600, "000000000001fc7b" },
	{ 4500, "0000000000025ba6" },
	{ 5400, "000000000002cca1" },
	{ 6000, "000000000002daf4" },
	{ 6300, "00000000000352c3" },
	{ 7200, "000000000003f1ff" },
	{ 7500, "000000000003fc17" },
	{ 8100, "000000000004af0b" },
	{ 8999, "000000000004e088" },
	{ 9000, "0000000000058f79" },
	{ 9001, "0000000000058f79" },
	{ 12000, "0000000000058f79" },}

for _, row in ipairs(HP_STEAM) do
	eqFixed(Content.enemyHpMult(row[1], Content.TIMELINE.steam), row[2], "enemyHpMult(steam, " .. row[1] .. ")")
	-- The default IS the Steam timeline: the Rust's own compiled-in constants.
	eqFixed(Content.enemyHpMult(row[1]), row[2], "enemyHpMult(default, " .. row[1] .. ")")
end
for _, row in ipairs(HP_ROBLOX) do
	eqFixed(Content.enemyHpMult(row[1], Content.TIMELINE.roblox), row[2], "enemyHpMult(roblox, " .. row[1] .. ")")
end
-- The curve is monotonic non-decreasing across each timeline's whole run.
for _, name in ipairs({ "steam", "roblox" }) do
	local tl = Content.TIMELINE[name]
	local prev = Content.enemyHpMult(0, tl)
	local step = (tl.ramp_interval - tl.ramp_interval % 17) / 17
	local t = step
	while t <= tl.boss_spawn_tick + tl.ramp_interval do
		local cur = Content.enemyHpMult(t, tl)
		checks += 1
		if Fixed.lt(cur, prev) then
			fail("enemyHpMult(" .. name .. ") decreased at tick " .. t)
		end
		prev = cur
		t += step
	end
end
-- The endpoint is `base(ramp_intervals_to_boss)`, identical on both timelines —
-- which is exactly what the F1 rescale is designed to preserve.
eq(
	Fixed.toHex(Content.enemyHpMult(Content.TIMELINE.steam.boss_spawn_tick, Content.TIMELINE.steam)),
	Fixed.toHex(Content.enemyHpMult(Content.TIMELINE.roblox.boss_spawn_tick, Content.TIMELINE.roblox)),
	"both timelines land on the same boss endpoint"
)

-- ============================ summary ============================

print(string.format("content_test: %d checks, %d failures", checks, failures))
if failures > 0 then
	error(string.format("%d content check(s) FAILED", failures), 0)
end
