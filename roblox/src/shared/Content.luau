--!nonstrict
--[[
	Content.luau — the M0 catalog (`CONTRACTS.md` C1), exposing the tables and
	accessors of `sim-core/crates/sim/src/content.rs`. Spec: `roblox/SIM-SPEC.md`
	§S2/§S3.

	The Rust tables remain the single source of truth; `content.json` is the build
	artifact generated from them and this module is its reader. Nothing here
	invents a number — every value comes out of the JSON, and every derived value
	(`damageMultiplier`, `enemyHpMult`, the scope ids, `is_meta`/`is_economy`) is a
	transliteration of the corresponding `content.rs` function.

	WHY THE JSON IS EMBEDDED
	------------------------
	Standalone Luau has no file I/O, and this layer may not touch a Roblox global
	(`CONTRACTS.md` C5), so there is no way to *read* `src/shared/content.json`
	from here. Every sim module does a bare `require("../Content")` and reads
	constants at module scope, so the catalog must be present the moment this file
	returns. It therefore ships as a verbatim copy of `content.json` in `RAW_JSON`
	and self-loads at require time.

	THE COPY IS CHECKED, NOT TRUSTED: `test/content_test.luau` asserts `RAW_JSON`
	is byte-identical to `src/shared/content.json`, so a regenerated artifact
	cannot silently leave this module stale. `Content.load(source)` re-loads from
	any source (the JSON text, or the table Rojo's `.json` ModuleScript returns),
	for a host that would rather point the sim at the synced instance.

	INDEXING
	--------
	Content ids are 0-BASED (array position IS identity, C1 / §S3); Luau arrays are
	1-INDEXED. This module stores plain 1-indexed arrays and callers do the offset,
	matching the convention already established across the sim modules:

	    Content.WEAPONS[inst.def + 1]      -- Rust: WEAPONS[w.def as usize]
	    Content.SPAWN_RING[ringIdx + 1]    -- Rust: SPAWN_RING[ring_idx]

	Every entry also carries its own 0-based `id`, so a def can be recovered from a
	definition without tracking the offset back.

	NAMES
	-----
	Tables mirror the Rust statics in UPPER_SNAKE (`WEAPONS`, `ENEMIES`,
	`MODIFIERS`, `WAVES`/`WAVE_M0`, `BOSS_ESCORT`, `SPAWN_RING`, `DAMAGE_MATRIX`,
	`TIMELINE`); functions are camelCase per §S3. lowerCamel aliases (`weapons`,
	`enemies`, `modifiers`, `waves`, `bossEscort`, `spawnRing`, `timeline`) point
	at the SAME objects, because both spellings are in use across the sim modules.

	Scalar constants are available BOTH as the raw `content.json` passthrough
	(`Content.constants.frost_max_stacks`) and flattened to the top level in
	UPPER_SNAKE mirroring `content.rs`'s own identifiers
	(`Content.FROST_MAX_STACKS`). The flat names are DERIVED from `constants` and
	the Steam timeline at load, never hand-listed, so they cannot drift apart.

	PAYLOAD ENUMS
	-------------
	`Attack`, `WeaponAbility`, `EnemyAbility` and `ModEffect` ship as the C1
	`{tag, a, b, c}` WORDS form. They are decoded into tables that KEEP the words
	and add the Rust variant name plus named operands:

	    { variant = "Hazard", tag = 6, a = 40, b = 300, c = 90,
	      dmg = 40, radius = 300, ticks = 90 }

	`variant` is the exact Rust variant name — branch on `variant` or on `tag`.
	`tag/a/b/c` are kept because `lib.rs::checksum` hashes `effect.words()` /
	`ability.words()` verbatim: a decoded value must reproduce its own encoding.

	TIMELINES
	---------
	BOTH ship — `Content.TIMELINE.steam` and `.roblox` (`docs/10` F1). Which one is
	active is the caller's decision; only the STEAM values are flattened to bare
	names, because those are the constants the Rust itself compiles in and the
	oracle runs. The fork's rescaled numbers are reachable only via
	`Content.TIMELINE.roblox.*`, so nothing picks them up by accident.

	ENGINE INDEPENDENCE
	-------------------
	References no Roblox global. Requires only its siblings `Fixed.luau` and
	`Json.luau`, by relative path.
]]

local Fixed = require("./Fixed")
local Json = require("./Json")

-- A verbatim copy of `src/shared/content.json` (see the header). Asserted
-- byte-identical to that file by `test/content_test.luau`.
-- The catalog text. GENERATED into `ContentData.luau` by
-- `cargo run -p sim --bin export-content`, which emits it alongside
-- `content.json` from the same Rust tables.
--
-- This layer cannot read files (no Roblox globals, and it must run under a
-- bare interpreter), yet every sim module reads the catalog at module scope,
-- so the JSON has to be embedded in a module. Keeping that module GENERATED
-- rather than hand-pasted is what stops it going stale -- it already did once,
-- when adding the timeline `tick_scale_*` fields moved `content_hash` and left
-- a hand-embedded copy behind.
local RAW_JSON = require("./ContentData")

local Content = {}

--- The `content.json` text this module was built from (see the header). Exposed
--- so a consumer — or `test/content_test.luau` — can prove it is byte-identical
--- to `src/shared/content.json`.
Content.RAW_JSON = RAW_JSON

-- ============================ payload enums ============================

-- `{ tag = { variantName, operandName... } }`, mirroring each `words()` in
-- `content.rs`. Operand names are listed in `a, b, c` order.

--- `content::Attack`. C1: the tag IS `attack_scope_id`, so a port carries one
--- mapping, not two.
local ATTACK = {
	[0] = { "SingleTarget" },
	[1] = { "Splash", "radius" },
	[2] = { "Barrage", "n" },
	[3] = { "Area", "radius" },
	[4] = { "Wave", "extra" },
	[5] = { "Bounce", "n" },
}

--- `content::WeaponAbility`.
local WEAPON_ABILITY = {
	[0] = { "None" },
	[1] = { "LifeDrain", "per_hit" },
	[2] = { "ManaDrain", "per_hit" },
	[3] = { "Knockback", "dist" },
	[4] = { "Root", "ticks" },
	[5] = { "VulnOnHit", "stacks" },
	[6] = { "Hazard", "dmg", "radius", "ticks" },
	[7] = { "Summon", "kind", "hp", "damage" },
}

--- `content::EnemyAbility`. `RangedAttack` carries FOUR fields in three operand
--- slots, so `b` is unpacked separately below (C1 addendum).
local ENEMY_ABILITY = {
	[0] = { "None" },
	[1] = { "RangedAttack", "range" },
}

--- `content::ModEffect`. Tags are the `ModEffect::words` discriminants.
local MOD_EFFECT = {
	[0] = { "DamageGlobalPct", "num", "den" },
	[1] = { "DamageTypePct", "damage_type", "num", "den" },
	[2] = { "DamageMulPct", "num", "den" },
	[3] = { "AttackSpeedPct", "num", "den" },
	[4] = { "BountyPct", "num", "den" },
	[5] = { "IncomeFlat", "flat" },
	[6] = { "MaxHp", "flat" },
	[7] = { "Armor", "flat" },
	[8] = { "ManaShield", "pool", "regen" },
	[9] = { "HpRegen", "flat" },
	[10] = { "Dodge", "num" },
	[11] = { "DamageScopePct", "scope", "num", "den" },
	[12] = { "SpikesFlat", "flat" },
	[13] = { "SpikesPct", "num", "den" },
	[14] = { "GrantVulnPulse", "magnitude", "range", "interval_ticks" },
	[15] = { "HealOnKill", "flat" },
	[16] = { "HealOnPoison", "flat" },
	[17] = { "IncomePct", "num", "den" },
	[18] = { "IncomeRegenPct", "num", "den" },
	[19] = { "BountyProc", "chance_pct", "bonus_pct" },
	[20] = { "DamageVsStunnedPct", "num", "den" },
	[21] = { "DamageVsPoisonedPct", "num", "den" },
	[22] = { "PoisonDamagePct", "num", "den" },
	[23] = { "StunDurationPct", "num", "den" },
	[24] = { "GrantDuplicator", "rarity", "copies" },
	[25] = { "GrantVoucher", "rarity" },
	[26] = { "GrantGold", "gold" },
	[27] = { "HealingPct", "num", "den" },
	[28] = { "MissingHpHealPct", "num", "den" },
	[29] = { "GrantRevive", "bonus" },
	[30] = { "DamagePerWeapon", "weapon_def", "damage_type", "num_pct" },
	[31] = { "TradeMaxHpForGold", "hp_cost", "gold_gain" },
	[32] = { "TradeRegenForGold", "regen_cost", "gold_gain" },
	[33] = { "GoldPerDamagePct", "num", "den" },
	[34] = { "IncomeShieldPct", "num", "den" },
	[35] = { "MaxHpPct", "num", "den" },
	[36] = { "HpRegenPct", "num", "den" },
	[37] = { "ManaRegenPct", "num", "den" },
	[38] = { "ShieldActiveDrPct", "num", "den" },
	[39] = { "HealOnDamaged", "flat" },
	[40] = { "DamagePerMaxHp", "num", "den" },
	[41] = { "DamagePerBountyPct", "num", "den" },
	[42] = { "ShieldActiveDamagePct", "num", "den" },
	[43] = { "ManaOnKill", "flat" },
	[44] = { "ShieldBreakStun", "range", "stun_ticks" },
	[45] = { "SpikesPoison", "dps", "ticks" },
	[46] = { "StackingSpikes", "per_stack", "max_stacks" },
	[47] = { "DamageAura", "range", "cadence_ticks", "damage" },
}

--- `ModEffect::is_meta` — META items modify the shop/purchase flow rather than
--- the tank's stats. Keyed by `words()` tag.
local META_TAGS = { [24] = true, [25] = true, [26] = true, [31] = true, [32] = true }
--- `ModEffect::is_economy` — income / gold generation.
local ECONOMY_TAGS = { [5] = true, [17] = true, [26] = true }

Content.ATTACK_VARIANTS = table.freeze(ATTACK)
Content.WEAPON_ABILITY_VARIANTS = table.freeze(WEAPON_ABILITY)
Content.ENEMY_ABILITY_VARIANTS = table.freeze(ENEMY_ABILITY)
Content.MOD_EFFECT_VARIANTS = table.freeze(MOD_EFFECT)

-- ============================ decoding ============================

local function num(v, what)
	if type(v) == "string" then
		-- `Json.decode` returns the literal TEXT for |v| >= 2^53. C1 promises the
		-- catalog never carries one, so reaching here means a corrupt artifact.
		error(what .. ": value " .. v .. " is outside the exact-integer range", 0)
	end
	if type(v) ~= "number" or v % 1 ~= 0 then
		error(what .. ": expected an integer, got " .. tostring(v), 0)
	end
	return v
end

--- Decode one `{tag, a, b, c}` WORDS payload. The result KEEPS `tag/a/b/c` —
--- `lib.rs::checksum` hashes `effect.words()` / `ability.words()` verbatim, so a
--- decoded value must be able to reproduce its own encoding — and adds
--- `variant` (the exact Rust variant name) plus the named operands.
local function decodeWords(variants, w, what)
	if type(w) ~= "table" then
		error(what .. ": expected a {tag,a,b,c} object", 0)
	end
	local tag = num(w.tag, what .. ".tag")
	local spec = variants[tag]
	if spec == nil then
		error(what .. ": unknown tag " .. tag, 0)
	end
	local a = num(w.a, what .. ".a")
	local b = num(w.b, what .. ".b")
	local c = num(w.c, what .. ".c")
	local d = { variant = spec[1], tag = tag, a = a, b = b, c = c }
	local ops = { a, b, c }
	for i = 2, #spec do
		d[spec[i]] = ops[i - 1]
	end
	return d
end

local B32 = 4294967296 -- 2^32 == 1 << ENEMY_RANGED_DTYPE_SHIFT

--- `EnemyAbility::from_words`. `RangedAttack` packs four fields into three
--- operands: `b = cooldown_ticks | (damage_type << 32)` (C1 addendum), unpacked
--- with `b // 2^32` and `b % 2^32` exactly as the contract prescribes.
local function decodeEnemyAbility(w, what)
	local d = decodeWords(ENEMY_ABILITY, w, what)
	if d.variant == "RangedAttack" then
		local b = d.b
		local cooldown = b % B32
		d.cooldown_ticks = cooldown
		d.damage_type = (b - cooldown) / B32
		d.damage = d.c
	end
	return table.freeze(d)
end

local function decodeModEffect(w, what)
	local d = decodeWords(MOD_EFFECT, w, what)
	d.is_meta = META_TAGS[d.tag] == true
	d.is_economy = ECONOMY_TAGS[d.tag] == true
	return table.freeze(d)
end

-- ============================ derived accessors ============================

--- `content::attack_scope_id` — the scope id of a weapon's attack class. C1
--- pins this to the `Attack` payload tag (test-asserted on the Rust side).
function Content.attackScopeId(attack)
	return attack.tag
end

--- `content::range_scope_id` — 6 = short (<= 600), 7 = long.
function Content.rangeScopeId(range)
	if range <= Content.RANGE_SCOPE_SHORT_MAX then
		return Content.RANGE_SCOPE_SHORT
	end
	return Content.RANGE_SCOPE_LONG
end

--- `content::rarity_scope_id` — 8 = Common .. 11 = Epic (`8 + rarity.min(3)`).
function Content.rarityScopeId(rarity)
	if rarity > 3 then
		rarity = 3
	end
	return Content.RARITY_SCOPE_BASE + rarity
end

--- `ModEffect::is_meta`.
function Content.modEffectIsMeta(effect)
	return effect.is_meta
end

--- `ModEffect::is_economy`.
function Content.modEffectIsEconomy(effect)
	return effect.is_economy
end

--- `content::damage_multiplier` — the armor matrix as a `Fixed` multiplier.
--- Mirrors the Rust's index normalisation exactly: `damage_type % 5` and
--- `min(armor_class, NUM_ARMOR_CLASSES - 1)`. Both arguments are 0-based ids.
function Content.damageMultiplier(damageType, armorClass)
	local dt = damageType % Content.NUM_DAMAGE_TYPES
	local ac = armorClass
	if ac > Content.NUM_ARMOR_CLASSES - 1 then
		ac = Content.NUM_ARMOR_CLASSES - 1
	end
	return Content.DAMAGE_MATRIX[dt + 1][ac + 1]
end

-- `enemy_hp_mult`'s three per-interval factors. Pure `(num, den)` ratios out of
-- `constants` and identical on BOTH timelines — only the tick gates rescale
-- (`docs/10` F1), which is what keeps the difficulty endpoint untouched.
local G, W, J
-- `base(k)`, the post-cliff value at the start of interval `k`, memoised. The
-- Rust recomputes it per call by compounding from `Fixed::ONE`; this is a cache
-- of a pure function, so every value is bit-identical to the Rust's.
local baseCache

local function baseAt(k)
	local have = 0
	while baseCache[have + 1] ~= nil do
		have += 1
	end
	for i = have + 1, k do
		baseCache[i] = Fixed.mul(Fixed.mul(Fixed.mul(baseCache[i - 1], G), W), J)
	end
	return baseCache[k]
end

--- `content::enemy_hp_mult` — enemy HP/damage scaling at `tick`, as a `Fixed`.
--- A stepped ramp: gentle climb -> steeper "warning" window -> an instantaneous
--- step AT each interval boundary; from the boss tick on it holds the endpoint
--- tier (`base(ramp_intervals_to_boss)`), which IS that last cliff's post value.
---
--- `timeline` is OPTIONAL and defaults to `Content.TIMELINE.steam` — the
--- constants the Rust itself compiles in, so the default IS the oracle. Pass
--- `Content.TIMELINE.roblox` for the F1 rescale. Both ship; neither is baked in.
function Content.enemyHpMult(tick, timeline)
	local tl = timeline or Content.TIMELINE.steam
	if tl.ramp_interval == nil then
		error("Content.enemyHpMult: not a timeline (want Content.TIMELINE.steam / .roblox)", 2)
	end
	if tick >= tl.boss_spawn_tick then
		return baseAt(tl.ramp_intervals_to_boss)
	end

	local interval = tl.ramp_interval
	local k = (tick - tick % interval) / interval
	local lo = k * interval
	local warnStart = lo + tl.gentle_ticks
	local hi = lo + interval

	local bk = baseAt(k)
	local gentleEnd = Fixed.mul(bk, G) -- value at the end of the gentle climb
	local warnEnd = Fixed.mul(gentleEnd, W) -- pre-cliff peak (just before the step)

	local a, b, from, to
	if tick < warnStart then
		-- Gentle climb: bk -> bk*G across the first `gentle_ticks`.
		a, b, from, to = lo, warnStart, bk, gentleEnd
	else
		-- Warning sub-ramp: bk*G -> bk*G*W across the final ticks (steeper). The
		-- dramatic step (warnEnd -> base(k+1)) lands AT `hi`.
		a, b, from, to = warnStart, hi, gentleEnd, warnEnd
	end
	-- `from + (to - from) * ((tick - a) / (b - a))`, all Fixed.
	return Fixed.add(from, Fixed.mul(Fixed.sub(to, from), Fixed.fromRatio(tick - a, b - a)))
end

-- ============================ load ============================

local function buildWeapon(id, w)
	local onHit = w.on_hit
	local what = "weapon[" .. id .. "]"
	return table.freeze({
		id = id,
		name = w.name,
		rarity = num(w.rarity, what .. ".rarity"),
		cost = num(w.cost, what .. ".cost"),
		damage = num(w.damage, what .. ".damage"),
		damage_type = num(w.damage_type, what .. ".damage_type"),
		attack = table.freeze(decodeWords(ATTACK, w.attack, what .. ".attack")),
		cooldown_ticks = num(w.cooldown_ticks, what .. ".cooldown_ticks"),
		range = num(w.range, what .. ".range"),
		proj_speed = num(w.proj_speed, what .. ".proj_speed"),
		on_hit = table.freeze({
			poison_dps = num(onHit.poison_dps, what .. ".on_hit.poison_dps"),
			poison_ticks = num(onHit.poison_ticks, what .. ".on_hit.poison_ticks"),
			frost_stacks = num(onHit.frost_stacks, what .. ".on_hit.frost_stacks"),
			fire_stacks = num(onHit.fire_stacks, what .. ".on_hit.fire_stacks"),
			stun_ticks = num(onHit.stun_ticks, what .. ".on_hit.stun_ticks"),
		}),
		ability = table.freeze(decodeWords(WEAPON_ABILITY, w.ability, what .. ".ability")),
	})
end

local function buildEnemy(id, e)
	local what = "enemy[" .. id .. "]"
	if type(e.boss) ~= "boolean" then
		error(what .. ".boss: expected a JSON boolean", 0)
	end
	return table.freeze({
		id = id,
		name = e.name,
		base_hp = num(e.base_hp, what .. ".base_hp"),
		move_speed = num(e.move_speed, what .. ".move_speed"),
		contact_damage = num(e.contact_damage, what .. ".contact_damage"),
		bounty = num(e.bounty, what .. ".bounty"),
		armor_class = num(e.armor_class, what .. ".armor_class"),
		archetype = num(e.archetype, what .. ".archetype"),
		ability = decodeEnemyAbility(e.ability, what .. ".ability"),
		boss = e.boss,
	})
end

local function buildModifier(id, m)
	local what = "modifier[" .. id .. "]"
	local effects = {}
	for i, w in ipairs(m.effects) do
		effects[i] = decodeModEffect(w, what .. ".effects[" .. i .. "]")
	end
	-- `ModifierDef::is_meta` / `is_economy`: true if ANY effect is.
	local isMeta, isEconomy = false, false
	for _, e in ipairs(effects) do
		isMeta = isMeta or e.is_meta
		isEconomy = isEconomy or e.is_economy
	end
	local ramp = nil
	if m.ramp ~= nil and m.ramp ~= Json.null then
		ramp = table.freeze({
			effect = decodeModEffect(m.ramp.effect, what .. ".ramp.effect"),
			interval_ticks = num(m.ramp.interval_ticks, what .. ".ramp.interval_ticks"),
		})
	end
	return table.freeze({
		id = id,
		name = m.name,
		rarity = num(m.rarity, what .. ".rarity"),
		cost = num(m.cost, what .. ".cost"),
		effects = table.freeze(effects),
		ramp = ramp,
		is_meta = isMeta,
		is_economy = isEconomy,
	})
end

local function buildWaveTable(list, what)
	local out = {}
	for i, w in ipairs(list) do
		out[i] = table.freeze({
			enemy = num(w.enemy, what .. ".enemy"),
			cadence_ticks = num(w.cadence_ticks, what .. ".cadence_ticks"),
			start_tick = num(w.start_tick, what .. ".start_tick"),
		})
	end
	return table.freeze(out)
end

--- Flatten a `content.json` scalar table onto `Content` in UPPER_SNAKE_CASE,
--- mirroring `content.rs`'s own identifiers. DERIVED, never hand-listed, so the
--- flat names and `Content.constants` cannot drift apart. A key present twice
--- (`tick_hz` and `ramp_per_round` appear in both `constants` and the Steam
--- timeline) must agree — a disagreement is a corrupt artifact, not a choice.
local function flatten(t, sourceName)
	for key, value in pairs(t) do
		if type(key) == "string" and type(value) ~= "table" then
			local name = string.upper(key)
			local prev = rawget(Content, name)
			if prev ~= nil and prev ~= value then
				error(
					"Content: " .. sourceName .. "." .. key .. " (" .. tostring(value) ..
						") disagrees with the already-flattened " .. name .. " (" .. tostring(prev) .. ")",
					0
				)
			end
			Content[name] = value
		end
	end
end

--- (Re)load the C1 catalog. `source` is either the `content.json` TEXT or an
--- already-decoded table (what Rojo's `.json` ModuleScript returns). Populates
--- this module in place and returns it.
---
--- The module self-loads from its embedded copy at require time, so callers do
--- NOT have to call this. It exists so a Roblox host can point the sim at the
--- Rojo-synced `content.json` instance, and so tests can load a variant.
function Content.load(source)
	local doc
	if type(source) == "string" then
		doc = Json.decode(source)
	elseif type(source) == "table" then
		doc = source
	else
		error("Content.load: expected the content.json text or a decoded table", 2)
	end

	if doc.schema_version ~= 1 then
		error("Content.load: unsupported schema_version " .. tostring(doc.schema_version), 2)
	end
	local k = doc.constants
	if type(k) ~= "table" then
		error("Content.load: missing `constants`", 2)
	end

	Content.SCHEMA_VERSION = doc.schema_version
	Content.CONTENT_HASH = doc.content_hash
	Content.constants = table.freeze(k)
	Content.CONSTANTS = Content.constants

	-- Timelines. BOTH ship; which one is active is the caller's decision.
	Content.TIMELINE = table.freeze({
		steam = table.freeze(doc.timeline.steam),
		roblox = table.freeze(doc.timeline.roblox),
	})
	Content.timeline = Content.TIMELINE

	-- Flat UPPER_SNAKE constants (see `flatten`). `constants` first, then the
	-- STEAM timeline: the Rust's own module constants (`BOSS_SPAWN_TICK`,
	-- `RAMP_INTERVAL`, `GENTLE_TICKS`, `ROUND_TICKS`, the cliff gates) ARE the
	-- Steam values, and the Steam timeline is what the oracle runs. The fork's
	-- rescaled values stay reachable as `Content.TIMELINE.roblox.*` only — they
	-- are deliberately NOT flattened, so nothing silently picks up the fork's
	-- numbers by reading a bare name.
	flatten(k, "constants")
	flatten(Content.TIMELINE.steam, "timeline.steam")

	if Content.ENEMY_RANGED_DTYPE_SHIFT ~= 32 then
		-- The unpack above is written against 2^32; C1 ships the constant
		-- precisely so a port can check rather than assume.
		error("Content.load: enemy_ranged_dtype_shift is not 32", 2)
	end
	if Content.FIXED_FRAC_BITS ~= Fixed.FRAC_BITS then
		error("Content.load: content.json fixed_frac_bits disagrees with Fixed.luau", 2)
	end

	-- `content::damage_multiplier`'s matrix, `[damage_type][armor_class]`, as
	-- `Fixed` values built from the exported raw i64s. Stored 1-INDEXED; go
	-- through `Content.damageMultiplier(dt, ac)` with 0-based ids.
	local rows = {}
	for i, row in ipairs(doc.damage_matrix) do
		local cols = {}
		for j, raw in ipairs(row) do
			cols[j] = Fixed.fromRaw(num(raw, "damage_matrix[" .. i .. "][" .. j .. "]"))
		end
		rows[i] = table.freeze(cols)
	end
	Content.DAMAGE_MATRIX = table.freeze(rows)
	Content.damageMatrix = Content.DAMAGE_MATRIX

	local weapons = {}
	for i, w in ipairs(doc.weapons) do
		weapons[i] = buildWeapon(i - 1, w)
	end
	Content.WEAPONS = table.freeze(weapons)

	local enemies = {}
	for i, e in ipairs(doc.enemies) do
		enemies[i] = buildEnemy(i - 1, e)
	end
	Content.ENEMIES = table.freeze(enemies)

	local modifiers = {}
	for i, m in ipairs(doc.modifiers) do
		modifiers[i] = buildModifier(i - 1, m)
	end
	Content.MODIFIERS = table.freeze(modifiers)

	Content.WAVES = buildWaveTable(doc.waves, "waves")
	Content.WAVE_M0 = Content.WAVES -- the Rust static's name
	Content.BOSS_ESCORT = buildWaveTable(doc.boss_escort, "boss_escort")

	local ring = {}
	for i, p in ipairs(doc.spawn_ring) do
		ring[i] = table.freeze({
			x = Fixed.fromRaw(num(p.x, "spawn_ring[" .. i .. "].x")),
			y = Fixed.fromRaw(num(p.y, "spawn_ring[" .. i .. "].y")),
		})
	end
	Content.SPAWN_RING = table.freeze(ring)

	-- lowercase aliases (same objects) — both spellings are in use across the
	-- concurrently-landed sim modules; neither group of callers is wrong.
	Content.weapons = Content.WEAPONS
	Content.enemies = Content.ENEMIES
	Content.modifiers = Content.MODIFIERS
	Content.waves = Content.WAVES
	Content.bossEscort = Content.BOSS_ESCORT
	Content.spawnRing = Content.SPAWN_RING

	-- C1 ships the table sizes precisely so a port can assert its own load:
	-- index-is-identity is worthless if a row went missing on the way in.
	local function checkCount(got, want, what)
		if got ~= want then
			error("Content.load: " .. what .. " count " .. got .. " ~= constants.num_" .. what .. " " .. want, 2)
		end
	end
	checkCount(#Content.WEAPONS, k.num_weapons, "weapons")
	checkCount(#Content.ENEMIES, k.num_enemies, "enemies")
	checkCount(#Content.MODIFIERS, k.num_modifiers, "modifiers")
	if #Content.DAMAGE_MATRIX ~= Content.NUM_DAMAGE_TYPES then
		error("Content.load: damage_matrix has " .. #Content.DAMAGE_MATRIX .. " rows", 2)
	end

	-- `enemy_hp_mult` curve parameters (identical on both timelines).
	G = Fixed.fromRatio(k.ramp_gentle_num, k.ramp_gentle_den)
	W = Fixed.fromRatio(k.ramp_warn_num, k.ramp_warn_den)
	J = Fixed.fromRatio(k.ramp_jump_num, k.ramp_jump_den)
	baseCache = { [0] = Fixed.ONE }

	Content.loaded = true
	return Content
end

Content.load(RAW_JSON)

return Content
