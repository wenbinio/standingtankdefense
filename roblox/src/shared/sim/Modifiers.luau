--!nonstrict
--[[
	Modifiers.luau — transcription of `sim-core/crates/sim/src/modifiers.rs`
	(`roblox/SIM-SPEC.md` §S1: this is a transcription, not a redesign).

	The modifier / stacking engine (`docs/05 §5.3`). Folds purchased modifiers
	into a small aggregate and answers the live queries damage resolution and
	weapon firing ask each tick. Deterministic: pure fixed-point, no RNG, no
	allocation beyond the aggregate itself. Struct fields mirror `state.rs` (so
	snapshots/checksums can serialize them); the behavior lives here.

	Stacking rule (from the source map): **additive within a kind, multiplicative
	across distinct multiplicative sources**. So all `+%` of the same additive
	flavor sum, then the result is multiplied by each independent `×` source.

	TRANSCRIPTION NOTES (Luau-specific, no behavior of their own)
	------------------------------------------------------------
	* `ModEffect` arrives from `content.json` in the `words()` encoding
	  (`CONTRACTS.md` C1): `{ tag, a, b, c }`. The dispatch table below is keyed
	  by that tag and each entry is named after the Rust variant it ports.
	* Rust `Fixed` values stay `Fixed.luau` values and are NEVER indexed (§S4).
	  Rust `i64`/`u32` stay plain Luau numbers.
	* Content ids are 0-based; Luau arrays are 1-indexed (§S3). Every such
	  boundary is spelled out with an explicit `+ 1`.
	* No Roblox global is referenced.
]]

local Fixed = require("../Fixed")
local Content = require("../Content")

local Modifiers = {}

--[[
	Overflow guard for the compounding percentage riders (`MaxHpPct` / `HpRegenPct`
	/ `ManaRegenPct`). Each such rider multiplies its stat by ~1.25 per purchase,
	so a pathological repeat-buy stack would otherwise run a stat to i64 overflow
	and poison every downstream `hp + x`. 1e12 sits orders of magnitude above any
	reachable real build (max HP tops out in the low millions even on a full
	snowball) yet leaves ~7 orders of headroom under i64::MAX for downstream adds,
	so the clamp is bit-stable across platforms and never trips in normal play.
]]
local STAT_CEIL = 1000000000000

-- u32::MAX, for the `saturating_mul` in the Dodge cap.
local U32_MAX = 4294967295

local ZERO = Fixed.ZERO
local ONE = Fixed.ONE
local add = Fixed.add
local sub = Fixed.sub
local mul = Fixed.mul
local fromRatio = Fixed.fromRatio
local fromInt = Fixed.fromInt

-- `Fixed::scale_i64` returning a plain Luau number. `scaleI64` yields an opaque
-- i64 value (so the saturation boundary stays exact); every consumer here feeds
-- it straight into integer arithmetic, exactly as the Rust does.
local function scaleToInt(f, v: number): number
	return Fixed.toInt(Fixed.scaleI64(f, v))
end

local function isZero(f): boolean
	return Fixed.eq(f, ZERO)
end

-- ============================ construction ============================

--- `Modifiers::new`.
function Modifiers.new()
	local addByType = table.create(5, ZERO)
	local addByScope = table.create(Content.NUM_SCOPES, ZERO)
	return {
		add_global = ZERO,
		add_by_type = addByType,
		add_by_scope = addByScope,
		mul_global = ONE,
		attack_speed = ZERO,
		vs_stunned = ZERO,
		vs_poisoned = ZERO,
		poison_dmg_mult = ONE,
		stun_dur_mult = ONE,
		weapon_count_scaling = {},
		dmg_per_maxhp_rate = ZERO,
		dmg_per_bounty_rate = ZERO,
		shield_active_dmg = ZERO,
	}
end

-- ============================ ramps ============================

--- Phase: apply each active time-scaling ramp whose interval has elapsed
--- (`docs/06`). Deterministic — fixed ticks, fixed order (ramps are append-only,
--- never reordered).
function Modifiers.applyRamps(s)
	local ramps = s.ramps
	if #ramps == 0 then
		return
	end
	for i = 1, #ramps do
		local r = ramps[i]
		while s.tick >= r.next_apply do
			Modifiers.applyEffect(s.modifiers, r.effect, s.economy, s.tank)
			r.next_apply += r.interval_ticks
		end
	end
