--[[
	Combat.luau — a TRANSCRIPTION of `sim-core/crates/sim/src/combat.rs`
	(`roblox/SIM-SPEC.md` §S1: this is a transcription, not a redesign — a
	behavioral difference is a bug even when the Luau version looks better).

	Combat behavior. Content is read via `Content`, randomness comes from
	`s.rng_targeting` ONLY, all math is integer/Fixed (no floats), and iteration
	is in a stable order (by `instance_id` / `id`). No cross-module calls for
	death accounting: kills are recorded by pushing the dead enemy's `def` onto
	`s.pending_kills` (economy drains it later).

	RNG DISCIPLINE (§S4). `rng_targeting` is the only stream this module draws
	from, and it is drawn from in exactly these places, in this order:
	  * SingleTarget / Splash — one `below(#candidates)` per firing weapon;
	  * Barrage(n)            — one `below(#pool - k)` per shot, k = 0..shots-1
	                            (INCLUDING the final shot, where the argument is
	                            1 — `below(1)` still consumes a word);
	  * Bounce(n)             — one `below(#candidates)` for the first target.
	Area and Wave draw NOTHING. `rng_proc` is never touched here — the dodge roll
	lives in `defense::hit_tank`, which this module calls.

	BOSS FOCUS SKIPS DRAWS. When a BOSS is among a weapon's in-range candidates it
	replaces the random pick outright (`fire_weapons`, `boss_pick`), so:
	  * SingleTarget / Splash / Bounce — NO `below` draw at all that fire;
	  * Barrage(n)                     — the first bolt is the boss and draws
	                                     nothing; the loop starts at k = 1, so the
	                                     volley consumes `shots - 1` draws.
	This is a real change to the `rng_targeting` SEQUENCE, not an optimization:
	skipping (or not skipping) one word here desynchronizes every later draw.

	Every enemy/projectile/hazard/minion loop below walks a 1-indexed array with
	an explicit numeric `for`, never generalized iteration: iteration order is
	part of the spec here, so it is spelled out rather than inherited.
]]

local Fixed = require("../Fixed")
local Content = require("../Content")
local State = require("./State")
local Status = require("./Status")
local Defense = require("./Defense")
local Modifiers = require("./Modifiers")
local Rng = require("../Rng")

-- ===========================================================================
-- EXTERNAL API BINDINGS
-- The single place another module's names appear. If a neighbour's public name
-- differs from what is assumed here, this block is the only edit needed.
-- ===========================================================================

-- Fixed (landed, `roblox/CONTRACTS.md` §C2).
local ZERO = Fixed.ZERO
local ONE = Fixed.ONE
local fromInt = Fixed.fromInt
local fmul = Fixed.mul
local fadd = Fixed.add
local fdiv = Fixed.div
local flt = Fixed.lt
local fle = Fixed.le
local feq = Fixed.eq
local floorToInt = Fixed.floorToInt

-- Rng (landed, §C3). `below` mutates the stream in place.
local below = Rng.below

-- State (`state.rs`): Vec2 math, id allocation, the damage scoreboard, and the
-- two Tank chokepoints (`Tank::heal` / `Tank::restore_mana`).
local vecDistSq = State.Vec2.distSq -- Vec2::dist_sq(self, o) -> Fixed
local vecStepToward = State.Vec2.stepToward -- Vec2::step_toward(self, target, max_step) -> Vec2
local vecStepAway = State.Vec2.stepAway -- Vec2::step_away(self, from, step) -> Vec2
local allocEntityId = State.allocEntityId -- ArenaState::alloc_entity_id(s) -> u32
local recordPlayerDamage = State.recordPlayerDamage -- ArenaState::record_player_damage(s, i64)
local tankHeal = State.Tank.heal -- Tank::heal(tank, i64)
local tankRestoreMana = State.Tank.restoreMana -- Tank::restore_mana(tank, i64)

-- Status (`status.rs`).
local applyOnHit = Status.applyOnHit
local vulnerabilityMult = Status.vulnerabilityMult
local isImmobile = Status.isImmobile
local moveSpeedMult = Status.moveSpeedMult
local reapDead = Status.reapDead

-- Defense (`defense.rs`).
local hitTank = Defense.hitTank

-- Modifiers (`modifiers.rs`). All take the `Modifiers` record as the receiver.
local weaponDamageMult = Modifiers.weaponDamageMult -- (mods, wdef) -> Fixed
local selfScalingAdd = Modifiers.selfScalingAdd -- (mods, dmg_type, weapons) -> Fixed
local dynamicGlobalAdd = Modifiers.dynamicGlobalAdd -- (mods, tank, economy) -> Fixed
local scaleOnHit = Modifiers.scaleOnHit -- (mods, on_hit) -> StatusOnHit (a NEW table)
local attackSpeedMult = Modifiers.attackSpeedMult -- (mods) -> Fixed

-- ===========================================================================
-- CONTENT ENCODING
-- `content.json` ships payload enums in the `words()` form `{tag, a, b, c}`
-- (`roblox/CONTRACTS.md` §C1). The tags below are pinned by `Attack::words` /
-- `WeaponAbility::words` / `EnemyAbility::words` in `content.rs`; they are a
-- property of the file format, not of `Content.luau`.
-- ===========================================================================

-- `Attack` (tag is deliberately identical to `attack_scope_id`).
local ATTACK_SINGLE_TARGET = 0
local ATTACK_SPLASH = 1 -- a = radius
local ATTACK_BARRAGE = 2 -- a = n
local ATTACK_AREA = 3 -- a = radius
local ATTACK_WAVE = 4 -- a = extra reach
local ATTACK_BOUNCE = 5 -- a = n

-- `WeaponAbility`.
local ABILITY_NONE = 0
local ABILITY_LIFE_DRAIN = 1 -- a = per_hit
local ABILITY_MANA_DRAIN = 2 -- a = per_hit
local ABILITY_KNOCKBACK = 3 -- a = dist
local ABILITY_ROOT = 4 -- a = ticks
local ABILITY_VULN_ON_HIT = 5 -- a = stacks
local ABILITY_HAZARD = 6 -- a = dmg, b = radius, c = ticks
local ABILITY_SUMMON = 7 -- a = kind, b = hp, c = damage

-- `EnemyAbility::RangedAttack`: four fields packed into three operands —
-- `b = cooldown_ticks | (damage_type << 32)` (§C1 addendum).
local ENEMY_ABILITY_RANGED = 1
local U32 = 4294967296

