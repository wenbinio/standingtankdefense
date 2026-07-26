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
--- If `income_regen_pct > 0`, also heal the tank that fraction of the award.
function Economy.tickIncome(s)
	local economy = s.economy
	local amount = Fixed.toInt(Fixed.scaleI64(economy.income_mult, economy.income_per_tick))
	State.awardGold(s, amount)
	-- Income-as-HP-regen (the source's "% of Gold Income as instant HP Regen").
	if Fixed.lt(ZERO, economy.income_regen_pct) and not s.dead then
		tankHeal(s.tank, Fixed.toInt(Fixed.scaleI64(economy.income_regen_pct, amount)))
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