end

-- ============================ live queries ============================

--[[
	Live GLOBAL additive damage bonus from the three DYNAMIC scalers, evaluated
	against the current `tank`/`economy` (NOT baked at purchase) — the offensive
	analogue of `selfScalingAdd`. Folded into the per-weapon multiplier at fire
	time exactly like `selfScalingAdd` (additive, then `×mul_global`):
	  * Mastercrafted Masonry: `rate × (max_hp / 2000)`.
	  * Golden Ring: `rate × ((bounty_mult − 1) / 0.5)`  ( = rate × 2 × bonus ).
	  * Arcane Mark: `shield_active_dmg` while `mana_shield > 0`, else nothing.
	Integer/Fixed only (feeds the checksum). A `bounty_mult` below the 1.0 base
	(never happens in normal play) is clamped so the term can't go negative.
]]
function Modifiers.dynamicGlobalAdd(m, tank, economy)
	local acc = ZERO
	if not isZero(m.dmg_per_maxhp_rate) and tank.max_hp > 0 then
		-- units = max_hp / 2000 (Fixed); bonus = rate × units.
		local units = fromRatio(tank.max_hp, 2000)
		acc = add(acc, mul(m.dmg_per_maxhp_rate, units))
	end
	if not isZero(m.dmg_per_bounty_rate) then
		-- bounty bonus above the 1.0 base, in units of 50% (×2 of the fraction).
		local bonus = sub(economy.bounty_mult, ONE)
		if Fixed.lt(ZERO, bonus) then
			local units = mul(bonus, fromInt(2))
			acc = add(acc, mul(m.dmg_per_bounty_rate, units))
		end
	end
	if not isZero(m.shield_active_dmg) and tank.mana_shield > 0 then
		acc = add(acc, m.shield_active_dmg)
	end
	return acc
end

--- Full damage multiplier for a specific weapon: global + its damage type +
--- each scope it matches (attack class, range bucket, rarity), times the
--- multiplicative product. Consulted at fire time (`docs/05 §5.3`).
---
--- The four additions run in the Rust's order: with saturating `Fixed` adds and
--- a multiplicative tail, reordering is observable.
function Modifiers.weaponDamageMult(m, w)
	local acc = add(m.add_global, m.add_by_type[w.damage_type % 5 + 1])
	acc = add(acc, m.add_by_scope[Content.attackScopeId(w.attack) + 1])
	acc = add(acc, m.add_by_scope[Content.rangeScopeId(w.range) + 1])
	acc = add(acc, m.add_by_scope[Content.rarityScopeId(w.rarity) + 1])
	return mul(add(ONE, acc), m.mul_global)
end

--- Additive self-scaling % for a weapon of `dmgType`, given the owned
--- weapons: `Σ rule.per × (count of rule.weapon_def)` over rules matching the
--- type. Resolved live at fire time (it depends on the current arsenal).
function Modifiers.selfScalingAdd(m, dmgType: number, weapons)
	local acc = ZERO
	local rules = m.weapon_count_scaling
	for i = 1, #rules do
		local rule = rules[i]
		if rule.dmg_type == dmgType then
			local count = 0
			for j = 1, #weapons do
				if weapons[j].def == rule.weapon_def then
					count += 1
				end
			end
			if count > 0 then
				acc = add(acc, mul(rule.per, fromInt(count)))
			end
		end
	end
	return acc
end

