--[[
	Status.luau — transcription of `sim-core/crates/sim/src/status.rs`
	(`roblox/SIM-SPEC.md` §S1/§S2). This is a TRANSCRIPTION, not a redesign: the
	Rust is the specification and a behavioral difference is a bug.

	Status-effect system (`docs/05 §2.5`): Poison (DoT), Frost (slow), Fire
	(damage vulnerability), and Stun (immobilize). Deterministic — integer/Fixed
	only, stable iteration order, no RNG. Applied on hit by combat, ticked once
	per sim tick by `tick`, and consulted by combat for damage/movement.

	TICK PHASES (SIM-SPEC §S5): `Status.pulse` is phase 6, `Status.tick` is
	phase 16. The other exports are helpers called from combat/defense at their
	own phases.

	No Roblox global is referenced; this module runs under a bare `luau`.
]]

local Fixed = require("../Fixed")
local Content = require("../Content")
local State = require("./State")

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

-- INTEGRATION NOTE: SIM-SPEC §S2 does not pin how `Content.luau` spells its
-- scalar constants (`State.luau` reads `Content.constants.*`, `Combat.luau`
-- reads top-level names). This is a load-time name lookup only — it cannot
-- change behavior, and it fails loudly rather than silently reading `nil`.
-- Collapse it once the spelling is pinned. `content::FROST_MAX_STACKS` == 25.
local FROST_MAX_STACKS = Content.FROST_MAX_STACKS
	or (Content.constants ~= nil and Content.constants.frost_max_stacks or nil)
if FROST_MAX_STACKS == nil then
	error("Status: Content is missing FROST_MAX_STACKS / constants.frost_max_stacks")
end

local Status = {}

--- Frost duration applied per application (ticks). Stacks clear on expiry.
local FROST_DURATION = 150 -- 5 s @ 30 Hz

--- Freeze duration when an enemy reaches `FROST_MAX_STACKS` (the source's Deep
--- Freeze: "1.5 seconds of Freeze when an enemy reaches 25 stacks of Frost,
--- resetting stacks to 0", `docs/appendix-A §A.2`). 1.5 s @ 30 Hz.
local FREEZE_DURATION = 45

--- Radius of a Fire death-explosion. The source says "surrounding enemies"
--- without a fixed number; we use the catalog's common splash radius (300,
--- `docs/appendix-A §A.2`).
local FIRE_EXPLOSION_RADIUS = 300

--- Fire death-explosion damage: "1 damage per 5 Stacks" (`docs/appendix-A
--- §A.2`) ⇒ `floor(fire_stacks / 5)`.
local FIRE_STACKS_PER_DAMAGE = 5

-- Integer-width ceilings for the saturating adds below. `EnemyStatus` types
-- `frost_stacks` as u8, `fire_stacks`/`vuln_stacks` as u16; Rust's
-- `saturating_add` clamps at those widths, so the port must too. This is not
-- cosmetic for `fire_stacks`: a long match really can pile past 65535.
local U8_MAX = 255
local U16_MAX = 65535

-- Rust's `i64::MIN`, used by `reap_dead` as the "already reaped this pass"
-- marker. Exactly representable as a double (-2^63).
local I64_MIN = -9223372036854775808

-- Pure constants hoisted out of the per-call bodies (the Rust builds them
-- inline; they are functions of literals only, so this is not a behavior
-- change).
local FROST_SLOW_FLOOR = Fixed.fromRatio(1, 10) -- "floor at 10%"
local FREEZE_VULN_BONUS = Fixed.fromRatio(1, 2) -- +50% damage taken while frozen

