--[[
	Arena.luau — the simulation entry point: a transcription of
	`sim-core/crates/sim/src/lib.rs` (`step` + `checksum`).
	Spec: `roblox/SIM-SPEC.md` §S5 (tick order) and §S6 (checksum parity).

	This is the seam every other sim module plugs into. It owns two things and
	nothing else:

	  * `Arena.step(state, input)` — the 21 phases of §S5, in exactly that order.
	    The order IS the spec (`docs/05 §5.6.1`); several phases depend on flags
	    set and consumed by their neighbours (e.g. `Defense.spikes` consumes the
	    tank-hit flag before `Defense.shieldBreakStun` reads its own break flag).
	  * `Arena.checksum(state)` — `lib.rs::checksum`, field-for-field, in order,
	    with entities iterated in ID-SORTED order so the digest is independent of
	    vector ordering (`docs/04 §4.4.4`). Render-only and transient state
	    (`tank_hit_this_tick`, `shield_broke_this_tick`, `bought_attack_mask`,
	    `weapons_bought`, `economy_purchases`, `last_target_pos`) is excluded by
	    construction — exactly as the Rust excludes it.

	ONE TRAVERSAL, TWO ACCUMULATORS
	-------------------------------
	The checksum field ORDER is the single most fragile thing in the R3 gate, so
	it is written down exactly once, in `traverse()`. `Arena.checksum` runs it
	against a plain `Checksum`; `Arena.checksumFields` runs the SAME traversal
	against `Checksum.newVerbose()`, which additionally logs every write with its
	label and the running digest. The `--verbose` dump therefore cannot drift
	from what is actually hashed.

	ENGINE INDEPENDENCE
	-------------------
	No Roblox global is referenced. Runs unchanged under a bare `luau`.

	CONSUMED INTERFACES (`SIM-SPEC.md` §S2 — other agents' modules):

	    Checksum.new() / Checksum.newVerbose()
	    c:writeU32(v, label?) · c:writeI64(v, label?) · c:writeFixed(x, label?)
	    c:writeU64(hi, lo, label?) · c:finish() -> hi, lo · c:finishHex() -> string

	    State.new(master_seed, player_id) -> ArenaState
	    Input.apply(s, inp)
	    Shop.generateOffers(s)      Economy.onRoundStart(s)
	    Modifiers.applyRamps(s)     Waves.spawn(s)          Status.pulse(s)
	    Combat.fireWeapons(s)       Combat.advanceProjectiles(s)
	    Combat.moveEnemies(s)       Combat.enemyRangedAttacks(s)
	    Defense.spikes(s)           Defense.shieldBreakStun(s)
	    Combat.tickHazards(s)       Combat.tickAura(s)      Combat.tickMinions(s)
	    Status.tick(s)              Economy.collectBounties(s)
	    Economy.tickIncome(s)       Defense.regen(s)        Economy.resolveDeaths(s)
]]

local Rng = require("../Rng")
local Checksum = require("../Checksum")

local State = require("./State")
local Input = require("./Input")
local Combat = require("./Combat")
local Defense = require("./Defense")
local Economy = require("./Economy")
local Modifiers = require("./Modifiers")
local Shop = require("./Shop")
local Status = require("./Status")
local Waves = require("./Waves")

local Arena = {}

--- Simulation frequency. The only clock the sim knows (`lib.rs::TICK_HZ`).
Arena.TICK_HZ = 30
--- Ticks per round (a new shop opens each round). 30 s @ 30 Hz.
---
--- TRANSCRIBED FROM `lib.rs::ROUND_TICKS` (= 900), NOT the `timeline.roblox`
--- rescale in `content.json` (600). The Rust is the oracle for R3: a trace's
--- checksums are produced by a sim using 900, so using 600 here would diverge at
--- the first round boundary. The F1 rescale is a separate, later switch — it is
--- exposed here as one named constant so that switch is a one-line change.
Arena.ROUND_TICKS = 30 * 30

local ROUND_TICKS = Arena.ROUND_TICKS
local U32_MAX = 4294967295

-- ============================ step ============================

--- Advance the arena by exactly one tick, applying `inp` (this player's action).
---
--- The order of operations below IS part of the spec: client and the validating
--- shadow-sim must apply phases in this exact sequence so their `checksum()`
--- agree (`docs/05 §5.6.1`).
function Arena.step(s, inp)
	if s.dead then
		-- Frozen after death, but keep advancing the tick so checksum traces
		-- across runs stay length-aligned.
		s.tick += 1
		return
	end

	-- 1. Round boundary → refresh shop + per-round economy.
	-- `tick` is a u32 (never negative), so floor division matches Rust's `/`.
	local new_round = s.tick // ROUND_TICKS
	if new_round ~= s.round then
		s.round = new_round
		Shop.generateOffers(s)
		Economy.onRoundStart(s)
		-- Stacking spikes (source: Bloody Spikes) reset to 0 at the round boundary
		-- ("resets when a new shop is made available"). Deterministic: keyed purely
		-- off the round transition (integer tick). At round 0 the counter is already
		-- 0, so this is a no-op there; from round 1 on it clears the prior round.
		s.tank.spikes_stacks = 0
	end
	-- 2. Player input (buy / reroll / clear).
	Input.apply(s, inp)
	-- 2b. Time-scaling growth: apply any ramping modifiers whose interval elapsed.
	Modifiers.applyRamps(s)
	-- 3. Spawn enemies for the current wave.
	Waves.spawn(s)
	-- 3b. Vulnerability-Pulse auras stack a damage-taken debuff on enemies in range.
	Status.pulse(s)
	-- 4. Weapons select targets and emit projectiles.
	Combat.fireWeapons(s)
	-- 5. Projectiles move; on arrival apply (splash) damage; kills → pending_kills.
	Combat.advanceProjectiles(s)
	-- 6. Enemies advance toward the tank; contact damage on arrival.
	Combat.moveEnemies(s)
	-- 6c. Ranged enemies (breathers/spitters/casters) attack the tank at standoff.
	Combat.enemyRangedAttacks(s)
	-- 6b. Spikes: if the tank was hit, retaliate against nearby enemies.
	Defense.spikes(s)
	-- 6b'. Shield-break stun (Energy Pulse): if the Mana Shield broke this tick,
	-- stun enemies in range. Runs after `spikes` (which consumes the hit flag) but
	-- before hazards/auras; the break flag is set by `hit_tank` during combat.
	Defense.shieldBreakStun(s)
	-- 6c. Hazards (land mines / burning oil) pulse damage to enemies in range.
	Combat.tickHazards(s)
	-- 6c'. Damage/poison aura (Blight Aura): periodic AoE on its integer cadence.
	Combat.tickAura(s)
	-- 6d. Summoned allies (Larvae / Spores) move and strike enemies.
	Combat.tickMinions(s)
	-- 7. Status effects: poison DoT, frost/stun decay (poison kills → pending_kills).
	Status.tick(s)
	-- 8. Drain pending_kills → award bounty (scaled by bounty_mult).
	Economy.collectBounties(s)
	-- 9. Passive income.
	Economy.tickIncome(s)
	-- 10. Tank regeneration (mana shield + HP).
	Defense.regen(s)
	-- 11. Death check (tank hp ≤ 0).
	Economy.resolveDeaths(s)

	s.tick += 1
end

--- Convenience re-export of the state constructor (`lib.rs` does `pub use state::*`).
function Arena.newState(master_seed, player_id)
	return State.new(master_seed, player_id)
end

-- ============================ checksum plumbing ============================

--- Copy `list` and sort it by the id field `key`. The Rust collects references
--- and sorts the COPY, leaving the live vector's order untouched — do the same,
--- so building a checksum never mutates sim state.
local function sortedById(list, key)
	local n = #list
	local out = table.create(n)
	for i = 1, n do
		out[i] = list[i]
	end
	table.sort(out, function(a, b)
		return a[key] < b[key]
	end)
	return out
end

--- The `(tag, a, b, c)` payload encoding shared by `WeaponAbility::words` and
--- `ModEffect::words` (`CONTRACTS.md` §C1). `nil` is `WeaponAbility::None`,
--- whose words are `(0, 0, 0, 0)`.
local function words(w)
	if w == nil then
		return 0, 0, 0, 0
	end
	local tag = w.tag
	if tag == nil then
		tag = w[1]
	end
	local a = w.a
	if a == nil then
		a = w[2]
	end
	local b = w.b
	if b == nil then
		b = w[3]
	end
	local c = w.c
	if c == nil then
		c = w[4]
	end
	return tag or 0, a or 0, b or 0, c or 0
end

--- `OfferKind` → the Rust's checksum code (Weapon = 0, Modifier = 1).
local function offerKindCode(k)
	if type(k) == "number" then
		return k
	end
	if k == "Weapon" then
		return 0
	end
	if k == "Modifier" then
		return 1
	end
	if type(k) == "table" and type(k.code) == "number" then
		return k.code
	end
	error("Arena.checksum: unrecognized OfferKind " .. tostring(k), 2)
end

local function boolU32(b)
	return b and 1 or 0
end

-- ============================ the traversal ============================

--- Deterministic `state_checksum` over all authoritative-relevant fields, in the
--- exact order of `lib.rs::checksum`. DO NOT REORDER, ADD OR REMOVE A FIELD: the
--- whole R3 gate is "this digest equals the Rust's", so a field-set or ordering
--- change silently invalidates it.
---
--- Written ONCE and driven by both the plain and the verbose `Checksum`, so the
--- `--verbose` field dump can never drift from what is actually hashed. Labels
--- are only materialized when `c` is a verbose accumulator (`c.log ~= nil`); in
--- the hot path the per-entity prefix is never built.
local function traverse(s, c)
	local vb = c.log ~= nil

	c:writeU32(s.tick, "tick")
	c:writeU32(s.round, "round")
	local mhi, mlo = Rng.seedParts(s.master_seed)
	c:writeU64(mhi, mlo, "master_seed")
	c:writeU32(s.player_id, "player_id")

	local t = s.tank
	c:writeI64(t.hp, "tank.hp")
	c:writeI64(t.max_hp, "tank.max_hp")
	c:writeFixed(t.pos.x, "tank.pos.x")
	c:writeFixed(t.pos.y, "tank.pos.y")
	c:writeU32(t.clear_cooldown_end, "tank.clear_cooldown_end")
	c:writeI64(t.armor, "tank.armor")
	c:writeU32(t.dodge_num, "tank.dodge_num")
	c:writeU32(t.dodge_den, "tank.dodge_den")
	c:writeI64(t.mana_shield, "tank.mana_shield")
	c:writeI64(t.mana_shield_max, "tank.mana_shield_max")
	c:writeI64(t.mana_regen_per_tick, "tank.mana_regen_per_tick")
	c:writeI64(t.hp_regen_per_tick, "tank.hp_regen_per_tick")
	c:writeI64(t.spikes_damage, "tank.spikes_damage")
	c:writeFixed(t.spikes_mult, "tank.spikes_mult")
	c:writeI64(t.heal_on_kill, "tank.heal_on_kill")
	c:writeI64(t.heal_on_poison, "tank.heal_on_poison")
	c:writeFixed(t.healing_mult, "tank.healing_mult")
	c:writeFixed(t.missing_hp_heal_pct, "tank.missing_hp_heal_pct")
	c:writeU32(t.revives, "tank.revives")
	c:writeI64(t.revive_bonus_hp, "tank.revive_bonus_hp")
	-- EXPANSION E2 exotic-mechanic tank state. The dynamic counters (`spikes_stacks`,
	-- `aura_tick`) MUST be checksummed (they drive behavior and change per tick); the
	-- config fields are included alongside for a clean snapshot↔checksum mirror.
	c:writeI64(t.shieldbreak_stun_range, "tank.shieldbreak_stun_range")
	c:writeU32(t.shieldbreak_stun_ticks, "tank.shieldbreak_stun_ticks")
	c:writeI64(t.spikes_poison_dps, "tank.spikes_poison_dps")
	c:writeU32(t.spikes_poison_ticks, "tank.spikes_poison_ticks")
	c:writeI64(t.spikes_stack_per, "tank.spikes_stack_per")
	c:writeU32(t.spikes_stacks, "tank.spikes_stacks")
	c:writeU32(t.spikes_stacks_max, "tank.spikes_stacks_max")
	c:writeI64(t.aura_range, "tank.aura_range")
	c:writeU32(t.aura_cadence, "tank.aura_cadence")
	c:writeI64(t.aura_damage, "tank.aura_damage")
	c:writeI64(t.aura_poison_dps, "tank.aura_poison_dps")
	c:writeU32(t.aura_poison_ticks, "tank.aura_poison_ticks")
	c:writeU32(t.aura_tick, "tank.aura_tick")

	local ec = s.economy
	c:writeI64(ec.gold, "economy.gold")
	c:writeI64(ec.income_per_tick, "economy.income_per_tick")
	c:writeFixed(ec.income_mult, "economy.income_mult")
	c:writeFixed(ec.income_regen_pct, "economy.income_regen_pct")
	c:writeFixed(ec.bounty_mult, "economy.bounty_mult")
	c:writeI64(ec.bounty_proc_chance_pct, "economy.bounty_proc_chance_pct")
	c:writeFixed(ec.bounty_proc_bonus, "economy.bounty_proc_bonus")
	c:writeFixed(ec.gold_per_damage, "economy.gold_per_damage")
	c:writeFixed(ec.income_shield_pct, "economy.income_shield_pct")
	c:writeU32(ec.rerolls_remaining, "economy.rerolls_remaining")
	c:writeI64(ec.reroll_cost, "economy.reroll_cost")

	c:writeU32(s.next_entity_id, "next_entity_id")
	c:writeU32(boolU32(s.dead), "dead")
	-- `death_tick.unwrap_or(u32::MAX)`.
	c:writeU32(s.death_tick ~= nil and s.death_tick or U32_MAX, "death_tick")
	local pk = s.pending_kills
	c:writeU32(#pk, "pending_kills.len")
	for i = 1, #pk do
		c:writeU32(pk[i], vb and "pending_kills[" .. i .. "]")
	end
	c:writeI64(s.total_damage_dealt, "total_damage_dealt")
	c:writeI64(s.total_gold_earned, "total_gold_earned")

	local rhi, rlo = Rng.state(s.rng_spawn)
	c:writeU64(rhi, rlo, "rng_spawn")
	rhi, rlo = Rng.state(s.rng_targeting)
	c:writeU64(rhi, rlo, "rng_targeting")
	rhi, rlo = Rng.state(s.rng_shop)
	c:writeU64(rhi, rlo, "rng_shop")
	rhi, rlo = Rng.state(s.rng_reroll)
	c:writeU64(rhi, rlo, "rng_reroll")
	rhi, rlo = Rng.state(s.rng_proc)
	c:writeU64(rhi, rlo, "rng_proc")

	-- Weapons (instance ids are monotonic; checksum in id order).
	local wl = sortedById(s.weapons, "instance_id")
	c:writeU32(#wl, "weapons.len")
	for i = 1, #wl do
		local x = wl[i]
		local p = vb and "weapons[" .. i .. "]."
		c:writeU32(x.instance_id, p and p .. "instance_id")
		c:writeU32(x.def, p and p .. "def")
		c:writeU32(x.next_fire_tick, p and p .. "next_fire_tick")
	end

	-- Modifiers aggregate.
	local md = s.modifiers
	c:writeFixed(md.add_global, "modifiers.add_global")
	-- NOTE: fixed-size Rust arrays — written with NO length prefix.
	for i = 1, #md.add_by_type do
		c:writeFixed(md.add_by_type[i], vb and "modifiers.add_by_type[" .. i .. "]")
	end
	for i = 1, #md.add_by_scope do
		c:writeFixed(md.add_by_scope[i], vb and "modifiers.add_by_scope[" .. i .. "]")
	end
	c:writeFixed(md.mul_global, "modifiers.mul_global")
	c:writeFixed(md.attack_speed, "modifiers.attack_speed")
	c:writeFixed(md.vs_stunned, "modifiers.vs_stunned")
	c:writeFixed(md.vs_poisoned, "modifiers.vs_poisoned")
	c:writeFixed(md.poison_dmg_mult, "modifiers.poison_dmg_mult")
	c:writeFixed(md.stun_dur_mult, "modifiers.stun_dur_mult")
	local wcs = md.weapon_count_scaling
	c:writeU32(#wcs, "modifiers.weapon_count_scaling.len")
	for i = 1, #wcs do
		local r = wcs[i]
		local p = vb and "modifiers.weapon_count_scaling[" .. i .. "]."
		c:writeU32(r.weapon_def, p and p .. "weapon_def")
		c:writeU32(r.dmg_type, p and p .. "dmg_type")
		c:writeFixed(r.per, p and p .. "per")
	end

	-- Active time-scaling ramps (append-only order).
	local ramps = s.ramps
	c:writeU32(#ramps, "ramps.len")
	for i = 1, #ramps do
		local r = ramps[i]
		local p = vb and "ramps[" .. i .. "]."
		local tag, a, b, d = words(r.effect)
		c:writeU32(tag, p and p .. "effect.tag")
		c:writeI64(a, p and p .. "effect.a")
		c:writeI64(b, p and p .. "effect.b")
		c:writeI64(d, p and p .. "effect.c")
		c:writeU32(r.interval_ticks, p and p .. "interval_ticks")
		c:writeU32(r.next_apply, p and p .. "next_apply")
	end

	-- Active vulnerability pulses.
	local vp = s.vuln_pulses
	c:writeU32(#vp, "vuln_pulses.len")
	for i = 1, #vp do
		local x = vp[i]
		local p = vb and "vuln_pulses[" .. i .. "]."
		c:writeU32(x.magnitude, p and p .. "magnitude")
		c:writeI64(x.range, p and p .. "range")
		c:writeU32(x.interval_ticks, p and p .. "interval_ticks")
		c:writeU32(x.next_tick, p and p .. "next_tick")
	end

	-- Pending meta perk (duplicator/voucher) armed for the next purchase.
	local perk = s.pending_perk
	if perk ~= nil then
		c:writeU32(1, "pending_perk.some")
		c:writeU32(perk.rarity, "pending_perk.rarity")
		c:writeU32(perk.extra_copies, "pending_perk.extra_copies")
		c:writeU32(boolU32(perk.free), "pending_perk.free")
	else
		c:writeU32(0, "pending_perk.some")
	end

	-- Shop offers (slot order is meaningful).
	c:writeU32(s.shop.shop_seq, "shop.shop_seq")
	local offers = s.shop.offers
	c:writeU32(#offers, "shop.offers.len")
	for i = 1, #offers do
		local o = offers[i]
		local p = vb and "shop.offers[" .. i .. "]."
		c:writeU32(offerKindCode(o.kind), p and p .. "kind")
		c:writeU32(o.def, p and p .. "def")
		c:writeI64(o.cost, p and p .. "cost")
	end

	-- Enemies in id order.
	local el = sortedById(s.enemies, "id")
	c:writeU32(#el, "enemies.len")
	for i = 1, #el do
		local x = el[i]
		local p = vb and "enemies[" .. i .. "]."
		c:writeU32(x.id, p and p .. "id")
		c:writeU32(x.def, p and p .. "def")
		c:writeI64(x.hp, p and p .. "hp")
		c:writeFixed(x.pos.x, p and p .. "pos.x")
		c:writeFixed(x.pos.y, p and p .. "pos.y")
		local st = x.status
		c:writeI64(st.poison_dps, p and p .. "status.poison_dps")
		c:writeU32(st.poison_ticks, p and p .. "status.poison_ticks")
		c:writeU32(st.frost_stacks, p and p .. "status.frost_stacks")
		c:writeU32(st.frost_ticks, p and p .. "status.frost_ticks")
		c:writeU32(st.fire_stacks, p and p .. "status.fire_stacks")
		c:writeU32(st.vuln_stacks, p and p .. "status.vuln_stacks")
		c:writeU32(st.stun_ticks, p and p .. "status.stun_ticks")
		c:writeU32(st.freeze_ticks, p and p .. "status.freeze_ticks")
	end

	-- Projectiles in id order.
	local pl = sortedById(s.projectiles, "id")
	c:writeU32(#pl, "projectiles.len")
	for i = 1, #pl do
		local x = pl[i]
		local p = vb and "projectiles[" .. i .. "]."
		c:writeU32(x.id, p and p .. "id")
		c:writeFixed(x.pos.x, p and p .. "pos.x")
		c:writeFixed(x.pos.y, p and p .. "pos.y")
		c:writeU32(x.target, p and p .. "target")
		c:writeI64(x.damage, p and p .. "damage")
		c:writeU32(x.damage_type, p and p .. "damage_type")
		c:writeFixed(x.splash_radius, p and p .. "splash_radius")
		c:writeFixed(x.speed, p and p .. "speed")
		local oh = x.on_hit
		c:writeI64(oh.poison_dps, p and p .. "on_hit.poison_dps")
		c:writeU32(oh.poison_ticks, p and p .. "on_hit.poison_ticks")
		c:writeU32(oh.frost_stacks, p and p .. "on_hit.frost_stacks")
		c:writeU32(oh.fire_stacks, p and p .. "on_hit.fire_stacks")
		c:writeU32(oh.stun_ticks, p and p .. "on_hit.stun_ticks")
		local atag, a, b, d = words(x.ability)
		c:writeU32(atag, p and p .. "ability.tag")
		c:writeI64(a, p and p .. "ability.a")
		c:writeI64(b, p and p .. "ability.b")
		c:writeI64(d, p and p .. "ability.c")
	end

	-- Hazards in id order (land mines / burning oil).
	local hz = sortedById(s.hazards, "id")
	c:writeU32(#hz, "hazards.len")
	for i = 1, #hz do
		local x = hz[i]
		local p = vb and "hazards[" .. i .. "]."
		c:writeU32(x.id, p and p .. "id")
		c:writeFixed(x.pos.x, p and p .. "pos.x")
		c:writeFixed(x.pos.y, p and p .. "pos.y")
		c:writeI64(x.dmg, p and p .. "dmg")
		c:writeU32(x.damage_type, p and p .. "damage_type")
		c:writeI64(x.radius, p and p .. "radius")
		c:writeU32(x.ticks_left, p and p .. "ticks_left")
	end

	-- Summoned allies (minions) in id order.
	local mn = sortedById(s.minions, "id")
	c:writeU32(#mn, "minions.len")
	for i = 1, #mn do
		local x = mn[i]
		local p = vb and "minions[" .. i .. "]."
		c:writeU32(x.id, p and p .. "id")
		c:writeFixed(x.pos.x, p and p .. "pos.x")
		c:writeFixed(x.pos.y, p and p .. "pos.y")
		c:writeU32(x.kind, p and p .. "kind")
		c:writeI64(x.hp, p and p .. "hp")
		c:writeI64(x.damage, p and p .. "damage")
		c:writeU32(x.damage_type, p and p .. "damage_type")
		c:writeU32(x.next_attack_tick, p and p .. "next_attack_tick")
		c:writeU32(x.expire_tick, p and p .. "expire_tick")
	end
end

-- ============================ public checksum API ============================

--- The `state_checksum` in the canonical wire form: 16 lowercase hex digits
--- (`CONTRACTS.md` §C4). A Luau number cannot hold a u64, so the string IS the
--- value here; comparing against a trace is plain string equality.
function Arena.checksum(s): string
	local c = Checksum.new()
	traverse(s, c)
	return c:finishHex()
end

--- The same digest as two u32 halves `(hi, lo)`, for callers that want it
--- numerically (e.g. to put on a wire).
function Arena.checksumParts(s)
	local c = Checksum.new()
	traverse(s, c)
	return c:finish()
end

--- Every checksummed write, in checksum order, as the verbose `Checksum` log:
--- `{label, kind, value, digest}` per entry, where `digest` is the running
--- checksum AFTER that write. Drives the trace runner's `--verbose` dump — when
--- a trace diverges, the first differing field (and hence the first differing
--- running digest) is one `diff` away (`SIM-SPEC.md` §S6). Read-only.
function Arena.checksumFields(s)
	local c = Checksum.newVerbose()
	traverse(s, c)
	return c.log
end

return Arena