--- Scale a weapon's on-hit status by the player's flavor modifiers
--- (Poison-damage % and Stun-duration %). Depends only on the modifiers, so
--- it bakes into the hit at fire time (like base damage). Frost/Fire stacks
--- are unaffected here.
---
--- The Rust takes `StatusOnHit` by value (it is `Copy`); the Luau input is a
--- shared table owned by `Content`, so a COPY is returned and the content table
--- is never mutated.
function Modifiers.scaleOnHit(m, onHit)
	local out = {
		poison_dps = onHit.poison_dps,
		poison_ticks = onHit.poison_ticks,
		frost_stacks = onHit.frost_stacks,
		fire_stacks = onHit.fire_stacks,
		stun_ticks = onHit.stun_ticks,
	}
	if out.poison_dps > 0 and not Fixed.eq(m.poison_dmg_mult, ONE) then
		out.poison_dps = scaleToInt(m.poison_dmg_mult, out.poison_dps)
	end
	if out.stun_ticks > 0 and not Fixed.eq(m.stun_dur_mult, ONE) then
		out.stun_ticks = scaleToInt(m.stun_dur_mult, out.stun_ticks)
	end
	return out
end

--- Total damage multiplier for a hit of `damageType`: `(1 + Σ matching
--- additive%) × Π multiplicative factors`.
function Modifiers.damageMult(m, damageType: number)
	local additive = add(add(ONE, m.add_global), m.add_by_type[damageType % 5 + 1])
	return mul(additive, m.mul_global)
end

--- Attack-speed multiplier (`1 + Σ attack-speed%`). Effective cooldown is the
--- base cooldown divided by this.
function Modifiers.attackSpeedMult(m)
	return add(ONE, m.attack_speed)
end

-- ============================ effect application ============================

-- One handler per `ModEffect` variant, keyed by its `words()` tag
-- (`content.rs::ModEffect::words`). The operand names match the Rust variant's
-- field names so each body reads as the Rust match arm it ports.
local EFFECTS: { [number]: (any, number, number, number, any, any) -> () } = {}

-- 0: DamageGlobalPct(n, d) — +% additive to ALL weapon damage.
EFFECTS[0] = function(m, n, d, _c, _economy, _tank)
	m.add_global = add(m.add_global, fromRatio(n, d))
end

-- 1: DamageTypePct(t, n, d) — +% additive to one damage type's damage.
EFFECTS[1] = function(m, t, n, d, _economy, _tank)
	local i = t % 5 + 1
	m.add_by_type[i] = add(m.add_by_type[i], fromRatio(n, d))
end

-- 2: DamageMulPct(n, d) — a multiplicative damage factor: ×(1 + n/d).
-- THIS is the "multiplicative across sources" half of the stacking rule: each
-- purchase compounds into `mul_global` rather than summing with the additives.
EFFECTS[2] = function(m, n, d, _c, _economy, _tank)
	m.mul_global = mul(m.mul_global, add(ONE, fromRatio(n, d)))
end

-- 3: AttackSpeedPct(n, d)
EFFECTS[3] = function(m, n, d, _c, _economy, _tank)
	m.attack_speed = add(m.attack_speed, fromRatio(n, d))
end

-- 4: BountyPct(n, d)
EFFECTS[4] = function(_m, n, d, _c, economy, _tank)
	economy.bounty_mult = add(economy.bounty_mult, fromRatio(n, d))
end

-- 5: IncomeFlat(f)
EFFECTS[5] = function(_m, f, _b, _c, economy, _tank)
	economy.income_per_tick += f
end

-- 6: MaxHp(f) — +flat max HP (and current HP).
EFFECTS[6] = function(_m, f, _b, _c, _economy, tank)
	tank.max_hp += f
	tank.hp += f
end

-- 7: Armor(a)
EFFECTS[7] = function(_m, a, _b, _c, _economy, tank)
	tank.armor += a
end

-- 8: ManaShield(pool, regen)
EFFECTS[8] = function(_m, pool, regen, _c, _economy, tank)
	tank.mana_shield_max += pool
	tank.mana_shield += pool
	tank.mana_regen_per_tick += regen
end

-- 9: HpRegen(r)
EFFECTS[9] = function(_m, r, _b, _c, _economy, tank)
	tank.hp_regen_per_tick += r
end

