--!nonstrict
--[[
	Economy.luau — transcription of `sim-core/crates/sim/src/economy.rs`
	(`roblox/SIM-SPEC.md` §S1: this is a transcription, not a redesign).

	Economy — AGENT C. Integer gold. Bounty scales by `s.economy.bounty_mult`;
	passive income does NOT (source rule, `docs/02 §2.4`). `income_mult` is its
	own, separate lever and is the ONLY multiplier passive income sees.

	TRANSCRIPTION NOTES (Luau-specific, no behavior of their own)
	------------------------------------------------------------
	* `Fixed` values stay `Fixed.luau` values and are never indexed (§S4);
	  `Fixed::scale_i64` results are converted with `Fixed.toInt` at the point
	  the Rust would already be holding an `i64`. `toInt` errors above 2^53
	  instead of rounding; per §S4 no gold/heal quantity reaches that, and a
	  loud failure beats a silently wrong wallet.
	* `award_gold` / `Tank::heal` / `Tank::restore_mana` live in `state.rs`, so
	  the `total_gold_earned` scoreboard accumulation and the `healing_mult` /
	  max-HP / pool caps are reached through `State`, not duplicated here.
	* Enemy defs are 0-based content ids; `Content.ENEMIES` is a 1-indexed Luau
	  array (§S3) — see `enemyDef` below.
	* `rng_proc` is drawn EXACTLY where the Rust draws it: once per kill, and
	  only when `bounty_proc_chance_pct > 0`. The `and` short-circuits the same
	  way the Rust's `&&` does, so a player with no proc leaves the stream
	  cursor untouched (§S4).
	* The income-regen ceiling is FOUR SEPARATE truncating steps, and Luau's `//`
	  is not taken on trust for any of them (§S4) — see `incomeRegenTickCap`.
	* No Roblox global is referenced.
]]

local Fixed = require("../Fixed")
local Rng = require("../Rng")
local Content = require("../Content")
local State = require("./State")

local Economy = {}

local ZERO = Fixed.ZERO
-- `Tank::heal` / `Tank::restore_mana` — the healing chokepoints in `state.rs`
-- (`healing_mult` and the max-HP / pool caps live there, in one place).
local tankHeal = State.Tank.heal
local tankRestoreMana = State.Tank.restoreMana

-- Content ids are 0-based (§S3); Luau array storage is 1-indexed.
local function enemyDef(def: number)
	return Content.ENEMIES[def + 1]
end

-- ===================== the income-regen ceiling =====================

-- Scalar content constants. `Content.luau` may expose them either as named
-- fields (the `content.rs` spelling) or under `constants` (the `content.json`
-- spelling); `Combat.luau` and `Status.luau` hedge the same way, so mirror them
-- rather than pin one shape and break on the other.
local function contentConst(name: string, snake: string): number
	local v = Content[name]
	if v == nil and Content.constants ~= nil then
		v = Content.constants[snake]
	end
	if v == nil then
		error("Economy: Content is missing " .. name, 2)
	end
	return v
end

