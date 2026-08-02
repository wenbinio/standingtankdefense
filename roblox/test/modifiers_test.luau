--!nonstrict
--[[
	modifiers_test.luau — oracle test for `sim/Modifiers.luau` + `sim/Economy.luau`.

	WHAT THIS IS
	------------
	Every expected value in the oracle was produced by RUNNING THE RUST
	(`sim-core/crates/sim/src/{modifiers,economy}.rs`) — never hand-derived
	(`CONTRACTS.md` C4 discipline, `SIM-SPEC.md` §S1). The generator
	(`sim-core/crates/sim/src/export/oracle.rs`) drives the real `apply_effect` /
	`apply_ramps` / `collect_bounties` / `tick_income` / `resolve_deaths` and
	serializes every `Fixed` as its 16-digit two's-complement raw, so a divergence
	is a bit-level divergence, not an "about right".

	It covers, per this transcription's failure modes:
	  * additive-within-a-kind vs multiplicative-across-sources composition, and
	    the ORDER of the multiplicative chain (reordering changes the result);
	  * every `ModEffect` tag (0..47) folded in one pass, with the aggregate,
	    tank and economy compared field by field;
	  * the retroactive `%`-riders (`MaxHpPct` / `HpRegenPct` / `ManaRegenPct`),
	    which read the CURRENT stat — so bundle order is load-bearing — plus
	    their `STAT_CEIL` clamps and the dodge hard-cap;
	  * `applyRamps` a tick before the interval, exactly on it, twice on the same
	    tick, and after a multi-interval jump (the catch-up `while`);
	  * income scoping — `income_mult` scales passive income, `bounty_mult` never
	    touches it — plus the regen/shield riders and the dead-tank gates;
	  * bounty procs against a FIXED RNG state: gold AND the post-op `rng_proc`
	    cursor are asserted after every operation, so one extra or skipped draw
	    fails loudly (`SIM-SPEC.md` §S4);
	  * the POWER-CURVE SOFT CAPS (`docs/11 §11.2`) on all seven knees — Max HP,
	    Armor, HP regen, Mana-Shield pool, Mana-Shield regen, the multiplicative
	    damage product, and `INCOME_REGEN_SOFT` on `IncomeRegenPct` (the ×0.25 knee
	    added by the `docs/11 §11.8` pass). Identity below the knee, full value
	    EXACTLY at it (continuity), cubic falloff above it (1/8 at 2×, 1/64 at
	    4×), monotonicity, attenuated reductions, and the `STAT_CEIL` clamp in
	    both directions. The 60-deep `[+2000 flat, +25%]` bundle and the 250-source
	    multiplicative chain matter most: they are what drive the `SOFT^3 / mag^3`
	    intermediate past 2^53, where a Luau port that lets it land in a raw
	    number goes silently wrong. `INCOME_REGEN_SOFT` gets the only case that
	    leaves its identity region — 150 copies of `Entangled Gold Mine`'s +25%,
	    the exact stack the degeneracy pass measured, which must land at ×1.2436
	    of income healed rather than the ×37.5 an uncapped sum would give;
	  * `arsenalSynergyAdd`'s caps table, as its own `synergy_cases` array — every
	    prefix of the damage-type and attack-class axes, the distinct-def axis
	    through and past its 8-extra cap, the catalog tail (so the upper words of
	    the 256-bit def bitset are exercised), and `Magic Bolt ×14`, which must
	    score EXACTLY +0% because copies are neither rewarded nor taxed.

	STALENESS
	---------
	`content_hash` is ASSERTED against `Content.CONTENT_HASH`. This oracle silently
	passed once while carrying pre-rebalance expectations, because the hash was
	recorded but never checked; it is checked now, so a regenerated catalog fails
	here loudly instead of asserting the port against a fossil.

	HOW TO RUN
	----------
	Standalone Luau has no file I/O, so the oracle arrives as a GENERATED Luau
	module (`OracleData.luau`), exactly the way the catalog reaches `Content.luau`
	through `ContentData.luau`. From `roblox/test`:

	    luau modifiers_test.luau

	A larger oracle document may be supplied instead, as JSON chunks after `-a`
	(the `parity.luau` convention); the `-a` text wins over the module:

	    luau modifiers_test.luau -a "$(cat big_oracle.json)"

	Exits non-zero on any divergence.

	REGENERATING
	------------
	Never hand-edit an expected value — that would make the test assert the port
	against itself and destroy the point of having an oracle. Regenerate the WHOLE
	document with one command, from the repo root:

	    cd sim-core && cargo run --release -p sim --bin export-oracle

	That rewrites `roblox/test/OracleData.luau` from
	`sim-core/crates/sim/src/export/oracle.rs`, and `--check` fails instead of
	writing if the committed file is stale. Every number in it — `effect_cases`,
	`synergy_cases`, `ramp_cases`, `econ_cases` and the scalar header — is computed
	by running the Rust; the module holds only the case INPUTS.

	The generator lives INSIDE the `sim` lib for one reason: `modifiers::
	apply_ramps` and every `economy::*` entry point are `pub(crate)`, so the
	previous out-of-repo generator could not call them and `ramp_cases` /
	`econ_cases` had to be carried forward by hand between passes. They duly went
	stale — the `docs/11 §11.8` pass left 10 assertions red, in two clusters:

	  * `ramp[Living Fortress]` `.tank.hp_regen_per_tick` (7 assertions): `MaxHp`
	    now also grants HP regen via `grant_pool_regen`, so a Max-HP ramp raises
	    regen every interval. The expectations still read 0.
	  * `econ[income_regen_ceiling_{early,partial_ramp,scaled_by_healing_mult}]`
	    `.tank.hp` (3 assertions): `economy::INCOME_REGEN_CAP_POOLS` moved 5→3 and
	    `INCOME_REGEN_CAP_POW` 4→5, so `income_regen_tick_cap` is strictly tighter.

	Both were STALENESS, not a port divergence — the Luau's answers matched the
	current Rust arithmetic exactly. "Carried forward verbatim" no longer exists as
	a category here, and no `pub(crate)` was widened to end it.
]]