--[[
	10: Dodge(n)
	Additive, but HARD-CAPPED at 70% of `dodge_den` (integer math:
	`dodge_den * 7 / 10`). This is a deliberate nerf: at the old
	99% ceiling dodge was an invincibility button — once a build hit
	it, NO enemy lever (HP / contact / volume / boss) could pressure
	it and ~88% of runs coasted to the 60-min cap purely on dodge.
	Capping the effective avoid-rate at 70% leaves dodge strong but
	mortal: ~30% of hits still land, so the HP staircase and the boss
	can finally threaten a snowball, and mitigation (armor / shield /
	regen / spikes) becomes a real competing axis instead of a
	strictly-worse alternative. Integer/Fixed only — feeds the
	checksum, so no floats.
]]
EFFECTS[10] = function(_m, n, _b, _c, _economy, tank)
	local cap = math.min(tank.dodge_den * 7, U32_MAX) // 10
	tank.dodge_num = math.min(tank.dodge_num + n, cap)
end

-- 11: DamageScopePct(sid, n, d)
EFFECTS[11] = function(m, sid, n, d, _economy, _tank)
	local i = sid % Content.NUM_SCOPES + 1
	m.add_by_scope[i] = add(m.add_by_scope[i], fromRatio(n, d))
end

-- 12: SpikesFlat(n)
EFFECTS[12] = function(_m, n, _b, _c, _economy, tank)
	tank.spikes_damage += n
end

-- 13: SpikesPct(n, d)
EFFECTS[13] = function(_m, n, d, _c, _economy, tank)
	tank.spikes_mult = add(tank.spikes_mult, fromRatio(n, d))
end

-- 14: GrantVulnPulse(..) — registered as per-arena trigger state in
-- `buy_modifier`; no aggregate contribution here.
EFFECTS[14] = function() end

-- 15: HealOnKill(n)
EFFECTS[15] = function(_m, n, _b, _c, _economy, tank)
	tank.heal_on_kill += n
end

-- 16: HealOnPoison(n)
EFFECTS[16] = function(_m, n, _b, _c, _economy, tank)
	tank.heal_on_poison += n
end

-- 17: IncomePct(n, d)
EFFECTS[17] = function(_m, n, d, _c, economy, _tank)
	economy.income_mult = add(economy.income_mult, fromRatio(n, d))
end

-- 18: IncomeRegenPct(n, d)
EFFECTS[18] = function(_m, n, d, _c, economy, _tank)
	economy.income_regen_pct = add(economy.income_regen_pct, fromRatio(n, d))
end

-- 19: BountyProc(c, b) — chance is a plain percent; the bonus is a FRACTION of
-- the base bounty, so `b` is a percentage over 100.
EFFECTS[19] = function(_m, c, b, _c3, economy, _tank)
	economy.bounty_proc_chance_pct += c
	economy.bounty_proc_bonus = add(economy.bounty_proc_bonus, fromRatio(b, 100))
end

-- 20: DamageVsStunnedPct(n, d)
EFFECTS[20] = function(m, n, d, _c, _economy, _tank)
	m.vs_stunned = add(m.vs_stunned, fromRatio(n, d))
end

-- 21: DamageVsPoisonedPct(n, d)
EFFECTS[21] = function(m, n, d, _c, _economy, _tank)
	m.vs_poisoned = add(m.vs_poisoned, fromRatio(n, d))
end

-- 22: PoisonDamagePct(n, d) — note this ADDS into a multiplier that starts at
-- ONE (so a single +50% yields ×1.5), matching the Rust.
EFFECTS[22] = function(m, n, d, _c, _economy, _tank)
	m.poison_dmg_mult = add(m.poison_dmg_mult, fromRatio(n, d))
end

-- 23: StunDurationPct(n, d) — same shape as PoisonDamagePct.
EFFECTS[23] = function(m, n, d, _c, _economy, _tank)
	m.stun_dur_mult = add(m.stun_dur_mult, fromRatio(n, d))
end

-- 24/25/26: GrantDuplicator / GrantVoucher / GrantGold — META purchase-flow
-- effects, intercepted in `buy_modifier`; no aggregate contribution here.
EFFECTS[24] = function() end
EFFECTS[25] = function() end
EFFECTS[26] = function() end

-- 27: HealingPct(n, d)
EFFECTS[27] = function(_m, n, d, _c, _economy, tank)
	tank.healing_mult = add(tank.healing_mult, fromRatio(n, d))
