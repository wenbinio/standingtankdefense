--[[
	Defense.luau — transcription of `sim-core/crates/sim/src/defense.rs`
	(`roblox/SIM-SPEC.md` §S1/§S2). This is a TRANSCRIPTION, not a redesign: the
	Rust is the specification and a behavioral difference is a bug.

	Tank defensive layer (`docs/05 §2.2`): Dodge → Armor → Mana Shield → HP, plus
	per-tick Mana-Shield and HP regeneration. Deterministic — the only RNG is the
	dodge roll from `rng_proc`.

	TICK PHASES (SIM-SPEC §S5): `Defense.spikes` is phase 11, `shieldBreakStun`
	phase 12, `regen` phase 19. `hitTank` is not a phase — combat calls it at
	every incoming-damage site (phases 9/10 and the hazard/boss paths).

	TRANSIENT FLAGS. `spikes` MUST run before `shieldBreakStun`: `spikes`
	consumes `tank_hit_this_tick`, which `hitTank` sets. Both flags are set by
	`hitTank` during combat and cleared within the same tick (here), so both are
	`false` at every tick boundary — which is why they are excluded from the
	checksum. Moving either set/clear site breaks the R3 checksum gate.

	No Roblox global is referenced; this module runs under a bare `luau`.
]]

local Fixed = require("../Fixed")
local Rng = require("../Rng")
local Content = require("../Content")
local State = require("./State")
local Status = require("./Status")

-- ---------------------------------------------------------------------------
-- CROSS-MODULE SEAM (SIM-SPEC §S2). These are the ONLY foreign symbols this
-- module touches; each line names the Rust item it stands for, so an interface
-- drift is a one-line fix here rather than a scavenger hunt through the body.
-- ---------------------------------------------------------------------------
local distSq = State.Vec2.distSq -- `Vec2::dist_sq(self, o) -> Fixed`
local tankHeal = State.Tank.heal -- `Tank::heal(&mut self, amount: i64)`
local recordPlayerDamage = State.recordPlayerDamage -- `ArenaState::record_player_damage`

-- Content ids are 0-based (array position IS identity, §C1/§S3); Luau arrays
-- are 1-indexed. This accessor is the ONLY place that offset lives.
local function enemyDef(def: number)
	return Content.ENEMIES[def + 1]
end

local Defense = {}

--- Radius (from the tank) of Spikes retaliation.
local SPIKES_RANGE = 400

--- Ticks per in-game second (30 Hz) — cadence of the Missing-HP pulse.
local TICKS_PER_SECOND = 30

