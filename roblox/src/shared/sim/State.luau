--[[
	State.luau — transcription of `sim-core/crates/sim/src/state.rs` + `ids.rs`.
	Spec: `roblox/SIM-SPEC.md` §S2 (module map), §S3 (naming), §S4 (numerics).

	THIS IS THE CROSS-AGENT SEAM. Every other sim module reads and mutates the
	tables built here. Field names are BYTE-IDENTICAL to the Rust, in snake_case
	(§S3) — that is what makes an oracle trace diff readable. Behavior modules
	READ/WRITE these fields but must not change the shapes.

	REPRESENTATION RULES (§S4)
	--------------------------
	* Rust `Fixed`  -> an OPAQUE `Fixed.luau` value. Never index it.
	* Rust `i64`/`u32`/`u16`/`u8` -> a plain Luau number (every such quantity in
	  this sim sits far inside 2^53).
	* Rust `u64` (`master_seed`) -> a frozen `{ hi, lo }` pair of u32 halves, the
	  same convention `Rng.luau` uses for u64 values. `Rng.seedParts` accepts it
	  directly.
	* Rust `Option<T>` -> `nil`.
	* Rust `Vec<T>` -> a 1-INDEXED Luau array. Content ids (`def`, `slot`) stay
	  0-BASED, because array position IS identity in `content.json` (C1). Every
	  `+ 1` therefore happens at a lookup boundary and is marked.
	* Rust `EntityId(u32)` / `Tick` -> a plain number.
	* Rust enums with payloads (`ModEffect`, `WeaponAbility`, `Attack`) -> the
	  `content.json` WORDS table `{ tag, a, b, c }` (C1), passed through verbatim.

	CHECKSUM CLASSIFICATION (see `lib.rs::checksum`; getting this wrong corrupts
	the R3 gate in a way that is hard to trace):
	* TRANSIENT — always false at a tick boundary, excluded from checksum/snapshot:
	      tank_hit_this_tick, shield_broke_this_tick
	* RENDER-ONLY / telemetry — never fed to the checksum, so it can never affect
	  determinism:
	      bought_attack_mask, weapons_bought, economy_purchases
	  (`Projectile.last_target_pos` is likewise not checksummed, but it IS
	  authoritative sim state — it is simply omitted by `lib.rs::checksum`.)
	* Everything else on `ArenaState` is authoritative and checksummed.

	ENGINE INDEPENDENCE: no Roblox global is referenced. Sibling modules are
	resolved by relative string require, which is the only form that works under
	the bare `luau` interpreter the tests run in.
]]

local Fixed = require("../Fixed")
local Rng = require("../Rng")
local Content = require("../Content")
local ModifiersMod = require("./Modifiers")

local State = {}

-- An opaque `Fixed.luau` value (see §S4 — never index it) and an `Rng.luau`
-- stream. Neither module exports a type, so they are aliased as `any` here
-- rather than being given a shape this file would then be asserting.
export type Fixed = any
export type Rng = any

--=============================================================================
-- ids.rs
--=============================================================================

--- `u32::MAX`. Used as the pre-first-round sentinel for `ArenaState.round` and
--- as the `death_tick == None` stand-in inside the checksum.
State.U32_MAX = 4294967295

--- `ids::Purpose` — independent RNG stream purposes (`docs/05 §5.6`). Values are
--- stable on the wire/checksum — do not reorder.
State.Purpose = table.freeze({
	Spawn = 0,
	Targeting = 1,
	Shop = 2,
	Reroll = 3,
	Proc = 4,
})

--=============================================================================
-- Vec2 — 2D point/vector in Fixed units. The tank sits at the origin.
--=============================================================================

export type Vec2 = { x: Fixed, y: Fixed }

local Vec2 = {}
State.Vec2 = Vec2

--- `Vec2::ZERO`. FROZEN: the Rust type is `Copy`, so every use there is a fresh
--- value. Sharing one table would alias; freezing turns an aliasing bug into a
--- loud error instead of silent state corruption. Use `Vec2.new` to build a
--- mutable point.
Vec2.ZERO = table.freeze({ x = Fixed.ZERO, y = Fixed.ZERO }) :: Vec2