end

-- 28: MissingHpHealPct(n, d)
EFFECTS[28] = function(_m, n, d, _c, _economy, tank)
	tank.missing_hp_heal_pct = add(tank.missing_hp_heal_pct, fromRatio(n, d))
end

-- 29: GrantRevive(bonus)
EFFECTS[29] = function(_m, bonus, _b, _c, _economy, tank)
	tank.revives += 1
	tank.revive_bonus_hp = math.max(tank.revive_bonus_hp, bonus)
end

-- 30: DamagePerWeapon(def, ty, num) — append a self-scaling rule; `per` is a
-- percentage over 100.
EFFECTS[30] = function(m, def, ty, num, _economy, _tank)
	table.insert(m.weapon_count_scaling, {
		weapon_def = def,
		dmg_type = ty,
		per = fromRatio(num, 100),
	})
end

-- 31/32: TradeMaxHpForGold / TradeRegenForGold — the HP/regen→gold trades need
-- the full ArenaState (gold scoreboard) and are intercepted in `buy_modifier`.
EFFECTS[31] = function() end
EFFECTS[32] = function() end

-- 33: GoldPerDamagePct(n, d)
EFFECTS[33] = function(_m, n, d, _c, economy, _tank)
	economy.gold_per_damage = add(economy.gold_per_damage, fromRatio(n, d))
end

-- 34: IncomeShieldPct(n, d)
EFFECTS[34] = function(_m, n, d, _c, economy, _tank)
	economy.income_shield_pct = add(economy.income_shield_pct, fromRatio(n, d))
end

--[[
	35/36/37 — Percentage riders evaluated against the CURRENT stat at apply-time
	(so in a bundle they compound on the flat effect listed before them).
	Integer/Fixed only — these feed the checksum. The compounding is
	CLAMPED to `STAT_CEIL`: repeatedly buying a `[flat, pct]` bundle
	multiplies the stat by ~1.25 each time, which would otherwise overflow
	i64 (and poison every downstream `hp + x`) in a pathological purchase
	stack. `STAT_CEIL` (1e12) sits far above any reachable real build yet
	leaves ample headroom for downstream adds, so the clamp is bit-stable
	across platforms and never trips in normal play. Same spirit as the
	Dodge hard-cap. The inc itself is also derived from the clamped stat,
	so it stays bounded.

	RETROACTIVITY: the increment is taken from the stat as it stands when this
	effect runs — NOT from a base snapshot — which is exactly why effect order
	inside a bundle is load-bearing and why `Modifiers.apply` walks
	`def.effects` in slice order.
]]
-- 35: MaxHpPct(n, d)
EFFECTS[35] = function(_m, n, d, _c, _economy, tank)
	local inc = scaleToInt(fromRatio(n, d), tank.max_hp)
	tank.max_hp = math.min(tank.max_hp + inc, STAT_CEIL)
	tank.hp += inc
end

-- 36: HpRegenPct(n, d)
EFFECTS[36] = function(_m, n, d, _c, _economy, tank)
	local inc = scaleToInt(fromRatio(n, d), tank.hp_regen_per_tick)
	tank.hp_regen_per_tick = math.max(-STAT_CEIL, math.min(tank.hp_regen_per_tick + inc, STAT_CEIL))
end

-- 37: ManaRegenPct(n, d)
EFFECTS[37] = function(_m, n, d, _c, _economy, tank)
	local inc = scaleToInt(fromRatio(n, d), tank.mana_regen_per_tick)
	tank.mana_regen_per_tick = math.max(-STAT_CEIL, math.min(tank.mana_regen_per_tick + inc, STAT_CEIL))
end

-- 38: ShieldActiveDrPct(n, d)
EFFECTS[38] = function(_m, n, d, _c, _economy, tank)
	tank.shield_active_dr = add(tank.shield_active_dr, fromRatio(n, d))
end

-- 39: HealOnDamaged(n)
EFFECTS[39] = function(_m, n, _b, _c, _economy, tank)
	tank.heal_on_damaged += n
end