-- `WeaponAbility::None` / `StatusOnHit::NONE`, as the hazard/aura/minion damage
-- sites pass them. Frozen: they are shared, never mutated.
local NO_ABILITY = table.freeze({ tag = ABILITY_NONE, a = 0, b = 0, c = 0 })
local STATUS_ON_HIT_NONE = table.freeze({
	poison_dps = 0,
	poison_ticks = 0,
	frost_stacks = 0,
	fire_stacks = 0,
	stun_ticks = 0,
})

local U16_MAX = 65535

-- Scalar content constants. `Content.luau` may expose them either as named
-- fields (the `content.rs` spelling) or under `constants` (the `content.json`
-- spelling); `Status.luau` hedges the same way, so mirror it rather than pin one
-- shape and break on the other.
local function contentConst(name: string, snake: string): number
	local v = Content[name]
	if v == nil and Content.constants ~= nil then
		v = Content.constants[snake]
	end
	if v == nil then
		error("Combat: Content is missing " .. name, 2)
	end
	return v
end

local BOSS_CONTACT_CADENCE = contentConst("BOSS_CONTACT_CADENCE", "boss_contact_cadence")
local DMG_MAGIC = contentConst("DMG_MAGIC", "dmg_magic")
local BOSS_SPAWN_TICK = contentConst("BOSS_SPAWN_TICK", "boss_spawn_tick")
local NUM_DAMAGE_TYPES = contentConst("NUM_DAMAGE_TYPES", "num_damage_types")

-- THE BOSS ENCOUNTER — ADAPTIVE ARMOR PLATES (`content.rs`, the BOSS ENCOUNTER
-- block). The boss is not immune to weapon fire, it is PLATED: one damage-type
-- plate is exposed at a time and takes `BOSS_EXPOSED_NUM/DEN × coverage`, every
-- other plate takes `BOSS_ARMORED_NUM/DEN`, and a `Clear` BREACHES all five for
-- `BOSS_BREACH_TICKS`.
--
-- Every one of these is read from the C1 artifact, never pinned here. This module
-- reimplements the plate/breach/enrage ARITHMETIC (the `content.rs` functions are
-- not part of the exported catalog), so a local copy of a BALANCE NUMBER would
-- drift silently the first time the encounter is retuned — precisely the failure
-- class the export pipeline exists to prevent. A missing name errors loudly.
local BOSS_PLATE_TICKS = contentConst("BOSS_PLATE_TICKS", "boss_plate_ticks")
local BOSS_EXPOSED_NUM = contentConst("BOSS_EXPOSED_NUM", "boss_exposed_num")
local BOSS_EXPOSED_DEN = contentConst("BOSS_EXPOSED_DEN", "boss_exposed_den")
local BOSS_ARMORED_NUM = contentConst("BOSS_ARMORED_NUM", "boss_armored_num")
local BOSS_ARMORED_DEN = contentConst("BOSS_ARMORED_DEN", "boss_armored_den")
local BOSS_BREACH_TICKS = contentConst("BOSS_BREACH_TICKS", "boss_breach_ticks")
local CLEAR_COOLDOWN_TICKS = contentConst("CLEAR_COOLDOWN_TICKS", "clear_cooldown_ticks")
local BOSS_ENRAGE_INTERVAL = contentConst("BOSS_ENRAGE_INTERVAL", "boss_enrage_interval")
local BOSS_ENRAGE_STEP_NUM = contentConst("BOSS_ENRAGE_STEP_NUM", "boss_enrage_step_num")
local BOSS_ENRAGE_STEP_DEN = contentConst("BOSS_ENRAGE_STEP_DEN", "boss_enrage_step_den")

-- Content ids are 0-based (array position IS identity, §C1/§S3); Luau arrays
-- are 1-indexed. These two accessors are the ONLY place that offset lives.
local function weaponDef(def: number)
	return Content.WEAPONS[def + 1]
end

local function enemyDef(def: number)
	return Content.ENEMIES[def + 1]
end

--- Rust `Vec2` is `Copy`; every by-value capture/assignment here clones so a
--- stored position can never alias a live entity's.
local function v2(v)
	return { x = v.x, y = v.y }
end

local function vecEq(a, b): boolean
	return feq(a.x, b.x) and feq(a.y, b.y)
end

--- `Fixed::scale_i64` as a plain Luau number (i64 quantities are numbers, §S4).
local function scaleI64(m, v: number): number
	return Fixed.toInt(Fixed.scaleI64(m, v))
end

local Combat = {}

-- ===========================================================================
-- THE BOSS ENCOUNTER
-- Transcriptions of the `content.rs` boss functions (`arsenal_coverage`,
-- `boss_enrage_mult`, `boss_exposed_type`, `boss_damage_mult`) plus
-- `combat::boss_breached` / `BossVuln`. The `content.rs` half lives here rather
-- than in `Content.luau` only because the C1 artifact does not export the
-- constants they read; the arithmetic is a straight transcription either way.
-- ===========================================================================

--- `content::boss_exposed_type` — which of the boss's five plates is exposed at
--- `tick`: the damage type that takes the amplified rate. Rotates every
--- `BOSS_PLATE_TICKS` from `BOSS_SPAWN_TICK`. Pure integer function of the tick —
--- no state, no RNG. The subtraction SATURATES (Rust `saturating_sub`), so a boss
--- placed before the boss tick gets plate 0 rather than a negative index.
local function bossExposedType(tick: number): number
	local since = tick - BOSS_SPAWN_TICK
	if since < 0 then
		since = 0
	end
	return math.floor(since / BOSS_PLATE_TICKS) % NUM_DAMAGE_TYPES
end

--- `content::boss_enrage_mult` — `1 + steps·(STEP_NUM/STEP_DEN)`, where `steps =
--- (tick -| BOSS_SPAWN_TICK) / BOSS_ENRAGE_INTERVAL`. `Fixed`, so it composes with
--- `enemyHpMult` the way the base contact scaling does. SATURATING subtraction
--- again: no enrage before the boss tick.
local function bossEnrageMult(tick: number)
	local since = tick - BOSS_SPAWN_TICK
	if since < 0 then
		since = 0
	end
	local steps = math.floor(since / BOSS_ENRAGE_INTERVAL)
	return fadd(ONE, Fixed.fromRatio(steps * BOSS_ENRAGE_STEP_NUM, BOSS_ENRAGE_STEP_DEN))
end