-- `content::BOSS_SPAWN_TICK` — read from the C1 artifact, never pinned here.
-- `economy.rs` uses the module constant DIRECTLY (it is not timeline-aware; there
-- is no `Arena.timelineFor` in the Rust's `income_regen_tick_cap`), and the
-- flattened name is the Steam value, which is what the oracle runs. Transcribed
-- literally: reading `Arena.bossSpawnTick(s)` here would be a redesign (§S1).
local BOSS_SPAWN_TICK = contentConst("BOSS_SPAWN_TICK", "boss_spawn_tick")

-- `economy.rs`'s own module constants. Unlike the boss numbers in `Combat.luau`
-- these are NOT in the C1 export (they are `pub(crate) const` in `economy.rs`,
-- not `content.rs`), so there is nothing to read them from and they are pinned
-- here against their Rust declaration. If `economy.rs` retunes them, this pair
-- must move with it — the trace corpus is what catches a miss.
local INCOME_REGEN_CAP_POOLS = 5
local INCOME_REGEN_CAP_POW = 4

-- Largest value the ceiling arithmetic may reach and stay exact in a double.
-- See `Economy.incomeRegenTickCap` for why the bound cannot be hit in practice
-- and why this errors instead of silently truncating.
local MAX_EXACT = 9007199254740992 -- 2^53

--- One truncating step of the ramp: `floor(c * tick / BOSS_SPAWN_TICK)`.
---
--- Rust's `/` on integers truncates toward zero; both operands are non-negative
--- here, so that IS `floor` — but §S4 forbids taking Luau's `//` (or a bare
--- `math.floor` of a float quotient) on trust, so the float quotient is corrected
--- by an exact integer multiply. `n` and `q * BOSS_SPAWN_TICK` are both ≤ the
--- guarded bound, so the correction itself is exact and a 1-ulp division error
--- cannot survive it.
local function capStep(c: number, tick: number): number
	local n = c * tick
	if n > MAX_EXACT then
		-- Unreachable for any pool this game produces (see the note on
		-- `incomeRegenTickCap`); a loud failure beats a silently wrong ceiling,
		-- which would diverge from the Rust rather than merely round.
		error("Economy: income-regen ceiling intermediate exceeds 2^53", 2)
	end
	local q = math.floor(n / BOSS_SPAWN_TICK)
	if q * BOSS_SPAWN_TICK > n then
		q -= 1
	elseif (q + 1) * BOSS_SPAWN_TICK <= n then
		q += 1
	end
	return q
end