local ARGS = { ... }

local Fixed = require("../src/shared/Fixed")
local Rng = require("../src/shared/Rng")
local Json = require("../src/shared/Json")
local Content = require("../src/shared/Content")
local State = require("../src/shared/sim/State")
local Modifiers = require("../src/shared/sim/Modifiers")
local Economy = require("../src/shared/sim/Economy")

-- The oracle document, emitted by RUNNING THE RUST (see REGENERATING above).
-- GENERATED by `cargo run -p sim --bin export-oracle`; never hand-edit it.
local OracleData = require("./OracleData")

-- ============================ harness ============================

local total, bad = 0, 0
local failures = {}

local function check(what: string, got: string, want: string)
	total += 1
	if got ~= want then
		bad += 1
		if #failures < 30 then
			table.insert(failures, string.format("  %s\n    got  %s\n    want %s", what, got, want))
		end
	end
end

local function checkFixed(what: string, got, want: string)
	check(what, Fixed.toHex(got), want)
end

local function checkInt(what: string, got: number, want)
	check(what, tostring(got), tostring(want))
end

-- `Rng.toHex` renders with an `0x` prefix; the oracle carries the bare 16-digit
-- form (`CONTRACTS.md` C4), so compare on that.
local function rngHex(r): string
	local hi, lo = Rng.state(r)
	return string.format("%08x%08x", hi, lo)
end

-- ============================ field tables ============================