-- 40/41/42 — DYNAMIC global-damage scalers (resolved live at fire time via
-- `dynamicGlobalAdd`, mirroring `DamagePerWeapon`/`selfScalingAdd`).
-- Accumulate only the per-unit RATE / flat bonus here.
EFFECTS[40] = function(m, n, d, _c, _economy, _tank)
	m.dmg_per_maxhp_rate = add(m.dmg_per_maxhp_rate, fromRatio(n, d))
end
EFFECTS[41] = function(m, n, d, _c, _economy, _tank)
	m.dmg_per_bounty_rate = add(m.dmg_per_bounty_rate, fromRatio(n, d))
end
EFFECTS[42] = function(m, n, d, _c, _economy, _tank)
	m.shield_active_dmg = add(m.shield_active_dmg, fromRatio(n, d))
end

-- 43: ManaOnKill(n)
EFFECTS[43] = function(_m, n, _b, _c, _economy, tank)
	tank.mana_on_kill += n
end

-- EXPANSION E2 — plain tank-field setters (no aggregate; integer only).
-- 44: ShieldBreakStun(r, t)
-- Shield-break stun (Energy Pulse): arm the pulse range/duration. Keep the
-- STRONGER of any stacked copies so two Energy Pulses don't shrink the window.
EFFECTS[44] = function(_m, r, t, _c, _economy, tank)
	tank.shieldbreak_stun_range = math.max(tank.shieldbreak_stun_range, r)
	tank.shieldbreak_stun_ticks = math.max(tank.shieldbreak_stun_ticks, t)
end

-- 45: SpikesPoison(dps, t)
-- Spikes poison (Poison Armor): keep whichever DoT deals more total
-- remaining damage (mirrors the poison-application rule in `status`).
EFFECTS[45] = function(_m, dps, t, _c, _economy, tank)
	local existing = tank.spikes_poison_dps * tank.spikes_poison_ticks
	local incoming = dps * t
	if incoming >= existing then
		tank.spikes_poison_dps = dps
		tank.spikes_poison_ticks = t
	end
end

-- 46: StackingSpikes(per, max)
-- Stacking spikes (Bloody Spikes): set the per-stack bonus and the cap.
-- Additive per-stack, max cap takes the larger of stacked copies.
EFFECTS[46] = function(_m, per, max, _c, _economy, tank)
	tank.spikes_stack_per += per
	tank.spikes_stacks_max = math.max(tank.spikes_stacks_max, max)
end

-- 47: DamageAura(r, c, d)
-- Damage/poison aura (Blight Aura): arm the emitter. Keep the wider range,
-- shorter (more frequent) cadence, and larger damage of any stacked
-- copies; the poison rider mirrors Poison Armor's DoT (2/tick × 90).
EFFECTS[47] = function(_m, r, c, d, _economy, tank)
	tank.aura_range = math.max(tank.aura_range, r)
	tank.aura_damage += d
	if tank.aura_cadence == 0 then
		tank.aura_cadence = c
	else
		tank.aura_cadence = math.min(tank.aura_cadence, c)
	end
	tank.aura_poison_dps = math.max(tank.aura_poison_dps, 2)
	tank.aura_poison_ticks = math.max(tank.aura_poison_ticks, 90)
end

--- Apply a single `ModEffect` (used both for a modifier's base effect and
--- for each tick of a time-scaling ramp). Damage/attack-speed effects
--- accumulate in this aggregate; economy/defensive effects apply immediately.
function Modifiers.applyEffect(m, effect, economy, tank)
	local handler = EFFECTS[effect.tag]
	if handler == nil then
		error("Modifiers.applyEffect: unknown ModEffect tag " .. tostring(effect.tag), 2)
	end
	handler(m, effect.a, effect.b, effect.c, economy, tank)
end

--- Fold a purchased modifier in (applies each of its base `effects`, in order).
--- Order is load-bearing: the percentage riders (35/36/37) read the CURRENT
--- stat, so a `[flat, pct]` bundle compounds.
function Modifiers.apply(m, def, economy, tank)
	local effects = def.effects
	for i = 1, #effects do
		Modifiers.applyEffect(m, effects[i], economy, tank)
	end
end

return Modifiers