--- Apply `raw` incoming damage to the tank through the defensive layers.
function Defense.hitTank(s, raw)
	if raw <= 0 then
		return
	end
	local tank = s.tank
	-- Dodge: chance to avoid the hit entirely (consumes a proc roll).
	-- NOTE (SIM-SPEC §S4): the draw is CONDITIONAL on `dodge_den > 0`, and the
	-- roll happens BEFORE the `< dodge_num` test, so `rng_proc` advances on every
	-- landed hit as well as every dodged one. Changing either condition
	-- desynchronizes the proc stream.
	if tank.dodge_den > 0 then
		local roll = Rng.below(s.rng_proc, tank.dodge_den)
		if roll < tank.dodge_num then
			return
		end
	end
	-- The hit landed (even if fully absorbed by the shield) → Spikes will fire.
	s.tank_hit_this_tick = true
	-- Heal-on-damaged (Dreadlord Fang): one flat heal per LANDED hit, regardless
	-- of how much is ultimately absorbed. Routes through `Tank::heal` so the
	-- `healing_mult` and the max-HP cap apply uniformly.
	if tank.heal_on_damaged > 0 then
		tankHeal(tank, tank.heal_on_damaged)
	end
	-- Armor: flat reduction, but at least 1 damage always lands.
	local remaining = raw - tank.armor
	if remaining < 1 then
		remaining = 1
	end
	-- Mana Shield absorbs before HP.
	if tank.mana_shield > 0 then
		-- Energy Shield: while the shield is active (pool > 0 at hit time), ALL
		-- incoming damage — what the shield absorbs AND what overflows to HP — is
		-- reduced by `shield_active_dr`. The DR is clamped to `[0, ONE]` so the
		-- `(1 - dr)` multiplier can never go negative; the min-1 guarantee is kept
		-- so a hit is never reduced fully to zero. Once the shield hits 0 the
		-- reduction stops (this whole branch is gated on `mana_shield > 0`).
		-- Integer/Fixed only — feeds the checksum.
		local dr = tank.shield_active_dr
		if Fixed.lt(dr, Fixed.ZERO) then
			dr = Fixed.ZERO
		elseif Fixed.lt(Fixed.ONE, dr) then
			dr = Fixed.ONE
		end
		remaining = Fixed.toInt(Fixed.scaleI64(Fixed.sub(Fixed.ONE, dr), remaining))
		if remaining < 1 then
			remaining = 1
		end
		local absorbed = math.min(remaining, tank.mana_shield)
		tank.mana_shield -= absorbed
		remaining -= absorbed
		-- Shield-break stun (source: Energy Pulse): detect the `>0 → 0` DOWN-EDGE
		-- here, at the moment the shield absorbs the hit that depletes it. The pulse
		-- itself (iterate enemies in range, stun them in stable id order) is deferred
		-- to `shieldBreakStun` so this borrow-only path stays free of the enemy
		-- loop. Armed only when the pulse is configured (`range > 0`).
		if tank.mana_shield == 0 and tank.shieldbreak_stun_range > 0 then
			s.shield_broke_this_tick = true
		end
	end
	tank.hp -= remaining
end

--- Shield-break stun pulse (source: Energy Pulse). If the Mana Shield transitioned
--- `>0 → 0` from a hit this tick, stun every non-boss enemy within
--- `shieldbreak_stun_range` for `shieldbreak_stun_ticks`, then consume the flag.
--- Iterates enemies in STABLE id order (the live `enemies` vec is id-ordered) so
--- the result is run-to-run identical. Integer/Fixed only; no RNG. Reuses the
--- existing stun status (`stun_ticks`, takes the longer remaining).
--- Tick phase 12 (SIM-SPEC §S5) — after `spikes`, which consumes the hit flag.
function Defense.shieldBreakStun(s)
	local broke = s.shield_broke_this_tick
	s.shield_broke_this_tick = false
	if not broke then
		return
	end
	local ticks = s.tank.shieldbreak_stun_ticks
	if ticks == 0 then
		return
	end
	local range = Fixed.fromInt(s.tank.shieldbreak_stun_range)
	local r2 = Fixed.mul(range, range)
	local tankPos = s.tank.pos
	-- `s.enemies` is maintained in id order by every spawn/reap path, so this loop
	-- is already stable-id-ordered; stun is order-independent regardless.
	local enemies = s.enemies
	for i = 1, #enemies do
		local e = enemies[i]
		if not enemyDef(e.def).boss then
			if Fixed.le(distSq(tankPos, e.pos), r2) then
				e.status.stun_ticks = math.max(e.status.stun_ticks, ticks)
			end
		end
	end
end