--- `content::arsenal_coverage` — the number of DISTINCT damage types across the
--- player's owned weapons, clamped to `1..=5`. Counted into a fixed 5-slot bitmap,
--- so it is O(n) and independent of iteration order. Clamped at 1 so a weaponless
--- tank's aura/hazard damage can never end up with the exposed plate weaker than
--- the armored ones.
local function arsenalCoverage(weapons): number
	local seen = table.create(NUM_DAMAGE_TYPES, false)
	for wi = 1, #weapons do
		seen[weaponDef(weapons[wi].def).damage_type % NUM_DAMAGE_TYPES + 1] = true
	end
	local n = 0
	for i = 1, NUM_DAMAGE_TYPES do
		if seen[i] then
			n += 1
		end
	end
	if n < 1 then
		n = 1
	end
	return n
end

--- `content::boss_damage_mult` — the multiplier applied to one hit of
--- `damageType` on the BOSS at `tick`, given the arsenal's `coverage` and whether
--- a recent `Clear` has `breached` every plate. REPLACES the normal armor matrix
--- for boss targets.
---
---   exposed (or breached): `BOSS_EXPOSED_NUM × coverage / BOSS_EXPOSED_DEN`
---   armored:               `BOSS_ARMORED_NUM / BOSS_ARMORED_DEN`
local function bossDamageMult(damageType: number, tick: number, breached: boolean, coverage: number)
	if breached or damageType % NUM_DAMAGE_TYPES == bossExposedType(tick) then
		-- Rust `coverage.clamp(1, 5)`.
		local c = coverage
		if c < 1 then
			c = 1
		elseif c > NUM_DAMAGE_TYPES then
			c = NUM_DAMAGE_TYPES
		end
		return Fixed.fromRatio(BOSS_EXPOSED_NUM * c, BOSS_EXPOSED_DEN)
	end
	return Fixed.fromRatio(BOSS_ARMORED_NUM, BOSS_ARMORED_DEN)
end

--- `combat::boss_breached` — is the boss currently BREACHED (all plates open) by a
--- recent `Clear`?
---
--- DERIVED, deliberately: `Input.apply` sets `tank.clear_cooldown_end = clear_tick
--- + CLEAR_COOLDOWN_TICKS`, so the last `Clear` landed at `clear_cooldown_end -
--- CLEAR_COOLDOWN_TICKS` and the breach runs for `BOSS_BREACH_TICKS` from there:
---
---     breached  <=>  clear_cooldown_end > tick
---               and  (clear_cooldown_end - tick) + BOSS_BREACH_TICKS > CLEAR_COOLDOWN_TICKS
---
--- Reading it off the existing cooldown field means the breach window adds NO new
--- per-arena state, so the state layout and `Arena.checksum` are untouched — do
--- not add a field for it. All-integer, no wall-clock, no RNG. The first guard
--- also keeps the u32 subtraction from underflowing and makes a never-Cleared tank
--- (`clear_cooldown_end == 0`) unbreached.
local function bossBreached(s): boolean
	local endTick = s.tank.clear_cooldown_end
	return endTick > s.tick and (endTick - s.tick) + BOSS_BREACH_TICKS > CLEAR_COOLDOWN_TICKS
end

--- `combat::BossVuln::of` — the boss's live vulnerability at one damage site:
--- which armor plate is exposed, and whether a recent `Clear` has BREACHED all
--- five. Captured once per damage pass (every term is a pure function of state
--- that cannot change mid-pass) and consulted at impact, exactly like `cond`.
local function bossVulnOf(s)
	return {
		tick = s.tick,
		breached = bossBreached(s),
		-- Distinct damage types in the arsenal (1..=5) — the term that makes
		-- BREADTH pay at the boss. A plate rotation alone cannot.
		coverage = arsenalCoverage(s.weapons),
	}
end

--- `combat::BossVuln::mult` — mitigation for one hit of `damageType` on the boss.
local function bossVulnMult(boss, damageType: number)
	return bossDamageMult(damageType, boss.tick, boss.breached, boss.coverage)
end

-- ===========================================================================
-- Ability accumulation
-- ===========================================================================

--- Accumulates the TANK-side / world-side effects of a weapon's signature
--- ability across all the enemies one attack damages, so they can be applied
--- once after the (tank-read-only) enemy loop ends. Enemy-LOCAL ability effects
--- (vuln stacks, knockback displacement, root) are applied directly to the
--- enemy inside `applyWeaponHit`; only effects that touch the tank or spawn
--- world entities are deferred here.
---
--- `summons` holds one entry per ally to raise: position of the slain enemy
--- plus the minion's seed stats (the per-strike `damage_type` is taken from the
--- killing weapon at flush time, so every minion of one cast shares it).
local function newAccum()
	return {
		-- Total HP to heal the tank (life-drain), summed per damaged enemy.
		heal = 0,
		-- Total Mana-Shield to restore (mana-drain), summed per damaged enemy.
		mana = 0,
		-- Where to drop a hazard (the first enemy this attack damaged), if any.
		hazard_at = nil,
		-- Corpses to raise as allies (Summon weapons that killed an enemy).
		summons = {},
	}
end

-- Tuning for summoned allies (Larvae / Spores).
local MAX_MINIONS = 16 -- global cap on living minions
local MINION_LIFETIME = 450 -- ~15s before a minion vanishes
local MINION_ATTACK_CD = 30 -- ticks between a minion's strikes (~1/s)
local MINION_REACH = 160 -- melee reach (world units)
local MINION_SPEED = 18 -- step toward the target per tick (out of reach)

--- Apply the deferred tank/world effects of `ability` (placed at `damageType`
--- for hazards). Called once after a damage site.
local function flushAccum(s, accum, ability, damageType: number)
	if accum.heal > 0 and not s.dead then
		tankHeal(s.tank, accum.heal)
	end
	if accum.mana > 0 then
		tankRestoreMana(s.tank, accum.mana)
	end
	if ability.tag == ABILITY_HAZARD and accum.hazard_at ~= nil then
		local id = allocEntityId(s)
		table.insert(s.hazards, {
			id = id,
			pos = accum.hazard_at,
			dmg = ability.a,
			damage_type = damageType,
			radius = ability.b,
			ticks_left = ability.c,
		})
	end
	-- Raise an ally from each corpse this attack made, up to the global cap.
	for si = 1, #accum.summons do
		local req = accum.summons[si]
		if #s.minions >= MAX_MINIONS then
			break
		end
		local id = allocEntityId(s)
		table.insert(s.minions, {
			id = id,
			pos = req.pos,
			kind = req.kind,
			hp = req.hp,
			damage = req.damage,
			damage_type = damageType,
			next_attack_tick = s.tick,
			expire_tick = s.tick + MINION_LIFETIME,
		})
	end