--- Apply a weapon's on-hit status to an enemy. Poison refreshes to the stronger
--- DoT; frost/fire add stacks (frost capped); stun takes the longer remaining.
function Status.applyOnHit(enemy, onHit)
	local st = enemy.status
	if onHit.poison_dps > 0 and onHit.poison_ticks > 0 then
		-- Keep whichever poison deals more total remaining damage.
		local existing = st.poison_dps * st.poison_ticks
		local incoming = onHit.poison_dps * onHit.poison_ticks
		if incoming >= existing then
			st.poison_dps = onHit.poison_dps
			st.poison_ticks = onHit.poison_ticks
		end
	end
	if onHit.frost_stacks > 0 then
		st.frost_stacks = math.min(st.frost_stacks + onHit.frost_stacks, U8_MAX)
		st.frost_ticks = FROST_DURATION
		-- Freeze payoff: reaching the cap immobilizes the enemy for a short
		-- window and resets the stacks (`docs/appendix-A §A.2`). While frozen it
		-- takes +50% damage (see `vulnerabilityMult`).
		if st.frost_stacks >= FROST_MAX_STACKS then
			st.frost_stacks = 0
			st.frost_ticks = 0
			st.freeze_ticks = math.max(st.freeze_ticks, FREEZE_DURATION)
		else
			st.frost_stacks = math.min(st.frost_stacks, FROST_MAX_STACKS)
		end
	end
	if onHit.fire_stacks > 0 then
		st.fire_stacks = math.min(st.fire_stacks + onHit.fire_stacks, U16_MAX)
	end
	if onHit.stun_ticks > 0 then
		st.stun_ticks = math.max(st.stun_ticks, onHit.stun_ticks)
	end
end

--- Damage-taken multiplier from status: Fire (+0.5% per stack), generic
--- Vulnerability stacks (+1% per stack, from Vulnerability-Pulse auras), and the
--- Freeze payoff (+50% while frozen, `docs/appendix-A §A.2`).
function Status.vulnerabilityMult(enemy)
	local st = enemy.status
	-- Left-associative, exactly as the Rust `ONE + a + b` folds: each `add` is
	-- its own saturating step.
	local m = Fixed.add(
		Fixed.add(Fixed.ONE, Fixed.fromRatio(st.fire_stacks * 5, 1000)),
		Fixed.fromRatio(st.vuln_stacks, 100)
	)
	if st.freeze_ticks > 0 then
		m = Fixed.add(m, FREEZE_VULN_BONUS) -- +50% damage taken while frozen
	end
	return m
end

--- Phase: Vulnerability-Pulse auras. For each active pulse whose interval has
--- elapsed, add its magnitude in vuln stacks to every enemy within range.
--- Deterministic (fixed ticks, append-only pulses, stable enemy order).
--- Tick phase 6 (SIM-SPEC §S5).
function Status.pulse(s)
	local pulses = s.vuln_pulses
	if #pulses == 0 then
		return
	end
	local tankPos = s.tank.pos
	-- (The Rust `mem::take`s the pulse vec purely to satisfy the borrow checker
	-- while `s.enemies` is mutated; nothing here appends pulses, so iterating in
	-- place is the same computation.)
	local enemies = s.enemies
	for pi = 1, #pulses do
		local p = pulses[pi]
		local range = Fixed.fromInt(p.range)
		local r2 = Fixed.mul(range, range)
		while s.tick >= p.next_tick do
			for i = 1, #enemies do
				local e = enemies[i]
				if Fixed.le(distSq(tankPos, e.pos), r2) then
					e.status.vuln_stacks = math.min(e.status.vuln_stacks + p.magnitude, U16_MAX)
				end
			end
			p.next_tick += p.interval_ticks
		end
	end
end

--- Movement-speed multiplier from status (Frost slow: -2% per stack, floored).
function Status.moveSpeedMult(enemy)
	local slow = Fixed.fromRatio(enemy.status.frost_stacks * 2, 100)
	local m = Fixed.sub(Fixed.ONE, slow)
	-- Floor at 10% so a fully-frosted enemy still crawls.
	if Fixed.lt(m, FROST_SLOW_FLOOR) then
		return FROST_SLOW_FLOOR
	end
	return m
end

--- Whether the enemy cannot move this tick (stunned or frozen).
function Status.isImmobile(enemy)
	return enemy.status.stun_ticks > 0 or enemy.status.freeze_ticks > 0
end