--- Spikes retaliation: if the tank was hit this tick, deal its (multiplied)
--- spikes damage to every non-boss enemy within `SPIKES_RANGE`. Consumes and
--- resets the hit flag. Kills push to `pending_kills` (so they award bounty).
--- Tick phase 11 (SIM-SPEC §S5).
function Defense.spikes(s)
	local hit = s.tank_hit_this_tick
	s.tank_hit_this_tick = false
	if not hit then
		return
	end
	local tank = s.tank
	-- Stacking spikes (source: Bloody Spikes): each landed hit grows the stack
	-- counter by 1 up to the cap; the bonus spikes damage is `per × stacks`. This
	-- grows BEFORE the retaliation so the hit that triggered the stack benefits from
	-- it (the source applies the new stack immediately). Integer only; resets at the
	-- round boundary (see `economy`/round phase). A no-op when `spikes_stack_per == 0`.
	local stackBonus = 0
	if tank.spikes_stack_per > 0 then
		if tank.spikes_stacks < tank.spikes_stacks_max then
			tank.spikes_stacks += 1
		end
		stackBonus = tank.spikes_stack_per * tank.spikes_stacks
	end
	-- Spikes damage = (flat + stacking bonus) × multiplier.
	local base = tank.spikes_damage + stackBonus
	local dmg = Fixed.toInt(Fixed.scaleI64(tank.spikes_mult, base))
	-- NOTE: the stack counter has ALREADY grown above when this bails out — the
	-- Rust orders it the same way.
	if dmg <= 0 then
		return
	end
	local range = Fixed.fromInt(SPIKES_RANGE)
	local r2 = Fixed.mul(range, range)
	local tankPos = tank.pos
	-- Spikes-applied Poison DoT (source: Poison Armor) — applied to every enemy the
	-- retaliation lands on, reusing the existing poison status (stronger-DoT rule).
	local poison = nil
	if tank.spikes_poison_dps > 0 and tank.spikes_poison_ticks > 0 then
		-- `content::StatusOnHit { poison_dps, poison_ticks, ..StatusOnHit::NONE }`
		poison = {
			poison_dps = tank.spikes_poison_dps,
			poison_ticks = tank.spikes_poison_ticks,
			frost_stacks = 0,
			fire_stacks = 0,
			stun_ticks = 0,
		}
	end
	local survivors = {}
	local spikesDealt = 0
	local pendingKills = s.pending_kills
	local enemies = s.enemies
	-- `s.enemies` is id-ordered, so this retaliation pass is stable across runs.
	for i = 1, #enemies do
		local e = enemies[i]
		local boss = enemyDef(e.def).boss
		if not boss and Fixed.le(distSq(tankPos, e.pos), r2) then
			e.hp -= dmg
			spikesDealt += dmg
			if poison ~= nil then
				Status.applyOnHit(e, poison)
			end
		end
		-- NOTE: spikes kills are reaped HERE, not through `Status.reapDead`, so a
		-- Fire-stacked enemy killed by spikes does NOT detonate. That asymmetry is
		-- the Rust's, and is transcribed as-is.
		if e.hp <= 0 then
			pendingKills[#pendingKills + 1] = e.def
		else
			survivors[#survivors + 1] = e
		end
	end
	s.enemies = survivors
	-- Spikes retaliation counts as player damage (scoreboard + Bloodmoney).
	recordPlayerDamage(s, spikesDealt)
end

--- Per-tick regeneration of the Mana Shield and HP (each capped at its max),
--- plus the once-per-second Missing-HP heal. HP healing routes through
--- `Tank::heal` so the "+% Healing" multiplier and the cap apply uniformly.
--- Tick phase 19 (SIM-SPEC §S5).
function Defense.regen(s)
	local tank = s.tank
	if tank.mana_regen_per_tick > 0 and tank.mana_shield < tank.mana_shield_max then
		tank.mana_shield = math.min(tank.mana_shield + tank.mana_regen_per_tick, tank.mana_shield_max)
	end
	if tank.hp_regen_per_tick > 0 then
		tankHeal(tank, tank.hp_regen_per_tick)
	elseif tank.hp_regen_per_tick < 0 then
		-- Negative regen (from a regen→gold trade) is a drain: subtract its
		-- magnitude from HP each tick. It bypasses `Tank::heal`/`healing_mult`
		-- and CAN bring the tank to death via the normal death path.
		tank.hp += tank.hp_regen_per_tick
	end
	-- Missing-HP heal: once per second, restore a fraction of the HP deficit.
	if Fixed.lt(Fixed.ZERO, tank.missing_hp_heal_pct) and s.tick % TICKS_PER_SECOND == 0 then
		local missing = tank.max_hp - tank.hp
		if missing > 0 then
			local amount = Fixed.toInt(Fixed.scaleI64(tank.missing_hp_heal_pct, missing))
			tankHeal(tank, amount)
		end
	end
end

return Defense