end

-- ===========================================================================
-- Damage application
-- ===========================================================================

--- Player's target-conditional damage bonuses, captured once per tick (they
--- don't change mid-tick) and applied at impact against each enemy's live
--- status.
local function condOf(s)
	return { vs_stunned = s.modifiers.vs_stunned, vs_poisoned = s.modifiers.vs_poisoned }
end

--- Multiplier for this enemy: `1 + Σ matching conditional bonuses`.
local function condMult(cond, e)
	local m = ONE
	if e.status.stun_ticks > 0 then
		m = fadd(m, cond.vs_stunned)
	end
	if e.status.poison_ticks > 0 then
		m = fadd(m, cond.vs_poisoned)
	end
	return m
end

--- Execute a weapon's signature ability against one enemy it just damaged.
--- Enemy-LOCAL effects (vulnerability stacks, knockback, root) mutate the enemy
--- here; tank/world effects accumulate into `accum` for a single post-loop
--- flush. Deterministic: integer/Fixed math, no RNG.
local function applyAbilityOnHit(e, ability, tankPos, accum)
	local tag = ability.tag
	if tag == ABILITY_NONE then
		return
	elseif tag == ABILITY_SUMMON then
		-- Raise from the corpse only if THIS hit was the killing blow.
		-- (`applyWeaponHit` has already subtracted the damage.)
		if e.hp <= 0 then
			table.insert(accum.summons, { pos = v2(e.pos), kind = ability.a, hp = ability.b, damage = ability.c })
		end
	elseif tag == ABILITY_LIFE_DRAIN then
		accum.heal += ability.a
	elseif tag == ABILITY_MANA_DRAIN then
		accum.mana += ability.a
	elseif tag == ABILITY_VULN_ON_HIT then
		-- u16 saturating_add.
		local v = e.status.vuln_stacks + ability.a
		e.status.vuln_stacks = if v > U16_MAX then U16_MAX else v
	elseif tag == ABILITY_ROOT then
		-- Root = immobilize + poisoned flag, applied directly (does NOT scale
		-- with +% Stun Duration, per the source). We reuse `stun_ticks` for
		-- the immobilize and ensure a poison marker so "Rooted enemies are
		-- considered Stunned & Poisoned".
		local ticks = ability.a
		if ticks > e.status.stun_ticks then
			e.status.stun_ticks = ticks
		end
		if e.status.poison_ticks < ticks then
			e.status.poison_ticks = ticks
			-- A token DoT so the poisoned condition holds without overwriting
			-- a stronger existing poison.
			if e.status.poison_dps == 0 then
				e.status.poison_dps = 1
			end
		end
	elseif tag == ABILITY_KNOCKBACK then
		-- Shove the enemy directly away from the tank by `dist` units.
		local away = vecStepAway(e.pos, tankPos, fromInt(ability.a))
		e.pos = v2(away)
	elseif tag == ABILITY_HAZARD then
		-- Record the FIRST damaged enemy's position as the drop point.
		if accum.hazard_at == nil then
			accum.hazard_at = v2(e.pos)
		end
	end
end