--- `economy::income_regen_tick_cap` — the per-tick ceiling on the
--- income-as-HP-regen heal:
---
---     cap(max_hp, t) = INCOME_REGEN_CAP_POOLS × max_hp × (t / BOSS_SPAWN_TICK)^4
---
--- evaluated as FOUR SEPARATE TRUNCATING STEPS. The per-step floor is part of the
--- definition: folding the four multiplies and four divisions into one
--- multiply-then-divide changes the result, so it must not be folded (`docs/11
--- §11.8`, and the `economy.rs` doc comment says so explicitly for this port).
---
--- It reaches its full value of `POOLS × max_hp` per tick exactly when the boss
--- arrives and is a rounding error in the opening minutes — a time gate on the
--- opening, not a magnitude bound. `income_regen_pct` stacks additively per copy
--- with no cap AND is an economy modifier, so the uncapped heal is
--- `pct × copies × income` and an all-economy build maximises both factors; the
--- ceiling is what stops a weaponless eco tank from being unkillable by tick
--- 1200. `economy.rs` carries the full derivation, including why a flat fraction
--- of `max_hp` provably cannot work and why 4 is the smallest exponent that can.
---
--- EXACTNESS. The largest intermediate is `c × tick`, i.e. `POOLS × max_hp × tick`
--- on the first step (later steps only exceed it once `tick > BOSS_SPAWN_TICK`,
--- and by at most `(55200/54000)^3 < 1.07` over the corpus's run length). It is
--- exact in a double for any `max_hp` below `2^53 / (5 × tick)` ≈ 3.3e10 at
--- `tick = 54000` — five orders of magnitude above the worst `max_hp` the Rust
--- has ever observed (448k, `economy.rs`), and the corpus's own maximum is far
--- lower still. Rust's `i64` admits pools up to `modifiers::STAT_CEIL` (1e12)
--- where a double would stop being exact, so `capStep` checks rather than
--- assumes: unreachable in play, but it fails loudly instead of quietly
--- disagreeing with the oracle.
function Economy.incomeRegenTickCap(max_hp: number, tick: number): number
	local c = max_hp * INCOME_REGEN_CAP_POOLS
	for _ = 1, INCOME_REGEN_CAP_POW do
		c = capStep(c, tick)
	end
	return c
end

--- Called once when a new round begins (before input). M0: keep simple
--- (no-op, or a small flat per-round bonus). Deterministic.
function Economy.onRoundStart(s)
	-- M0: no per-round bonus. Kept as a deterministic no-op.
	local _ = s
end

--- Phase 7: drain `s.pending_kills`; for each enemy def add
--- `floor(EnemyDef::bounty * bounty_mult)` to gold. Leave `pending_kills` empty.
---
--- `pending_kills` is populated by combat during the tick and drained here, so
--- it is empty at every tick boundary (which is why it is excluded from the
--- checksum).
function Economy.collectBounties(s)
	local economy = s.economy
	local mult = economy.bounty_mult
	local chance = economy.bounty_proc_chance_pct
	local bonus = economy.bounty_proc_bonus
	local gained = 0
	local kills = 0
	local killsVec = s.pending_kills
	for i = 1, #killsVec do
		local bounty = enemyDef(killsVec[i]).bounty
		local base = Fixed.toInt(Fixed.scaleI64(mult, bounty))
		gained += base
		-- Chance-based bonus bounty (the source's "+X% Bounty with Y% chance").
		-- Only draw RNG when the player actually owns a proc, so the baseline
		-- RNG cursor is untouched for everyone else.
		if chance > 0 and Rng.below(s.rng_proc, 100) < chance then
			gained += Fixed.toInt(Fixed.scaleI64(bonus, base))
		end
		kills += 1
	end
	table.clear(s.pending_kills)
	State.awardGold(s, gained)
	-- On-kill trigger: heal the tank per enemy killed this tick.
	if s.tank.heal_on_kill > 0 and kills > 0 and not s.dead then
		tankHeal(s.tank, kills * s.tank.heal_on_kill)
	end
	-- On-kill trigger: restore Mana Shield per enemy killed this tick (the source's
	-- Maw of Death). Mirrors `heal_on_kill`: applies per kill, capped at the pool
	-- max via `restoreMana` (a no-op if the tank owns no shield pool).
	if s.tank.mana_on_kill > 0 and kills > 0 and not s.dead then
		tankRestoreMana(s.tank, kills * s.tank.mana_on_kill)
	end
end

--- Phase 8: add `floor(income_per_tick × income_mult)` to gold. `bounty_mult`
--- still never touches income (source rule); `income_mult` is its own lever.
--- If `income_regen_pct > 0`, also heal the tank that fraction of the award,
--- **bounded per tick by [`Economy.incomeRegenTickCap`]** (`docs/11 §11.3` #5:
--- bind the tail, stay invisible at p90). The bound is applied to the *input* of
--- `Tank::heal` — so `+% Healing` still scales it and the pool clamp still
--- applies, both of which live in `State.Tank.heal`.
function Economy.tickIncome(s)
	local economy = s.economy
	local amount = Fixed.toInt(Fixed.scaleI64(economy.income_mult, economy.income_per_tick))
	State.awardGold(s, amount)
	-- Income-as-HP-regen (the source's "% of Gold Income as instant HP Regen").
	if Fixed.lt(ZERO, economy.income_regen_pct) and not s.dead then
		local raw = Fixed.toInt(Fixed.scaleI64(economy.income_regen_pct, amount))
		local cap = Economy.incomeRegenTickCap(s.tank.max_hp, s.tick)
		tankHeal(s.tank, math.min(raw, cap))
	end
	-- Income-as-survival: route a fraction of the award into the Mana-Shield pool
	-- (capped at its max), mirroring the HP-regen path above.
	if Fixed.lt(ZERO, economy.income_shield_pct) and not s.dead then
		local addend = Fixed.toInt(Fixed.scaleI64(economy.income_shield_pct, amount))
		if addend > 0 and s.tank.mana_shield < s.tank.mana_shield_max then
			s.tank.mana_shield = math.min(s.tank.mana_shield + addend, s.tank.mana_shield_max)
		end
	end
end

--- Phase 9: resolve a fatal hit. A pending revive (Ankh) is consumed first —
--- fully repairing the tank and granting `revive_bonus_hp` Max HP — so the tank
--- survives. Only when no revive remains does it die.
function Economy.resolveDeaths(s)
	if s.tank.hp <= 0 and not s.dead then
		if s.tank.revives > 0 then
			s.tank.revives -= 1
			s.tank.max_hp += s.tank.revive_bonus_hp
			s.tank.hp = s.tank.max_hp -- full repair
			return
		end
		s.dead = true
		s.death_tick = s.tick
	end
end

return Economy