-- The oracle serializes `Fixed` as a raw hex bit pattern; it is rebuilt with
-- `Fixed.fromHex`, never through a Luau number (`CONTRACTS.md` C4).
local FIXED_TANK = { "spikes_mult", "shield_active_dr", "healing_mult", "missing_hp_heal_pct" }
local FIXED_ECON = {
	"income_mult",
	"income_regen_pct",
	"bounty_mult",
	"bounty_proc_bonus",
	"gold_per_damage",
	"income_shield_pct",
}
local INT_TANK = {
	"hp",
	"max_hp",
	"armor",
	"dodge_num",
	"dodge_den",
	"mana_shield",
	"mana_shield_max",
	"mana_regen_per_tick",
	"hp_regen_per_tick",
	"spikes_damage",
	"heal_on_damaged",
	"heal_on_kill",
	"mana_on_kill",
	"heal_on_poison",
	"revives",
	"revive_bonus_hp",
	"shieldbreak_stun_range",
	"shieldbreak_stun_ticks",
	"spikes_poison_dps",
	"spikes_poison_ticks",
	"spikes_stack_per",
	"spikes_stacks",
	"spikes_stacks_max",
	"aura_range",
	"aura_cadence",
	"aura_damage",
	"aura_poison_dps",
	"aura_poison_ticks",
	"aura_tick",
}
local INT_ECON = { "gold", "income_per_tick", "bounty_proc_chance_pct" }
local FIXED_MODS = {
	"add_global",
	"mul_global",
	"attack_speed",
	"vs_stunned",
	"vs_poisoned",
	"poison_dmg_mult",
	"stun_dur_mult",
	"dmg_per_maxhp_rate",
	"dmg_per_bounty_rate",
	"shield_active_dmg",
}
local ON_HIT_FIELDS = { "poison_dps", "poison_ticks", "frost_stacks", "fire_stacks", "stun_ticks" }

-- Forced pre-state, so the clamps / ceilings / overheal paths are reachable.
local SETUP: { [string]: (any, number) -> () } = {
	gold = function(s, v)
		s.economy.gold = v
	end,
	income_per_tick = function(s, v)
		s.economy.income_per_tick = v
	end,
	bounty_proc_chance_pct = function(s, v)
		s.economy.bounty_proc_chance_pct = v
	end,
	hp = function(s, v)
		s.tank.hp = v
	end,
	max_hp = function(s, v)
		s.tank.max_hp = v
	end,
	-- Needed to place `armor` above its soft-cap knee (1000); the flat `Armor`
	-- effect alone cannot reach it from a fresh tank inside one case.
	armor = function(s, v)
		s.tank.armor = v
	end,
	mana_shield = function(s, v)
		s.tank.mana_shield = v
	end,
	mana_shield_max = function(s, v)
		s.tank.mana_shield_max = v
	end,
	heal_on_kill = function(s, v)
		s.tank.heal_on_kill = v
	end,
	mana_on_kill = function(s, v)
		s.tank.mana_on_kill = v
	end,
	revives = function(s, v)
		s.tank.revives = v
	end,
	revive_bonus_hp = function(s, v)
		s.tank.revive_bonus_hp = v
	end,
	hp_regen_per_tick = function(s, v)
		s.tank.hp_regen_per_tick = v
	end,
	mana_regen_per_tick = function(s, v)
		s.tank.mana_regen_per_tick = v
	end,
	dodge_den = function(s, v)
		s.tank.dodge_den = v
	end,
	aura_cadence = function(s, v)
		s.tank.aura_cadence = v
	end,
	tick = function(s, v)
		s.tick = v
	end,
	dead = function(s, v)
		s.dead = v ~= 0
	end,
}

local FIXED_SETUP: { [string]: (any, any) -> () } = {
	income_mult = function(s, v)
		s.economy.income_mult = v
	end,
	income_regen_pct = function(s, v)
		s.economy.income_regen_pct = v
	end,
	income_shield_pct = function(s, v)
		s.economy.income_shield_pct = v
	end,
	bounty_mult = function(s, v)
		s.economy.bounty_mult = v
	end,
	bounty_proc_bonus = function(s, v)
		s.economy.bounty_proc_bonus = v
	end,
	healing_mult = function(s, v)
		s.tank.healing_mult = v
	end,
}

-- ============================ compare / seed ============================