--- Apply one weapon hit to an enemy: `base × armor-matrix × modifier-stack ×
--- fire-vulnerability × target-conditional` damage, then the weapon's on-hit
--- status.
---
--- The BOSS takes a different path (`docs/11 §11.5` #1): plate mitigation
--- (`bossVulnMult`) replaces the armor matrix, and it is still immune to STATUS
--- and to weapon ABILITIES — no poison/fire/frost/stun/root/knockback, no
--- life-drain, no corpse summons. That asymmetry is load-bearing, not laziness: a
--- 60-tick stun aura on an 18-tick cooldown (Shocker) would otherwise pin the boss
--- forever and delete the climax, and letting Fire/Poison DoTs tick on a
--- multi-million HP pool would reintroduce a damage source the plates cannot gate
--- at all.
---
--- Returns the integer damage subtracted from the enemy, for the caller's
--- damage/Bloodmoney accounting.
local function applyWeaponHit(e, base: number, damageType: number, modMult, cond, boss, onHit, ability, tankPos, accum): number
	local edef = enemyDef(e.def)
	if edef.boss then
		-- Plated, not immune. No armor matrix, no status vulnerability (the boss
		-- carries no statuses), no on-hit, no ability — just mitigated damage.
		local dmg = scaleI64(fmul(bossVulnMult(boss, damageType), modMult), base)
		e.hp -= dmg
		return dmg
	end
	local armor = Content.damageMultiplier(damageType, edef.armor_class)
	local vuln = vulnerabilityMult(e)
	local dmg = scaleI64(fmul(fmul(fmul(armor, modMult), vuln), condMult(cond, e)), base)
	e.hp -= dmg
	applyOnHit(e, onHit)
	applyAbilityOnHit(e, ability, tankPos, accum)
	return dmg
end

-- ===========================================================================
-- Phase 4 — fire weapons
-- ===========================================================================

--- Phase 4: each ready weapon (`s.tick >= next_fire_tick`) picks a target —
--- RANDOM among enemies within `range` (via `rng_targeting.below(n)`, matching
--- the source's "attack at random") — and emits a `Projectile` toward it
--- (`damage`, `damage_type`, `splash_radius` = 0 for SingleTarget or
--- `Splash(r)` radius, `speed` = proj_speed). Sets `next_fire_tick = s.tick +
--- cooldown_ticks`. No enemy in range ⇒ do not fire (DO NOT ADVANCE COOLDOWN —
--- load-bearing; see the `continue` below).
function Combat.fireWeapons(s)
	-- Collect new projectiles first so the ordering of id allocation matches the
	-- Rust (hazards/minions from `flushAccum` get their ids DURING the weapon
	-- loop; projectiles get theirs after it). Weapons fire in their existing
	-- (instance_id) order; candidate enemies are gathered in stable id order
	-- before any random pick.
	local newProjectiles = {}
	local anyInstantDamage = false
	-- Damage dealt by instant attacks this call (projectile damage is recorded
	-- on impact in `advanceProjectiles`).
	local instantDamage = 0
	-- Target-conditional bonuses are constant across this tick's fires.
	local cond = condOf(s)
	local boss = bossVulnOf(s)

	for wi = 1, #s.weapons do
		local def = s.weapons[wi].def
		if s.tick < s.weapons[wi].next_fire_tick then
			continue
		end
		local wdef = weaponDef(def)
		local range = fromInt(wdef.range)
		local rangeSq = fmul(range, range)

		-- Candidates: enemies within range, in stable id order. Entries are
		-- 1-based indices into `s.enemies`.
		local candidates = {}
		for ei = 1, #s.enemies do
			if fle(vecDistSq(s.tank.pos, s.enemies[ei].pos), rangeSq) then
				table.insert(candidates, ei)
			end
		end
		if #candidates == 0 then
			-- No enemy in range: do not fire, do not advance cooldown.
			continue
		end

		-- Full per-weapon multiplier (global + type + scope) is resolved HERE,
		-- at fire time. For projectiles we bake it into the carried damage; for
		-- instant attacks we pass it to applyWeaponHit.
		-- Static multiplier (global + type + scope) plus live self-scaling
		-- ("+% per owned weapon"). Folding identity:
		-- (1 + add_static + add_self)×mul = weapon_damage_mult + add_self×mul.
		local staticMult = weaponDamageMult(s.modifiers, wdef)
		local addSelf = selfScalingAdd(s.modifiers, wdef.damage_type, s.weapons)
		-- DYNAMIC global-damage scalers resolved from the LIVE tank/economy (per
		-- 2000 Max HP / per 50% Bounty / while Mana Shield active). GLOBAL
		-- additive, so they thread in exactly like `addSelf` — additive, then
		-- `×mul_global`.
		local addDyn = dynamicGlobalAdd(s.modifiers, s.tank, s.economy)
		local wmult = fadd(staticMult, fmul(fadd(addSelf, addDyn), s.modifiers.mul_global))
		local baked = scaleI64(wmult, wdef.damage)
		-- Poison-damage / stun-duration scalers depend only on the player's
		-- modifiers, so (like base damage) they bake into the hit at fire time.
		local onHit = scaleOnHit(s.modifiers, wdef.on_hit)
		local ability = wdef.ability
		local tankPos = s.tank.pos
		-- Instant-attack abilities (Area/Wave/Bounce) accumulate here, flushed
		-- once after this weapon's instant hits resolve.
		local accum = newAccum()

		local function newProj(targetIdx: number, splash)
			local e = s.enemies[targetIdx]
			table.insert(newProjectiles, {
				id = 0, -- assigned after the loop
				pos = v2(s.tank.pos),
				target = e.id,
				last_target_pos = v2(e.pos),
				damage = baked,
				damage_type = wdef.damage_type,
				splash_radius = splash,
				speed = fromInt(wdef.proj_speed),
				on_hit = onHit,
				ability = ability,
			})
		end

		-- BOSS FOCUS (`docs/11 §11.5` #1). While The Hippocrate is in range it is
		-- this weapon's target, ahead of the escort chaff. "Attack at random" still
		-- governs every other tick of the match; the boss is the one exception,
		-- because it is a building-sized enemy planted on the tank.
		--
		-- This is not flavor, it is THE fix. Random targeting alone made the arsenal
		-- invisible to the boss: with ~50 escort enemies alive, a random pick lands
		-- on the boss ~2% of the time, and a measured 80-seed sweep with boss
		-- mitigation switched OFF ENTIRELY still had the arsenal contribute under 1M
		-- of the boss's HP bar (33M at the time) across a 290 s fight — the kill was
		-- ~30 Clears start to finish. Letting weapons damage the boss is worthless
		-- without letting them AIM at it.
		--
		-- The price, deliberately paid: single-target guns stop answering the escort
		-- while the boss is up, so the escort swarm is what threatens to kill you
		-- during the climax. That is the fight's central tension.
		--
		-- RNG: a focused fire REPLACES the `below` draw, it does not precede it. See
		-- the BOSS FOCUS SKIPS DRAWS note in the module header — the skipped word is
		-- part of the oracle.
		local bossPick = nil
		for ci = 1, #candidates do
			if enemyDef(s.enemies[candidates[ci]].def).boss then
				bossPick = candidates[ci]
				break
			end
		end

		local atk = wdef.attack
		local tag = atk.tag
		if tag == ATTACK_SINGLE_TARGET then
			local pick = bossPick
			if pick == nil then
				pick = candidates[below(s.rng_targeting, #candidates) + 1]
			end
			newProj(pick, ZERO)
		elseif tag == ATTACK_SPLASH then
			local pick = bossPick
			if pick == nil then
				pick = candidates[below(s.rng_targeting, #candidates) + 1]
			end
			newProj(pick, fromInt(atk.a))
		elseif tag == ATTACK_BARRAGE then
			-- N distinct random in-range targets (partial Fisher–Yates). With the
			-- boss in range it claims the FIRST bolt and the remaining N-1 still
			-- scatter — a volley spreads, so only part of it finds the boss.
			local pool = table.clone(candidates)
			local shots = math.min(atk.a, #pool)
			-- `k` is the Rust's 0-based index; `pool` is 1-based.
			local k0 = 0
			if bossPick ~= nil then
				local at = nil
				for i = 1, #pool do
					if pool[i] == bossPick then
						at = i
						break
					end
				end
				if at == nil then
					-- Unreachable: `bossPick` came out of `candidates`, which `pool`
					-- is a clone of. Rust `.unwrap()`s the same search.
					error("Combat.fireWeapons: boss focus target missing from the barrage pool", 2)
				end
				pool[1], pool[at] = pool[at], pool[1]
				newProj(pool[1], ZERO)
				k0 = 1
			end
			for k = k0, shots - 1 do
				local j = k + below(s.rng_targeting, #pool - k)
				pool[k + 1], pool[j + 1] = pool[j + 1], pool[k + 1]
				newProj(pool[k + 1], ZERO)
			end
		elseif tag == ATTACK_AREA then
			-- Instant pulse around the tank.
			local r = fromInt(atk.a)
			local r2 = fmul(r, r)
			for ei = 1, #s.enemies do
				local e = s.enemies[ei]
				if fle(vecDistSq(tankPos, e.pos), r2) then
					instantDamage += applyWeaponHit(
						e,
						wdef.damage,
						wdef.damage_type,
						wmult,
						cond,
						boss,
						onHit,
						ability,
						tankPos,
						accum
					)
				end
			end
			anyInstantDamage = true
		elseif tag == ATTACK_WAVE then
			-- Instant sweep out to range + extra around the tank.
			local reach = fadd(range, fromInt(atk.a))
			local r2 = fmul(reach, reach)
			for ei = 1, #s.enemies do
				local e = s.enemies[ei]
				if fle(vecDistSq(tankPos, e.pos), r2) then
					instantDamage += applyWeaponHit(
						e,
						wdef.damage,
						wdef.damage_type,
						wmult,
						cond,
						boss,
						onHit,
						ability,
						tankPos,
						accum
					)
				end
			end
			anyInstantDamage = true
		elseif tag == ATTACK_BOUNCE then
			-- Random first target (the boss if it is in range), then the N-1 nearest
			-- OTHER enemies to it.
			local first = bossPick
			if first == nil then
				first = candidates[below(s.rng_targeting, #candidates) + 1]
			end
			local origin = s.enemies[first].pos
			local order = {}
			for i = 1, #s.enemies do
				if i ~= first then
					table.insert(order, i)
				end
			end
			-- Sort by (distance to origin, id) for a deterministic chain. Fixed
			-- comparison is exactly raw comparison, so this reproduces the
			-- Rust's `(dist_sq.raw(), id.0)` key; the id makes the order total,
			-- so the sort's (in)stability cannot matter.
			local d2 = {}
			for oi = 1, #order do
				local i = order[oi]
				d2[i] = vecDistSq(s.enemies[i].pos, origin)
			end
			table.sort(order, function(a, b)
				local c = Fixed.compare(d2[a], d2[b])
				if c ~= 0 then
					return c < 0
				end
				return s.enemies[a].id < s.enemies[b].id
			end)
			local targets = { first }
			local take = math.max(atk.a - 1, 0)
			for i = 1, math.min(take, #order) do
				table.insert(targets, order[i])
			end
			for tj = 1, #targets do
				local ti = targets[tj]
				instantDamage += applyWeaponHit(
					s.enemies[ti],
					wdef.damage,
					wdef.damage_type,
					wmult,
					cond,
					boss,
					onHit,
					ability,
					tankPos,
					accum
				)
			end
			anyInstantDamage = true
		end

		-- Apply this weapon's deferred ability effects (life/mana drain heal,
		-- hazard placement). A no-op for projectile attacks (their abilities
		-- resolve at impact) and for ability-less weapons.
		flushAccum(s, accum, ability, wdef.damage_type)

		-- Effective cooldown is reduced by the attack-speed modifier.
		local asm = attackSpeedMult(s.modifiers)
		local cd = math.max(floorToInt(fdiv(fromInt(wdef.cooldown_ticks), asm)), 1)
		s.weapons[wi].next_fire_tick = s.tick + cd
	end

	-- Record instant-attack damage (scoreboard + Bloodmoney) now that no enemy
	-- borrow is held.
	recordPlayerDamage(s, instantDamage)

	for pi = 1, #newProjectiles do
		local p = newProjectiles[pi]
		p.id = allocEntityId(s)
		table.insert(s.projectiles, p)
	end

	-- Instant attacks (Area/Wave/Bounce) can kill: reap the dead now so bounty
	-- is awarded this tick, preserving id order. Fire-stacked deaths explode.
	if anyInstantDamage then
		reapDead(s)
	end
end

-- ===========================================================================
-- Phase 5 — advance projectiles
-- ===========================================================================

--- Phase 5: move each projectile toward its target by `speed`
--- (`Vec2::step_toward`). On arrival (reached target pos) apply damage: single
--- target hits the target enemy; `splash_radius > 0` hits all enemies within
--- that radius of the impact point. Remove projectiles that have arrived; if a
--- projectile's target enemy no longer exists, detonate at `last_target_pos`
--- (splash) or just remove it (single target). Keeps `s.enemies` in id order.
function Combat.advanceProjectiles(s)
	-- Move each projectile toward its target, detect arrival, and on arrival
	-- apply damage. We collect "impacts" while iterating then apply hp changes /
	-- removals afterward in a stable pass.
	local impacts = {}
	local survivors = {}

	local projectiles = s.projectiles
	s.projectiles = {}
	for pi = 1, #projectiles do
		local p = projectiles[pi]
		-- Resolve current target position (if the enemy still exists).
		local targetPos = nil
		for ei = 1, #s.enemies do
			if s.enemies[ei].id == p.target then
				targetPos = s.enemies[ei].pos
				break
			end
		end

		if targetPos ~= nil then
			local tpos = targetPos
			p.last_target_pos = v2(tpos)
			local moved = vecStepToward(p.pos, tpos, p.speed)
			if vecEq(moved, tpos) then
				-- Arrived: detonate at the target position.
				table.insert(impacts, {
					point = v2(tpos),
					target = p.target,
					damage = p.damage,
					damage_type = p.damage_type,
					splash_radius = p.splash_radius,
					on_hit = p.on_hit,
					ability = p.ability,
				})
			else
				p.pos = v2(moved)
				table.insert(survivors, p)
			end
		else
			-- Target gone: splash projectiles detonate at last known pos;
			-- single-target projectiles simply vanish.
			if flt(ZERO, p.splash_radius) then
				table.insert(impacts, {
					point = v2(p.last_target_pos),
					target = p.target,
					damage = p.damage,
					damage_type = p.damage_type,
					splash_radius = p.splash_radius,
					on_hit = p.on_hit,
					ability = p.ability,
				})
			end
			-- either way, projectile is removed (not pushed to survivors).
		end
	end

	s.projectiles = survivors

	-- Apply impacts in order. Final damage = base × armor-matrix ×
	-- modifier-stack × status-vulnerability (`docs/05 §5.3`). The BOSS takes plate
	-- mitigation instead of the armor matrix and no status/ability at all; every
	-- other enemy also takes the weapon's on-hit status. Conditional bonuses are
	-- evaluated live at impact against the target's status.
	local cond = condOf(s)
	local boss = bossVulnOf(s)
	local tankPos = s.tank.pos
	local impactDamage = 0
	for ii = 1, #impacts do
		local imp = impacts[ii]
		-- Weapon multiplier was baked into projectile damage at fire time.
		local modMult = ONE
		-- Each impact's signature ability accumulates over the enemies it hits,
		-- then flushes (tank heal / mana / hazard) once.
		local accum = newAccum()
		if flt(ZERO, imp.splash_radius) then
			local radiusSq = fmul(imp.splash_radius, imp.splash_radius)
			for ei = 1, #s.enemies do
				local e = s.enemies[ei]
				if fle(vecDistSq(imp.point, e.pos), radiusSq) then
					impactDamage += applyWeaponHit(
						e,
						imp.damage,
						imp.damage_type,
						modMult,
						cond,
						boss,
						imp.on_hit,
						imp.ability,
						tankPos,
						accum
					)
				end
			end
		else
			for ei = 1, #s.enemies do
				local e = s.enemies[ei]
				if e.id == imp.target then
					impactDamage += applyWeaponHit(
						e,
						imp.damage,
						imp.damage_type,
						modMult,
						cond,
						boss,
						imp.on_hit,
						imp.ability,
						tankPos,
						accum
					)
					break
				end
			end
		end
		flushAccum(s, accum, imp.ability, imp.damage_type)
	end
	recordPlayerDamage(s, impactDamage)

	-- Remove dead enemies, preserving id order; Fire-stacked deaths explode.
	reapDead(s)
end

-- ===========================================================================
-- Phase 6 — enemy movement + contact
-- ===========================================================================

--- Phase 6: move each enemy toward the tank (origin) by `EnemyDef::move_speed`
--- (`Vec2::step_toward`). On contact (reaches origin) deal `contact_damage` to
--- `s.tank.hp` and remove the enemy (no bounty for self-destruct). Keeps
--- `s.enemies` in id order.
function Combat.moveEnemies(s)
	local tankPos = s.tank.pos
	local survivors = {}

	-- Contact damage scales with match time on the SAME curve as enemy HP
	-- (`Content.enemyHpMult`): late game gets deadlier, not just tankier
	-- (`docs/05`; mirrors the spawn-time HP scaling in `waves::spawn`).
	local dmgMult = Content.enemyHpMult(s.tick)

	local enemies = s.enemies
	s.enemies = {}
	for xi = 1, #enemies do
		local e = enemies[xi]
		-- Stunned / frozen enemies can't move this tick.
		if isImmobile(e) then
			table.insert(survivors, e)
			continue
		end
		local edef = enemyDef(e.def)
		-- Movement is slowed by Frost stacks.
		local speed = fmul(fromInt(edef.move_speed), moveSpeedMult(e))
		local contact = scaleI64(dmgMult, edef.contact_damage)
		local moved = vecStepToward(e.pos, tankPos, speed)
		if vecEq(moved, tankPos) then
			if edef.boss then
				-- BOSS — PERSISTENT ATTRITION FIGHT (not a one-shot
				-- self-destruct). The boss does NOT despawn on contact: it
				-- plants at the tank and grinds it with a CADENCED contact hit
				-- (every `BOSS_CONTACT_CADENCE` ticks) until the player kills it
				-- or the tank dies. Determinism: the cadence and the enrage are
				-- pure functions of `s.tick` (no wall-clock, no new RNG);
				-- dodge/armor/shield are still honored per hit inside `hitTank`.
				e.pos = v2(moved) -- pin at the tank
				if s.tick % BOSS_CONTACT_CADENCE == 0 then
					-- ENRAGE: the hit grows every `BOSS_ENRAGE_INTERVAL` the boss
					-- stays alive, so the encounter is a RACE against a rising
					-- damage curve rather than a fixed DPS threshold you either
					-- clear or don't. Pure integer function of the tick. Note the
					-- TWO successive `scale_i64`s (`enemyHpMult` into `contact`
					-- above, then the enrage) — each truncates, so folding them
					-- into one multiply would not round the same way.
					hitTank(s, scaleI64(bossEnrageMult(s.tick), contact))
				end
				table.insert(survivors, e)
			else
				-- Normal enemy: self-destruct on contact (no bounty), one hit,
				-- gone.
				hitTank(s, contact)
			end
		else
			e.pos = v2(moved)
			table.insert(survivors, e)
		end
	end

	s.enemies = survivors
end

-- ===========================================================================
-- Phase — hazards
-- ===========================================================================

--- Phase: tick persistent hazards (land mines / burning oil). Each hazard
--- pulses `dmg` (× armor matrix × status vulnerability) to every non-boss enemy
--- within its radius, decrements its lifetime, and expires at zero. Hazards are
--- processed in id order and enemies in id order, so the pass is deterministic.
--- Hazard damage is a player source (scoreboard / Bloodmoney) and kills route
--- through the shared death path (`reapDead`), so bounty and Fire explosions
--- still fire.
function Combat.tickHazards(s)
	if #s.hazards == 0 then
		return
	end
	local cond = condOf(s)
	local boss = bossVulnOf(s)
	local tankPos = s.tank.pos
	local total = 0
	local any = false
	local hazards = s.hazards
	s.hazards = {}
	for hi = 1, #hazards do
		local h = hazards[hi]
		if h.ticks_left == 0 then
			continue
		end
		local radius = fromInt(h.radius)
		local r2 = fmul(radius, radius)
		-- A hazard carries no on-hit status and no chained ability (so `accum`
		-- is never flushed — matching the Rust, which does not call flush here).
		local accum = newAccum()
		for ei = 1, #s.enemies do
			local e = s.enemies[ei]
			if fle(vecDistSq(h.pos, e.pos), r2) then
				total += applyWeaponHit(
					e,
					h.dmg,
					h.damage_type,
					ONE,
					cond,
					boss,
					STATUS_ON_HIT_NONE,
					NO_ABILITY,
					tankPos,
					accum
				)
				any = true
			end
		end
		h.ticks_left -= 1
	end
	-- Drop expired hazards, preserving id order.
	local kept = {}
	for hi = 1, #hazards do
		local h = hazards[hi]
		if h.ticks_left > 0 then
			table.insert(kept, h)
		end
	end
	s.hazards = kept
	recordPlayerDamage(s, total)
	if any then
		reapDead(s)
	end
end

-- ===========================================================================
-- Phase — tank aura
-- ===========================================================================

--- Phase: periodic damage/poison AURA centered on the tank (source: Blight
--- Aura). A per-tank integer counter (`tank.aura_tick`) advances every tick;
--- each time it reaches the cadence it RESETS to 0 and the aura fires: deal
--- `aura_damage` (× armor matrix × status vulnerability) AND apply the tank's
--- `aura_poison_*` Poison DoT to every non-boss enemy within `aura_range`, in
--- STABLE id order. The cadence is a pure integer count of ticks
--- (`aura_cadence`), NOT wall-clock. Aura damage is a player source and kills
--- route through the shared death path. Integer/Fixed only; no RNG. A no-op
--- when the aura is unconfigured (`aura_cadence == 0`).
function Combat.tickAura(s)
	if s.tank.aura_cadence == 0 then
		return
	end
	-- Advance the integer cadence counter; fire (and reset) on the boundary.
	s.tank.aura_tick += 1
	if s.tank.aura_tick < s.tank.aura_cadence then
		return
	end
	s.tank.aura_tick = 0

	local dmg = s.tank.aura_damage
	local range = fromInt(s.tank.aura_range)
	local r2 = fmul(range, range)
	local cond = condOf(s)
	local boss = bossVulnOf(s)
	local tankPos = s.tank.pos
	local poison
	if s.tank.aura_poison_dps > 0 and s.tank.aura_poison_ticks > 0 then
		poison = {
			poison_dps = s.tank.aura_poison_dps,
			poison_ticks = s.tank.aura_poison_ticks,
			frost_stacks = 0,
			fire_stacks = 0,
			stun_ticks = 0,
		}
	else
		poison = STATUS_ON_HIT_NONE
	end
	local total = 0
	local any = false
	local accum = newAccum()
	-- `s.enemies` is id-ordered ⇒ this AoE pass is run-to-run stable.
	for ei = 1, #s.enemies do
		local e = s.enemies[ei]
		if fle(vecDistSq(tankPos, e.pos), r2) then
			total += applyWeaponHit(e, dmg, DMG_MAGIC, ONE, cond, boss, poison, NO_ABILITY, tankPos, accum)
			any = true
		end
	end
	recordPlayerDamage(s, total)
	if any then
		reapDead(s)
	end
end

-- ===========================================================================
-- Phase 6d — minions
-- ===========================================================================

--- Phase 6d: summoned allies act. Each minion (stable id order) targets the
--- nearest non-boss enemy: in reach it strikes on its attack cooldown, else it
--- steps toward the target. Strike damage scales with match time on the same
--- curve as enemy HP/damage and routes through the shared death path, so a
--- minion's kills award bounty and trigger Fire explosions. Expired minions are
--- removed. Deterministic: integer/Fixed math, stable id order, no RNG.
function Combat.tickMinions(s)
	if #s.minions == 0 then
		return
	end
	local now = s.tick
	local live = {}
	for mi = 1, #s.minions do
		local m = s.minions[mi]
		if now < m.expire_tick then
			table.insert(live, m)
		end
	end
	s.minions = live
	if #s.minions == 0 then
		return
	end

	local dmgMult = Content.enemyHpMult(now)
	local reach = fromInt(MINION_REACH)
	local reach2 = fmul(reach, reach)
	local speed = fromInt(MINION_SPEED)

	local minions = s.minions
	s.minions = {}
	local strikes = {} -- {target, scaled dmg, dtype}
	for mi = 1, #minions do
		local m = minions[mi]
		-- Nearest non-boss enemy; ties resolve to the lower id (id-ordered list).
		local bestId, bestPos, bestD2 = nil, nil, nil
		for ei = 1, #s.enemies do
			local e = s.enemies[ei]
			if not enemyDef(e.def).boss then
				local d2 = vecDistSq(m.pos, e.pos)
				if bestD2 == nil or flt(d2, bestD2) then
					bestId, bestPos, bestD2 = e.id, e.pos, d2
				end
			end
		end
		if bestD2 ~= nil then
			if fle(bestD2, reach2) then
				if now >= m.next_attack_tick then
					m.next_attack_tick = now + MINION_ATTACK_CD
					table.insert(strikes, { bestId, scaleI64(dmgMult, m.damage), m.damage_type })
				end
			else
				m.pos = v2(vecStepToward(m.pos, bestPos, speed))
			end
		end
	end
	s.minions = minions

	if #strikes == 0 then
		return
	end
	-- Resolve strikes (deterministic: minion id order). A minion strike carries
	-- no on-hit status and no chained ability.
	local cond = condOf(s)
	local boss = bossVulnOf(s)
	local tankPos = s.tank.pos
	local accum = newAccum()
	local total = 0
	for ki = 1, #strikes do
		local st = strikes[ki]
		local tid, dmg, dtype = st[1], st[2], st[3]
		for ei = 1, #s.enemies do
			local e = s.enemies[ei]
			if e.id == tid then
				total += applyWeaponHit(e, dmg, dtype, ONE, cond, boss, STATUS_ON_HIT_NONE, NO_ABILITY, tankPos, accum)
				break
			end
		end
	end
	recordPlayerDamage(s, total)
	reapDead(s)
end

-- ===========================================================================
-- Phase 6c — enemy ranged attacks
-- ===========================================================================

--- Phase 6c: ENEMY-side ranged attacks. Each enemy whose def carries
--- `EnemyAbility::RangedAttack` and is within its `range` of the tank fires on a
--- deterministic per-enemy phase — `(tick + enemy id) % cooldown == 0` —
--- dealing `damage` to the tank through the defensive layer (`Defense.hitTank`,
--- which honors dodge / mana-shield / armor). The phase is derived from existing
--- deterministic quantities (tick + stable id), so it needs NO per-enemy runtime
--- state and never touches the snapshot/checksum. Enemies are visited in stable
--- id order. Damage scales with match time on the same curve as contact damage
--- / HP (`Content.enemyHpMult`).
function Combat.enemyRangedAttacks(s)
	local tankPos = s.tank.pos
	local dmgMult = Content.enemyHpMult(s.tick)

	-- Resolve which enemies fire (and for how much) BEFORE calling into
	-- `hitTank` (which mutates the tank / draws `rng_proc`). Stable id order.
	local hits = {}
	for ei = 1, #s.enemies do
		local e = s.enemies[ei]
		-- Stunned / frozen enemies can't attack this tick.
		if isImmobile(e) then
			continue
		end
		local edef = enemyDef(e.def)
		local ab = edef.ability
		if ab.tag ~= ENEMY_ABILITY_RANGED then
			continue
		end
		-- `b = cooldown_ticks | (damage_type << 32)`; `damage_type` is reserved
		-- for future tank armor-class matrixing and render telemetry.
		local cooldownTicks = ab.b % U32
		if cooldownTicks == 0 then
			continue
		end
		local reach = fromInt(ab.a)
		if flt(fmul(reach, reach), vecDistSq(tankPos, e.pos)) then
			continue -- not yet within standoff range
		end
		-- Deterministic per-enemy firing phase (`u32` wrapping add).
		if ((s.tick + e.id) % U32) % cooldownTicks ~= 0 then
			continue
		end
		-- Time-scaled raw damage; the tank's flat armor / dodge / mana-shield
		-- are applied inside `Defense.hitTank`.
		table.insert(hits, scaleI64(dmgMult, ab.c))
	end

	for hi = 1, #hits do
		local raw = hits[hi]
		hitTank(s, raw)
	end
end

return Combat