--- `Vec2::new`.
function Vec2.new(x: Fixed, y: Fixed): Vec2
	return { x = x, y = y }
end

--- `Vec2::dist_sq` — squared distance to `o` (no sqrt; use for range checks
--- against `range*range`).
function Vec2.distSq(self: Vec2, o: Vec2): Fixed
	local dx = Fixed.sub(o.x, self.x)
	local dy = Fixed.sub(o.y, self.y)
	return Fixed.add(Fixed.mul(dx, dx), Fixed.mul(dy, dy))
end

--- `Vec2::step_toward` — move from `self` toward `target` by at most `max_step`;
--- clamps to target on arrival. Fully deterministic (integer sqrt). Returns the
--- new point (a FRESH table, mirroring Rust's `Copy` return).
function Vec2.stepToward(self: Vec2, target: Vec2, max_step: Fixed): Vec2
	local dx = Fixed.sub(target.x, self.x)
	local dy = Fixed.sub(target.y, self.y)
	local d2 = Fixed.add(Fixed.mul(dx, dx), Fixed.mul(dy, dy))
	local step2 = Fixed.mul(max_step, max_step)
	if Fixed.le(d2, step2) or Fixed.eq(d2, Fixed.ZERO) then
		return { x = target.x, y = target.y }
	end
	local dist = Fixed.sqrt(d2)
	return {
		x = Fixed.add(self.x, Fixed.div(Fixed.mul(dx, max_step), dist)),
		y = Fixed.add(self.y, Fixed.div(Fixed.mul(dy, max_step), dist)),
	}
end

--- `Vec2::step_away` — move `self` directly AWAY from `from` by `step` units
--- (Knockback). If `self == from` (degenerate, e.g. an enemy exactly on the
--- tank) the point is unchanged. Fully deterministic (integer sqrt).
function Vec2.stepAway(self: Vec2, from: Vec2, step: Fixed): Vec2
	local dx = Fixed.sub(self.x, from.x)
	local dy = Fixed.sub(self.y, from.y)
	local d2 = Fixed.add(Fixed.mul(dx, dx), Fixed.mul(dy, dy))
	if Fixed.eq(d2, Fixed.ZERO) then
		return { x = self.x, y = self.y }
	end
	local dist = Fixed.sqrt(d2)
	return {
		x = Fixed.add(self.x, Fixed.div(Fixed.mul(dx, step), dist)),
		y = Fixed.add(self.y, Fixed.div(Fixed.mul(dy, step), dist)),
	}
end

--=============================================================================
-- content.rs lookups
--
-- Content ids are 0-BASED (array position IS identity, C1); Luau arrays are
-- 1-indexed. These two functions are the ONLY place that conversion happens in
-- this file (§S3).
--=============================================================================

local function weaponDef(def: number)
	return Content.WEAPONS[def + 1]
end

local function modifierDef(def: number)
	return Content.MODIFIERS[def + 1]
end

-- `ModEffect` tags intercepted by `buy_modifier` (they carry per-arena trigger
-- state). Values are `content.rs ModEffect::words` tags — stable on the wire.
local EFF_GRANT_VULN_PULSE = 14
local EFF_GRANT_DUPLICATOR = 24
local EFF_GRANT_VOUCHER = 25
local EFF_GRANT_GOLD = 26
local EFF_TRADE_MAX_HP_FOR_GOLD = 31
local EFF_TRADE_REGEN_FOR_GOLD = 32

-- `ModifierDef::is_economy` / `is_meta` (true if ANY of the def's effects is)
-- are precomputed on each catalog entry by `Content.luau`, so this file reads
-- them rather than carrying a second copy of the tag sets.

--=============================================================================
-- Tank — the player's stationary tank.
--=============================================================================

export type Tank = {
	hp: number,
	max_hp: number,
	pos: Vec2,
	clear_cooldown_end: number,
	armor: number,
	dodge_num: number,
	dodge_den: number,
	mana_shield: number,
	mana_shield_max: number,
	mana_regen_per_tick: number,
	hp_regen_per_tick: number,
	spikes_damage: number,
	spikes_mult: Fixed,
	shield_active_dr: Fixed,
	heal_on_damaged: number,
	heal_on_kill: number,
	mana_on_kill: number,
	heal_on_poison: number,
	healing_mult: Fixed,
	missing_hp_heal_pct: Fixed,
	revives: number,
	revive_bonus_hp: number,
	-- EXPANSION E2 (exotic mechanics)
	shieldbreak_stun_range: number,
	shieldbreak_stun_ticks: number,
	spikes_poison_dps: number,
	spikes_poison_ticks: number,
	spikes_stack_per: number,
	spikes_stacks: number,
	spikes_stacks_max: number,
	aura_range: number,
	aura_cadence: number,
	aura_damage: number,
	aura_poison_dps: number,
	aura_poison_ticks: number,
	aura_tick: number,
}

local Tank = {}
State.Tank = Tank

--- `Tank::heal` — apply healing, scaled by `healing_mult` and capped at
--- `max_hp`. The single chokepoint for every heal so "+% Healing" and the cap
--- live in one place.
function Tank.heal(self: Tank, amount: number)
	if amount <= 0 then
		return
	end
	local scaled = Fixed.toInt(Fixed.scaleI64(self.healing_mult, amount))
	local hp = self.hp + scaled
	if hp > self.max_hp then
		hp = self.max_hp
	end
	self.hp = hp
end

--- `Tank::restore_mana` — restore `amount` to the Mana-Shield pool, capped at
--- its max. A no-op if the tank has no shield pool. NOT scaled by
--- `healing_mult` (it is shield, not HP).
function Tank.restoreMana(self: Tank, amount: number)
	if amount <= 0 or self.mana_shield_max <= 0 then
		return
	end
	local m = self.mana_shield + amount
	if m > self.mana_shield_max then
		m = self.mana_shield_max
	end
	self.mana_shield = m
end

--=============================================================================
-- WeaponInstance / EnemyStatus / VulnPulse / Enemy / Hazard / Minion /
-- Projectile
--=============================================================================

--- An owned weapon instance (multiple copies of one def stack as separate
--- instances — the source's "everything stacks"). `def` indexes
--- `content::WEAPONS` (0-based).
export type WeaponInstance = {
	instance_id: number,
	def: number,
	next_fire_tick: number,
}

--- Per-enemy status effects (Poison / Frost / Fire / Stun). Pure integer
--- counters (no floats) for determinism. Default = no status.
export type EnemyStatus = {
	poison_dps: number,
	poison_ticks: number,
	frost_stacks: number,
	frost_ticks: number,
	fire_stacks: number,
	vuln_stacks: number,
	stun_ticks: number,
	freeze_ticks: number,
}

local EnemyStatus = {}
State.EnemyStatus = EnemyStatus

--- `EnemyStatus::default()` — a fresh all-zero status.
function EnemyStatus.default(): EnemyStatus
	return {
		poison_dps = 0,
		poison_ticks = 0,
		frost_stacks = 0,
		frost_ticks = 0,
		fire_stacks = 0,
		vuln_stacks = 0,
		stun_ticks = 0,
		freeze_ticks = 0,
	}
end

--- A periodic aura the tank emits: every `interval_ticks` it adds `magnitude`
--- vulnerability stacks to enemies within `range` (Vulnerability Pulse).
export type VulnPulse = {
	magnitude: number,
	range: number,
	interval_ticks: number,
	next_tick: number,
}

--- An active enemy. `def` indexes `content::ENEMIES` (0-based).
export type Enemy = {
	id: number,
	def: number,
	hp: number,
	pos: Vec2,
	status: EnemyStatus,
}

local Enemy = {}
State.Enemy = Enemy

--- `Enemy::new` — construct an enemy with no status (the common case).
function Enemy.new(id: number, def: number, hp: number, pos: Vec2): Enemy
	return { id = id, def = def, hp = hp, pos = pos, status = EnemyStatus.default() }
end

--- A persistent damaging area dropped by a weapon ability (mine field /
--- scorched ground). Each tick it pulses `dmg` to every non-boss enemy within
--- `radius`, for `ticks_left` ticks, then expires. Stored in id order.
export type Hazard = {
	id: number,
	pos: Vec2,
	dmg: number,
	damage_type: number,
	radius: number,
	ticks_left: number,
}

--- A temporary ALLY summoned by a weapon (a raised Larva / Spore). `kind` is the
--- render kind: 0 = larva, 1 = spore. Stored in id order.
export type Minion = {
	id: number,
	pos: Vec2,
	kind: number,
	hp: number,
	damage: number,
	damage_type: number,
	next_attack_tick: number,
	expire_tick: number,
}

--- An in-flight projectile (homes on `target`; applies splash at arrival).
--- `on_hit` is a `content::StatusOnHit` table; `ability` is a
--- `content::WeaponAbility` WORDS table `{ tag, a, b, c }`.
--- `splash_radius == Fixed.ZERO` ⇒ single target.
export type Projectile = {
	id: number,
	pos: Vec2,
	target: number,
	last_target_pos: Vec2,
	damage: number,
	damage_type: number,
	splash_radius: Fixed,
	speed: Fixed,
	on_hit: any,
	ability: any,
}

--=============================================================================
-- Economy / Offer / ShopState / PendingPerk / ActiveRamp / Modifiers
--=============================================================================

export type Economy = {
	gold: number,
	income_per_tick: number,
	income_mult: Fixed,
	income_regen_pct: Fixed,
	bounty_mult: Fixed,
	bounty_proc_chance_pct: number,
	bounty_proc_bonus: Fixed,
	gold_per_damage: Fixed,
	income_shield_pct: Fixed,
	rerolls_remaining: number,
	reroll_cost: number,
}

--- `OfferKind` — what a shop slot sells. The integer values are the ones
--- `lib.rs::checksum` writes for an offer's kind, so they are wire-stable.
State.OfferKind = table.freeze({
	Weapon = 0, -- `def` indexes `content::WEAPONS`
	Modifier = 1, -- `def` indexes `content::MODIFIERS`
})

--- One purchasable shop slot (a weapon or a stacking modifier).
export type Offer = {
	kind: number,
	def: number,
	cost: number,
}

--- The per-round shop. `offers` is a 1-indexed array; a `BuyOffer` input's
--- `slot` is 0-BASED, so it indexes `offers[slot + 1]`.
export type ShopState = {
	offers: { Offer },
	shop_seq: number,
}

--- A one-shot meta perk armed by a meta item (Magic Coin / Duplicator / Black
--- Market), consumed by the next matching non-meta purchase (`docs/06` #5).
export type PendingPerk = {
	rarity: number, -- 255 = any rarity
	extra_copies: number,
	free: boolean,
}

local PendingPerk = {}
State.PendingPerk = PendingPerk

--- `PendingPerk::matches` — whether a purchase at `rarity` qualifies.
function PendingPerk.matches(self: PendingPerk, rarity: number): boolean
	return self.rarity == 255 or self.rarity == rarity
end

--- An active time-scaling growth: re-applies `effect` every `interval_ticks`
--- (the source's "+X every 30 seconds"). `effect` is a `ModEffect` WORDS table.
export type ActiveRamp = {
	effect: any,
	interval_ticks: number,
	next_apply: number,
}

--- One self-scaling damage rule (see `Modifiers.weapon_count_scaling`).
export type WeaponCountScale = {
	weapon_def: number,
	dmg_type: number,
	per: Fixed,
}

--- Aggregated damage & attack-speed modifiers (`docs/05 §5.3`). The struct lives
--- here (so checksum/snapshot can serialize it); the behavior — including the
--- `Modifiers::new` constructor — lives in `Modifiers.luau`, exactly as the Rust
--- splits `state.rs` from `modifiers.rs`.
--- `add_by_type` has 5 entries (Normal, Piercing, Magic, Siege, Chaos) and
--- `add_by_scope` has `content::NUM_SCOPES` (12); both are 1-INDEXED Luau arrays
--- over 0-based ids, i.e. type `t` lives at `add_by_type[t + 1]`.
export type Modifiers = {
	add_global: Fixed,
	add_by_type: { Fixed },
	add_by_scope: { Fixed },
	mul_global: Fixed,
	attack_speed: Fixed,
	vs_stunned: Fixed,
	vs_poisoned: Fixed,
	poison_dmg_mult: Fixed,
	stun_dur_mult: Fixed,
	weapon_count_scaling: { WeaponCountScale },
	dmg_per_maxhp_rate: Fixed,
	dmg_per_bounty_rate: Fixed,
	shield_active_dmg: Fixed,
}

--=============================================================================
-- ArenaState
--=============================================================================

--- The complete authoritative arena state. `Arena.step` is a pure function of
--- this plus the tick's `Input` (`docs/03 §3.3`).
export type ArenaState = {
	tick: number,
	round: number, -- `State.U32_MAX` sentinel before the first round starts
	master_seed: { hi: number, lo: number },
	player_id: number,

	tank: Tank,
	weapons: { WeaponInstance },
	enemies: { Enemy },
	projectiles: { Projectile },
	hazards: { Hazard },
	minions: { Minion },
	economy: Economy,
	shop: ShopState,
	modifiers: Modifiers,
	ramps: { ActiveRamp },
	vuln_pulses: { VulnPulse },
	pending_perk: PendingPerk?,

	-- TRANSIENT (see the header): false at every tick boundary, not checksummed.
	tank_hit_this_tick: boolean,
	shield_broke_this_tick: boolean,

	next_entity_id: number,
	dead: boolean,
	death_tick: number?,
	pending_kills: { number },

	total_damage_dealt: number,
	total_gold_earned: number,

	-- RENDER/PROFILE-ONLY telemetry (see the header): never checksummed.
	bought_attack_mask: number,
	weapons_bought: number,
	economy_purchases: number,

	-- Per-purpose RNG streams (cursors ride in snapshots).
	rng_spawn: Rng,
	rng_targeting: Rng,
	rng_shop: Rng,
	rng_reroll: Rng,
	rng_proc: Rng,
}

--- `ArenaState::new` — fresh arena for `master_seed`/`player_id`. Starts with one
--- Bow so combat is exercised from tick 0; the first `step()` generates the
--- round-0 shop.
---
--- `master_seed` accepts anything `Rng.seedParts` does: a number below 2^53, a
--- decimal string, a `"0x…"` string, or a `{ hi, lo }` pair. It is NORMALIZED to
--- a frozen `{ hi, lo }` pair on the state, because a u64 has no exact plain-
--- number form (§S4).
function State.new(master_seed: any, player_id: number): ArenaState
	local shi, slo = Rng.seedParts(master_seed)
	local seed = table.freeze({ hi = shi, lo = slo })

	-- `let d = |p: Purpose| Rng::derive(master_seed, player_id, p as u32, 0);`
	local function d(purpose: number)
		return Rng.derive(seed, player_id, purpose, 0)
	end

	local s: ArenaState = {
		tick = 0,
		round = State.U32_MAX,
		master_seed = seed,
		player_id = player_id,
		tank = {
			hp = 24000,
			max_hp = 24000,
			pos = Vec2.ZERO,
			clear_cooldown_end = 0,
			armor = 0,
			dodge_num = 0,
			dodge_den = 100,
			mana_shield = 0,
			mana_shield_max = 0,
			mana_regen_per_tick = 0,
			hp_regen_per_tick = 0,
			spikes_damage = 0,
			spikes_mult = Fixed.ONE,
			shield_active_dr = Fixed.ZERO,
			heal_on_damaged = 0,
			heal_on_kill = 0,
			mana_on_kill = 0,
			heal_on_poison = 0,
			healing_mult = Fixed.ONE,
			missing_hp_heal_pct = Fixed.ZERO,
			revives = 0,
			revive_bonus_hp = 0,
			shieldbreak_stun_range = 0,
			shieldbreak_stun_ticks = 0,
			spikes_poison_dps = 0,
			spikes_poison_ticks = 0,
			spikes_stack_per = 0,
			spikes_stacks = 0,
			spikes_stacks_max = 0,
			aura_range = 0,
			aura_cadence = 0,
			aura_damage = 0,
			aura_poison_dps = 0,
			aura_poison_ticks = 0,
			aura_tick = 0,
		},
		weapons = {},
		enemies = {},
		projectiles = {},
		hazards = {},
		minions = {},
		economy = {
			gold = 500,
			income_per_tick = 20, -- 600 gold/s baseline (tuning)
			income_mult = Fixed.ONE,
			income_regen_pct = Fixed.ZERO,
			bounty_mult = Fixed.ONE,
			bounty_proc_chance_pct = 0,
			bounty_proc_bonus = Fixed.ZERO,
			gold_per_damage = Fixed.ZERO,
			income_shield_pct = Fixed.ZERO,
			rerolls_remaining = 5,
			reroll_cost = 100,
		},
		shop = { offers = {}, shop_seq = 0 }, -- `ShopState::default()`
		modifiers = ModifiersMod.new(),
		ramps = {},
		vuln_pulses = {},
		pending_perk = nil,
		tank_hit_this_tick = false,
		shield_broke_this_tick = false,
		next_entity_id = 1,
		dead = false,
		death_tick = nil,
		pending_kills = {},
		total_damage_dealt = 0,
		total_gold_earned = 0,
		bought_attack_mask = 0,
		weapons_bought = 0,
		economy_purchases = 0,
		-- Stream derivation order matches the Rust field order exactly; `derive`
		-- is a pure hash, so this is order-independent, but keep it aligned.
		rng_spawn = d(State.Purpose.Spawn),
		rng_targeting = d(State.Purpose.Targeting),
		rng_shop = d(State.Purpose.Shop),
		rng_reroll = d(State.Purpose.Reroll),
		rng_proc = d(State.Purpose.Proc),
	}

	local id = State.allocEntityId(s)
	s.weapons[#s.weapons + 1] = {
		instance_id = id,
		def = Content.constants.starting_weapon,
		next_fire_tick = 0,
	}
	return s
end

--- `ArenaState::alloc_entity_id` — allocate a fresh, never-reused entity id.
function State.allocEntityId(s: ArenaState): number
	local id = s.next_entity_id
	s.next_entity_id = id + 1
	return id
end

--- `ArenaState::enemy_index` — index of a live enemy by id, if present.
--- NOTE: returns the 1-BASED Luau array index (the Rust returns a 0-based
--- `usize`), so the result indexes `s.enemies` directly. `nil` if absent.
function State.enemyIndex(s: ArenaState, id: number): number?
	local enemies = s.enemies
	for i = 1, #enemies do
		if enemies[i].id == id then
			return i
		end
	end
	return nil
end

--- `ArenaState::award_gold` — the single chokepoint for every gold gain: credits
--- the wallet AND the `total_gold_earned` scoreboard. ALL gold income (bounty,
--- passive income, grants, trades) routes through here so the scoreboard stays
--- authoritative.
---
--- The Rust adds SATURATINGLY at the i64 boundary. A Luau number cannot express
--- that boundary, and every reachable total sits far inside 2^53 (§S4), so the
--- add is plain. See the report note on the i64-saturation gap.
function State.awardGold(s: ArenaState, amount: number)
	if amount == 0 then
		return
	end
	s.economy.gold += amount
	s.total_gold_earned += amount
end

--- `ArenaState::record_player_damage` — record `amount` of damage dealt by a
--- PLAYER source: accumulates the scoreboard total and pays the damage-scaled
--- bounty (Bloodmoney) through `award_gold`. Called once per damage site.
function State.recordPlayerDamage(s: ArenaState, amount: number)
	if amount <= 0 then
		return
	end
	s.total_damage_dealt += amount
	if Fixed.compare(s.economy.gold_per_damage, Fixed.ZERO) > 0 then
		State.awardGold(s, Fixed.toInt(Fixed.scaleI64(s.economy.gold_per_damage, amount)))
	end
end

--- `ArenaState::buy_modifier` — apply a purchased modifier (folds into the
--- damage/attack-speed aggregate, and applies any immediate economy/defensive
--- effect).
---
--- Scans the modifier's effects IN ORDER. A few effects carry per-arena trigger
--- state and are intercepted here; every other effect folds into the
--- Modifiers/Tank/Economy aggregates via the normal path. (Each catalog entry
--- carries a single effect today, so this reproduces the prior single-effect
--- behavior exactly.)
function State.buyModifier(s: ArenaState, def_idx: number)
	local def = modifierDef(def_idx)
	local ramp = def.ramp
	for _, effect in ipairs(def.effects) do
		local tag = effect.tag
		if tag == EFF_GRANT_VULN_PULSE then
			-- GrantVulnPulse(mag, range, interval); `mag as u16`, `interval as u32`.
			local interval = effect.c % 4294967296
			s.vuln_pulses[#s.vuln_pulses + 1] = {
				magnitude = effect.a % 65536,
				range = effect.b,
				interval_ticks = interval,
				next_tick = s.tick + interval,
			}
		-- META items (`docs/06` #5) arm the purchase flow / grant gold.
		elseif tag == EFF_GRANT_DUPLICATOR then
			-- GrantDuplicator(rarity, copies); `rarity as u8`, `copies as u32`.
			s.pending_perk = {
				rarity = effect.a % 256,
				extra_copies = effect.b % 4294967296,
				free = false,
			}
		elseif tag == EFF_GRANT_VOUCHER then
			s.pending_perk = { rarity = effect.a % 256, extra_copies = 0, free = true }
		elseif tag == EFF_GRANT_GOLD then
			State.awardGold(s, effect.a)
		-- HP/defense <-> Gold trades: pay tank stats for gold, intercepted here so
		-- the gold routes through the scoreboard.
		elseif tag == EFF_TRADE_MAX_HP_FOR_GOLD then
			s.tank.max_hp -= effect.a
			if s.tank.hp > s.tank.max_hp then
				s.tank.hp = s.tank.max_hp
			end
			State.awardGold(s, effect.b)
		elseif tag == EFF_TRADE_REGEN_FOR_GOLD then
			-- MAY go negative — a negative regen drains HP each tick.
			s.tank.hp_regen_per_tick -= effect.a
			State.awardGold(s, effect.b)
		else
			ModifiersMod.applyEffect(s.modifiers, effect, s.economy, s.tank)
		end
	end
	-- `if let Some(r) = ramp`. The `.effect ~= nil` guard also absorbs a
	-- `Json.null` that survived content decoding (§S3: `Option<T>` -> `nil`).
	if ramp ~= nil and ramp.effect ~= nil then
		s.ramps[#s.ramps + 1] = {
			effect = ramp.effect,
			interval_ticks = ramp.interval_ticks,
			next_apply = s.tick + ramp.interval_ticks,
		}
	end
end

--- `ArenaState::grant_offer` — grant one unit of an offer (a weapon instance or
--- a modifier application), independent of gold/perk handling. Used by `Input`
--- for the base purchase and for each extra duplicator copy.
function State.grantOffer(s: ArenaState, offer: Offer)
	if offer.kind == State.OfferKind.Weapon then
		local id = State.allocEntityId(s)
		local def = weaponDef(offer.def)
		-- Telemetry (cosmetic, off-checksum): record the attack class bought.
		-- `Attack`'s WORDS tag IS `attack_scope_id` (CONTRACTS.md C1 addendum,
		-- test-asserted in the Rust), so a port carries one mapping, not two.
		s.bought_attack_mask =
			bit32.band(bit32.bor(s.bought_attack_mask, bit32.lshift(1, def.attack.tag)), 0xFFFF)
		s.weapons_bought += 1
		s.weapons[#s.weapons + 1] = {
			instance_id = id,
			def = offer.def,
			next_fire_tick = s.tick,
		}
	else
		if modifierDef(offer.def).is_economy then
			s.economy_purchases += 1
		end
		State.buyModifier(s, offer.def)
	end
end

--- `ArenaState::offer_rarity` — a purchase's rarity (for perk matching).
function State.offerRarity(offer: Offer): number
	if offer.kind == State.OfferKind.Weapon then
		return weaponDef(offer.def).rarity
	end
	return modifierDef(offer.def).rarity
end

--- `ArenaState::offer_is_meta` — whether an offer is a META item (must not
--- trigger or be duplicated).
function State.offerIsMeta(offer: Offer): boolean
	return offer.kind == State.OfferKind.Modifier and modifierDef(offer.def).is_meta
end

return State