--- Reap every enemy at `hp <= 0`, pushing its `def` to `pending_kills` and
--- triggering its Fire death-explosion. A Fire-stacked enemy that dies deals
--- `floor(fire_stacks / 5)` damage to surrounding enemies within
--- `FIRE_EXPLOSION_RADIUS` (`docs/appendix-A §A.2`); that damage can chain
--- further deaths, which detonate in turn. Fully deterministic: dead enemies are
--- processed in id-sorted order, and the loop fixes a stable point. The shared
--- death-reaping chokepoint for every combat/status death site so Fire
--- explosions fire wherever an enemy dies. Bosses neither explode nor take
--- explosion damage (they are damaged only by `Clear`).
function Status.reapDead(s)
	local radius = Fixed.fromInt(FIRE_EXPLOSION_RADIUS)
	local radiusSq = Fixed.mul(radius, radius)
	while true do
		local enemies = s.enemies
		-- Collect dead enemies (id-sorted) so explosions resolve deterministically.
		local dead = {}
		for i = 1, #enemies do
			if enemies[i].hp <= 0 then
				dead[#dead + 1] = i
			end
		end
		if #dead == 0 then
			return
		end
		-- Entity ids are unique (`alloc_entity_id` never reuses), so ordering by
		-- id alone is a total order and `table.sort`'s instability is unreachable
		-- — matching Rust's stable `sort_by_key(|&i| s.enemies[i].id.0)`.
		table.sort(dead, function(a, b)
			return enemies[a].id < enemies[b].id
		end)

		-- For each dead enemy: record the kill and, if it carried Fire, splash
		-- explosion damage onto living non-boss enemies in range.
		local explosionDamage = 0
		local pendingKills = s.pending_kills
		for k = 1, #dead do
			local di = dead[k]
			local e = enemies[di]
			local def, fireStacks, pos = e.def, e.status.fire_stacks, e.pos
			pendingKills[#pendingKills + 1] = def
			-- Mark reaped so it is not collected again next round.
			e.hp = I64_MIN
			if fireStacks ~= 0 and not enemyDef(def).boss then
				local dmg = math.floor(fireStacks / FIRE_STACKS_PER_DAMAGE)
				if dmg > 0 then
					for i = 1, #enemies do
						local o = enemies[i]
						if i ~= di and o.hp > 0 and not enemyDef(o.def).boss then
							if Fixed.le(distSq(pos, o.pos), radiusSq) then
								o.hp -= dmg
								explosionDamage += dmg
							end
						end
					end
				end
			end
		end
		-- Remove the enemies reaped this pass; survivors keep id order.
		local kept = {}
		for i = 1, #enemies do
			local e = enemies[i]
			if e.hp ~= I64_MIN then
				kept[#kept + 1] = e
			end
		end
		s.enemies = kept
		-- Explosion damage is a player source (scoreboard / Bloodmoney).
		recordPlayerDamage(s, explosionDamage)
		-- Loop: chained deaths from this pass's explosions detonate next pass.
	end
end

--- Per-tick status processing: apply Poison DoT, decay timers, clear expired
--- Frost. Poison kills push their `def` to `pending_kills` (so they still award
--- bounty). Boss enemies are immune to Poison (only `Clear` hurts the boss).
--- Tick phase 16 (SIM-SPEC §S5).
function Status.tick(s)
	local poisonHits = 0
	local poisonDamage = 0
	-- (The Rust rebuilds `s.enemies` into a `survivors` vec only to move each
	-- `Enemy` out of the borrow; nothing is filtered here, so mutating in place
	-- is the same computation in the same order.)
	local enemies = s.enemies
	for i = 1, #enemies do
		local e = enemies[i]
		local st = e.status
		local immune = enemyDef(e.def).boss

		-- Poison damage-over-time.
		if st.poison_ticks > 0 then
			if not immune then
				e.hp -= st.poison_dps
				poisonHits += 1
				poisonDamage += st.poison_dps
			end
			st.poison_ticks -= 1
			if st.poison_ticks == 0 then
				st.poison_dps = 0
			end
		end
		-- Frost duration / expiry.
		if st.frost_ticks > 0 then
			st.frost_ticks -= 1
			if st.frost_ticks == 0 then
				st.frost_stacks = 0
			end
		end
		-- Stun duration.
		if st.stun_ticks > 0 then
			st.stun_ticks -= 1
		end
		-- Freeze duration (clears cleanly; no residual effect).
		if st.freeze_ticks > 0 then
			st.freeze_ticks -= 1
		end
	end
	-- Reap poison kills (and their Fire death-explosions) in a stable pass.
	Status.reapDead(s)
	-- Poison DoT counts toward the player's damage scoreboard / Bloodmoney.
	recordPlayerDamage(s, poisonDamage)
	-- On-poison trigger: heal the tank per enemy that took poison this tick.
	if s.tank.heal_on_poison > 0 and poisonHits > 0 and not s.dead then
		tankHeal(s.tank, poisonHits * s.tank.heal_on_poison)
	end
end

return Status