local function compareTank(tag: string, tank, want)
	for _, k in INT_TANK do
		checkInt(tag .. ".tank." .. k, tank[k], want[k])
	end
	for _, k in FIXED_TANK do
		checkFixed(tag .. ".tank." .. k, tank[k], want[k])
	end
end

local function compareEconomy(tag: string, econ, want)
	for _, k in INT_ECON do
		checkInt(tag .. ".economy." .. k, econ[k], want[k])
	end
	for _, k in FIXED_ECON do
		checkFixed(tag .. ".economy." .. k, econ[k], want[k])
	end
end

local function compareModifiers(tag: string, m, want)
	for _, k in FIXED_MODS do
		checkFixed(tag .. ".modifiers." .. k, m[k], want[k])
	end
	for i = 1, 5 do
		checkFixed(tag .. ".add_by_type[" .. i .. "]", m.add_by_type[i], want.add_by_type[i])
	end
	for i = 1, Content.NUM_SCOPES do
		checkFixed(tag .. ".add_by_scope[" .. i .. "]", m.add_by_scope[i], want.add_by_scope[i])
	end
	local rules = m.weapon_count_scaling
	checkInt(tag .. ".#weapon_count_scaling", #rules, #want.weapon_count_scaling)
	for i = 1, math.min(#rules, #want.weapon_count_scaling) do
		local w = want.weapon_count_scaling[i]
		checkInt(tag .. ".wcs[" .. i .. "].weapon_def", rules[i].weapon_def, w.weapon_def)
		checkInt(tag .. ".wcs[" .. i .. "].dmg_type", rules[i].dmg_type, w.dmg_type)
		checkFixed(tag .. ".wcs[" .. i .. "].per", rules[i].per, w.per)
	end
end

local function seedTank(tank, src)
	for _, k in INT_TANK do
		tank[k] = src[k]
	end
	for _, k in FIXED_TANK do
		tank[k] = Fixed.fromHex(src[k])
	end
end

local function seedEconomy(econ, src)
	for _, k in INT_ECON do
		econ[k] = src[k]
	end
	for _, k in FIXED_ECON do
		econ[k] = Fixed.fromHex(src[k])
	end
end

local function seedModifiers(m, src)
	for _, k in FIXED_MODS do
		m[k] = Fixed.fromHex(src[k])
	end
	for i = 1, 5 do
		m.add_by_type[i] = Fixed.fromHex(src.add_by_type[i])
	end
	for i = 1, Content.NUM_SCOPES do
		m.add_by_scope[i] = Fixed.fromHex(src.add_by_scope[i])
	end
	m.weapon_count_scaling = {}
	for _, w in src.weapon_count_scaling do
		table.insert(m.weapon_count_scaling, {
			weapon_def = w.weapon_def,
			dmg_type = w.dmg_type,
			per = Fixed.fromHex(w.per),
		})
	end
end

-- ============================ effect cases ============================

local function runEffectCases(doc)
	for _, case in doc.effect_cases do
		local tag = "effect[" .. case.name .. "]"
		local s = State.new(1, 0)
		s.modifiers = Modifiers.new()
		for _, kv in case.setup do
			SETUP[kv.k](s, kv.v)
		end
		for _, e in case.effects do
			Modifiers.applyEffect(s.modifiers, e, s.economy, s.tank)
		end
		compareModifiers(tag, s.modifiers, case.modifiers)
		compareEconomy(tag, s.economy, case.economy)
		compareTank(tag, s.tank, case.tank)

		local q = case.queries
		for t = 0, 4 do
			checkFixed(
				tag .. ".damageMult(" .. t .. ")",
				Modifiers.damageMult(s.modifiers, t),
				q.damage_mult[t + 1]
			)
		end
		checkFixed(tag .. ".attackSpeedMult", Modifiers.attackSpeedMult(s.modifiers), q.attack_speed_mult)
		checkFixed(
			tag .. ".dynamicGlobalAdd",
			Modifiers.dynamicGlobalAdd(s.modifiers, s.tank, s.economy),
			q.dynamic_global_add
		)
		-- Content ids are 0-based; `WEAPONS[i]` is weapon id `i - 1` (§S3).
		for i = 1, #q.weapon_damage_mult do
			checkFixed(
				tag .. ".weaponDamageMult(def " .. (i - 1) .. ")",
				Modifiers.weaponDamageMult(s.modifiers, Content.WEAPONS[i]),
				q.weapon_damage_mult[i]
			)
		end
		local arsenal = {}
		for _, def in case.arsenal do
			table.insert(arsenal, { def = def })
		end
		for t = 0, 4 do
			checkFixed(
				tag .. ".selfScalingAdd(" .. t .. ")",
				Modifiers.selfScalingAdd(s.modifiers, t, arsenal),
				q.self_scaling_add[t + 1]
			)
		end
		-- The breadth bonus on its own, so a divergence separates "synergy is
		-- wrong" from "the per-weapon-count rules are wrong" — they share one
		-- additive accumulator inside `selfScalingAdd`.
		checkFixed(tag .. ".arsenalSynergyAdd", Modifiers.arsenalSynergyAdd(arsenal), q.arsenal_synergy_add)
		local raw = Content.WEAPONS[q.scale_on_hit.def + 1].on_hit
		local before = table.clone(raw)
		local oh = Modifiers.scaleOnHit(s.modifiers, raw)
		for _, k in ON_HIT_FIELDS do
			checkInt(tag .. ".scaleOnHit." .. k, oh[k], q.scale_on_hit[k])
			-- The Rust takes `StatusOnHit` by value; the Luau input is Content's
			-- own table and must come back untouched.
			checkInt(tag .. ".scaleOnHit-preserves-content." .. k, raw[k], before[k])
		end
	end
end

-- ============================ synergy cases ============================

--[[
	`Modifiers.arsenalSynergyAdd` in isolation — a pure function of the owned
	arsenal, so it needs no state at all and every case is just (arsenal → Fixed).

	Worth its own array rather than folding into `effect_cases`, because the thing
	being pinned is a CAPS TABLE: +10% per damage type beyond the first (4 extra
	max), +8% per attack class (5 max), +5% per distinct def (8 max). The
	interesting failures are all off-by-one in a clamp or a cap, which only show up
	when you walk each axis one step at a time and then past its cap.

	`self_scaling_add` is asserted alongside on a FRESH aggregate (no
	`weapon_count_scaling` rules), which pins that the synergy is the first and only
	term there — i.e. that it really does reach damage through the additive pool.
]]
local function runSynergyCases(doc)
	for _, case in doc.synergy_cases do
		local tag = "synergy[" .. case.name .. "]"
		local arsenal = {}
		for _, def in case.arsenal do
			table.insert(arsenal, { def = def })
		end
		checkFixed(tag, Modifiers.arsenalSynergyAdd(arsenal), case.synergy)
		local m = Modifiers.new()
		for t = 0, 4 do
			-- GLOBAL, not per-type: every damage type must read the same value.
			checkFixed(
				tag .. ".selfScalingAdd(" .. t .. ")",
				Modifiers.selfScalingAdd(m, t, arsenal),
				case.self_scaling_add[t + 1]
			)
		end
	end
end

-- ============================ ramp cases ============================

local function runRampCases(doc)
	for _, case in doc.ramp_cases do
		local tag = "ramp[" .. case.name .. "]"
		local s = State.new(1, 0)
		s.modifiers = Modifiers.new()
		-- Seeded from the post-`buy_modifier` snapshot: this isolates
		-- `applyRamps` from the purchase path, which State/Shop own.
		seedModifiers(s.modifiers, case.pre.modifiers)
		seedEconomy(s.economy, case.pre.economy)
		seedTank(s.tank, case.pre.tank)
		s.ramps = {
			{
				effect = case.ramp.effect,
				interval_ticks = case.ramp.interval_ticks,
				next_apply = case.ramp.next_apply,
			},
		}
		for _, step in case.steps do
			local st = tag .. "@" .. step.tick
			s.tick = step.tick
			Modifiers.applyRamps(s)
			checkInt(st .. ".#ramps", #s.ramps, #step.ramps)
			for i = 1, #step.ramps do
				checkInt(st .. ".ramps[" .. i .. "].next_apply", s.ramps[i].next_apply, step.ramps[i].next_apply)
				checkInt(
					st .. ".ramps[" .. i .. "].interval_ticks",
					s.ramps[i].interval_ticks,
					step.ramps[i].interval_ticks
				)
			end
			compareModifiers(st, s.modifiers, step.modifiers)
			compareEconomy(st, s.economy, step.economy)
			compareTank(st, s.tank, step.tank)
		end
	end
end

-- ============================ economy cases ============================

local function runEconCases(doc)
	for _, case in doc.econ_cases do
		local tag = "econ[" .. case.name .. "]"
		local s = State.new("0x" .. case.seed, 0)
		s.modifiers = Modifiers.new()
		-- The RNG stream must start where the Rust's does, or every proc
		-- assertion below is meaningless.
		check(tag .. ".rng_proc-initial", rngHex(s.rng_proc), case.rng_proc_initial)
		for _, kv in case.setup do
			SETUP[kv.k](s, kv.v)
		end
		for _, kv in case.fixed_setup do
			FIXED_SETUP[kv.k](s, Fixed.fromHex(kv.v))
		end
		s.total_gold_earned = 0
		s.total_damage_dealt = 0

		for i, op in case.ops do
			local st = tag .. "#" .. i .. "." .. op
			if op == "tick_income" then
				Economy.tickIncome(s)
			elseif op == "collect_bounties" then
				s.pending_kills = table.clone(case.pending_kills)
				Economy.collectBounties(s)
			elseif op == "resolve_deaths" then
				Economy.resolveDeaths(s)
			elseif op == "on_round_start" then
				Economy.onRoundStart(s)
			elseif op == "zero_hp" then
				s.tank.hp = 0
			else
				error("unknown op " .. tostring(op))
			end
			local after = case.steps[i].after
			compareEconomy(st, s.economy, after.economy)
			compareTank(st, s.tank, after.tank)
			checkInt(st .. ".total_gold_earned", s.total_gold_earned, after.total_gold_earned)
			checkInt(st .. ".total_damage_dealt", s.total_damage_dealt, after.total_damage_dealt)
			-- `pending_kills` must be empty at every tick boundary.
			checkInt(st .. ".#pending_kills", #s.pending_kills, after.pending_kills)
			check(st .. ".dead", tostring(s.dead), tostring(after.dead))
			check(
				st .. ".death_tick",
				tostring(s.death_tick),
				if after.death_tick == Json.null then "nil" else tostring(after.death_tick)
			)
			-- The RNG cursor after every op: one extra or skipped `below` draw
			-- shows up right here (§S4).
			check(st .. ".rng_proc", rngHex(s.rng_proc), after.rng_proc)
		end
	end
end

-- ============================ run ============================

-- `-a` (the `parity.luau` convention) may supply a larger document instead; with
-- no arguments this runs against the generated module.
local doc = Json.decode(if #ARGS > 0 then table.concat(ARGS) else OracleData)

assert(doc.schema_version == 1, "oracle schema_version")
checkInt("num_scopes", Content.NUM_SCOPES, doc.num_scopes)
-- A stale oracle must fail HERE, not by silently asserting the port against
-- pre-rebalance expectations (see STALENESS in the header).
check("content_hash", Content.CONTENT_HASH, doc.content_hash)

runEffectCases(doc)
runSynergyCases(doc)
runRampCases(doc)
runEconCases(doc)

print(string.format("modifiers_test: %d checks, %d failures", total, bad))
if bad > 0 then
	for _, f in failures do
		print(f)
	end
	if bad > #failures then
		print(string.format("  ... and %d more", bad - #failures))
	end
	error("modifiers_test FAILED")
end
print("modifiers_test OK")
