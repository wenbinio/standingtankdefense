--[[
	Waves.luau — transcription of `sim-core/crates/sim/src/waves.rs`.

	Wave spawning. Deterministic; randomness only from `s.rng_spawn`
	(`SIM-SPEC.md` §S4 — stream discipline is sacred).

	RNG DRAW SEQUENCE (`rng_spawn`), per call to `Waves.spawn`:
	  * the boss spawn itself draws NOTHING (it uses `SPAWN_RING[0]` directly);
	  * in the boss phase, one `Rng.below(rng_spawn, #SPAWN_RING)` per BOSS_ESCORT
	    entry that fires this tick, in catalog order;
	  * otherwise one `Rng.below(rng_spawn, #SPAWN_RING)` per WAVE_M0 entry that
	    fires this tick, in catalog order.
	`Rng.below` is a rejection loop: it consumes a VARIABLE number of u32 words.
	That is reproduced exactly by `Rng.luau`; never substitute an equivalent.
	`rng_spawn` is drawn from HERE AND NOWHERE ELSE, matching the Rust.

	TIMELINE (`docs/10` F1, `CONTRACTS.md` C1, `PHASE2-CONTRACTS.md` P3).
	`content.json` ships two timelines. This module hardcodes NEITHER: it reads
	`boss_spawn_tick`, `ramp_interval`, `gentle_ticks`, `ramp_intervals_to_boss`
	and `tick_scale_num/den` from a timeline table resolved, in order, from
	    `state.timeline`  ->  `Waves.timeline`  ->  `Content.TIMELINE.steam`.
	The DEFAULT is therefore **steam**, because the R3 oracle traces are
	generated from the Rust, which runs the Steam timeline. Switching the
	default would fail the gate for a reason that is not a bug.

	`state.timeline` is the SINGLE SOURCE OF TRUTH (P3) and the only form that is
	safe with per-player sharded arenas (`docs/03`): `Waves.timeline` is a
	process-wide override and would apply to every arena in the server at once.
	Use it in tests, not in the arena runner.

	The wave/escort `start_tick` gates are CONTENT: they ship pre-baked at STEAM
	scale so one catalog serves both timelines, and `gateTick` below rescales
	them onto the active timeline by `tick_scale_num/den` (identity on Steam).
	Without that rescale a Roblox-timeline run (boss at 9000) would leave 7 of
	the 17 roster gates permanently shut.

	NOT rescaled, deliberately: `cadence_ticks`. F1 is a rescale of the run's
	*schedule*, not of its spawn rate — compressing cadences too would cut the
	total enemy count and turn the rescale into a rebalance.
]]

local Fixed = require("../Fixed")
local Rng = require("../Rng")

local Waves = {}

-- ============================ content access ============================
-- INTEGRATION NOTE: `Content.luau` is authored concurrently and neither its
-- export names nor its INDEX BASE are pinned by SIM-SPEC §S2 (flagged, not
-- widened). Both are resolved once, at content-binding time:
--   * names — a fixed candidate list, erroring if none matches;
--   * index base — detected per catalog (`t[0] ~= nil` ⇒ 0-based) and
--     cross-checked against `constants.num_*`.
-- The index base is NOT cosmetic: `Rng.below(rng, #WEAPONS)` on a 0-based table
-- would draw over 85 instead of 86 and silently desynchronize `rng_shop`
-- forever. Getting it wrong loudly is the whole point of the cross-check.

local contentModule = nil
local view = nil

--- Inject the content module (tests, or an explicit wiring from `Arena`).
function Waves.setContent(c)
	contentModule = c
	view = nil
end

local function pick(t, names, what)
	for _, n in names do
		local v = t[n]
		if v ~= nil then
			return v
		end
	end
	error("Waves: Content is missing " .. what .. " (tried: " .. table.concat(names, ", ") .. ")", 3)
end

--- Bind a catalog indexed by 0-based content ids (`SIM-SPEC.md` §S3) to a
--- uniform accessor, whichever index base the table actually uses.
local function bindCatalog(t, expected, what)
	local zeroBased = t[0] ~= nil
	local n = if zeroBased then #t + 1 else #t
	if expected ~= nil and n ~= expected then
		error(
			string.format("Waves: %s has %d entries, content says %d", what, n, expected),
			3
		)
	end
	local base = if zeroBased then 0 else 1
	return {
		n = n,
		--- `id` is a 0-based content id.
		get = function(id)
			return t[id + base]
		end,
	}
end

--- A `Fixed` value from either an already-`Fixed` value or a raw i64 number.
local function asFixed(v)
	if type(v) == "number" then
		return Fixed.fromRaw(v)
	end
	return v
end

--- A table's entries in content order, whatever its index base.
local function ordered(t)
	if t[0] == nil then
		return t
	end
	local n = #t + 1
	local out = table.create(n)
	for i = 1, n do
		out[i] = t[i - 1]
	end
	return out
end

local function content()
	if view ~= nil then
		return view
	end
	if contentModule == nil then
		local ok, mod = pcall(require, "../Content")
		if not ok then
			error(
				"Waves: no content — require('../Content') failed ("
					.. tostring(mod)
					.. "); call Waves.setContent(Content) first",
				3
			)
		end
		contentModule = mod
	end
	local c = contentModule
	local constants = pick(c, { "CONSTANTS", "constants" }, "the constants table")
	local ring = ordered(pick(c, { "SPAWN_RING", "spawnRing", "spawn_ring" }, "the spawn ring"))
	-- Normalize the ring ONCE into Vec2-of-Fixed. Enemies get a fresh copy per
	-- spawn so nothing downstream can mutate the shared ring in place.
	local ringF = table.create(#ring)
	for i = 1, #ring do
		ringF[i] = { x = asFixed(ring[i].x), y = asFixed(ring[i].y) }
	end
	local boss = c.BOSS
	if boss == nil then
		boss = constants.boss
	end
	if boss == nil then
		error("Waves: Content is missing the boss enemy id (BOSS / constants.boss)", 3)
	end
	view = {
		enemies = bindCatalog(
			pick(c, { "ENEMIES", "enemies" }, "the enemy catalog"),
			constants.num_enemies,
			"the enemy catalog"
		),
		waves = ordered(pick(c, { "WAVE_M0", "WAVES", "waves" }, "the wave table")),
		escort = ordered(pick(c, { "BOSS_ESCORT", "bossEscort", "boss_escort" }, "the boss escort table")),
		ring = ringF,
		boss = boss,
		constants = constants,
		timeline = pick(c, { "TIMELINE", "timeline" }, "the timeline table"),
		-- `content.rs::enemy_hp_mult`; owned by `Content.luau` (SIM-SPEC §S2).
		enemyHpMult = pick(c, { "enemyHpMult", "enemy_hp_mult" }, "enemyHpMult"),
	}
	return view
end

-- ============================ timeline ============================

--- PROCESS-WIDE timeline override (nil ⇒ `Content.TIMELINE.steam`). Applies to
--- every arena that has no `state.timeline` of its own, so it is a TEST hook —
--- the arena runner sets `state.timeline` per shard instead (P3).
Waves.timeline = nil

--- Resolve a timeline argument to a timeline table: a table passes through, a
--- string names one of the two `content.json` ships (`"steam"` / `"roblox"`),
--- and `nil` stays `nil` (meaning "no override" / the Steam default).
---
--- Validates that the result actually looks like a timeline, because the failure
--- mode of a wrong table here is silent: a missing `boss_spawn_tick` would make
--- `s.tick == bossTick` never true and the boss would simply never arrive.
function Waves.resolveTimeline(tl)
	if tl == nil then
		return nil
	end
	if type(tl) == "string" then
		local t = content().timeline[tl]
		if t == nil then
			error("Waves: unknown timeline '" .. tl .. "'", 3)
		end
		return t
	end
	if type(tl) ~= "table" or tl.boss_spawn_tick == nil or tl.round_ticks == nil then
		error("Waves: not a timeline (want a table with boss_spawn_tick/round_ticks, or \"steam\"/\"roblox\")", 3)
	end
	return tl
end

--- Set the process-wide timeline: a table, or `"steam"` / `"roblox"`. `nil`
--- restores the default (steam). Prefer `state.timeline` — see above.
function Waves.setTimeline(tl)
	Waves.timeline = Waves.resolveTimeline(tl)
end

--- Rescale a content `start_tick` gate onto the active timeline.
---
--- The wave/escort `start_tick` gates ship at STEAM scale (up to 45000) and are
--- deliberately not pre-rescaled in `content.json`, so the one catalog serves
--- both timelines. Without this, a Roblox-timeline run (boss at 9000) would
--- leave 7 of 17 roster gates permanently shut — the roster would silently
--- collapse to its opening mix with no error anywhere.
---
--- `tick_scale_num/den` is 1/1 on Steam, so this is the identity there and the
--- Rust oracle is unaffected. Every shipped gate divides evenly by 6, so the
--- Roblox rescale is exact rather than merely floored.
local function gateTick(tl, startTick: number): number
	local num, den = tl.tick_scale_num, tl.tick_scale_den
	if num == nil or den == nil or den == 0 or num == den then
		return startTick
	end
	return math.floor(startTick * num / den)
end

--- The timeline in force for `s`: per-arena override, then module override,
--- then the Steam default (which is what the Rust — and the oracle — runs).
local function timelineFor(s)
	if s ~= nil and s.timeline ~= nil then
		return s.timeline
	end
	if Waves.timeline ~= nil then
		return Waves.timeline
	end
	local steam = content().timeline.steam
	if steam == nil then
		error("Waves: Content.TIMELINE has no 'steam' entry", 3)
	end
	return steam
end

--- The timeline in force for `s` (public form of the resolver above): the
--- per-arena `state.timeline`, else the module override, else Steam. `s` may be
--- `nil` to ask for "the default timeline". This is the ONE resolver — `Arena`
--- and anything else needing a timeline constant calls through here rather than
--- re-implementing the precedence.
function Waves.timelineFor(s)
	return timelineFor(s)
end

--- Rescale a content `start_tick` gate onto `tl` (identity on Steam). Public so
--- tooling/tests can check the gate schedule without re-deriving the rule.
function Waves.gateTick(tl, startTick: number): number
	return gateTick(tl, startTick)
end

-- ============================ enemy_hp_mult ============================

--- `content::enemy_hp_mult` — the stepped HP/damage ramp. In the Rust this
--- lives in `content.rs`, and `Content.luau` owns it here (SIM-SPEC §S2); this
--- is a FORWARDER, deliberately not a second copy, because `combat.rs` reads
--- the same curve and two transcriptions of it could drift apart invisibly.
--- The only thing added is the timeline resolution below: `Content.enemyHpMult`
--- defaults to Steam, and so does this, but a `state.timeline` / `Waves.timeline`
--- override is honoured here.
function Waves.enemyHpMult(tick, tl)
	local fn = content().enemyHpMult
	return fn(tick, tl or timelineFor(nil))
end

-- ============================ spawn ============================

--- `ArenaState::alloc_entity_id` — allocate a fresh, never-reused entity id.
--- Inlined (verbatim, two lines) rather than calling `State.allocEntityId` so
--- this module loads and unit-tests without `State.luau`. Swap it if the
--- integrator prefers one copy.
local function allocEntityId(s)
	local id = s.next_entity_id
	s.next_entity_id += 1
	return id
end

--- `Enemy::new` — construct an enemy with no status (the common case).
local function newEnemy(id, def, hp, pos)
	return {
		id = id,
		def = def,
		hp = hp,
		pos = { x = pos.x, y = pos.y },
		status = {
			poison_dps = 0,
			poison_ticks = 0,
			frost_stacks = 0,
			frost_ticks = 0,
			fire_stacks = 0,
			vuln_stacks = 0,
			stun_ticks = 0,
			freeze_ticks = 0,
		},
	}
end

--- Phase 5: for each wave-table entry, if `s.tick % cadence_ticks == 0` (and the
--- entry's `start_tick` gate has opened), spawn one enemy of that def at a
--- `SPAWN_RING` position chosen via `Rng.below(s.rng_spawn, #SPAWN_RING)`.
--- Ids come from `alloc_entity_id`; enemies are pushed in id order. HP is
--- `EnemyDef::base_hp` scaled by `enemy_hp_mult`.
function Waves.spawn(s)
	local c = content()
	local tl = timelineFor(s)
	local ring = c.ring
	local nring = #ring
	local enemies = c.enemies
	local bossTick = tl.boss_spawn_tick

	-- Boss phase (end of the run): spawn The Hippocrate exactly once at
	-- the boss tick. The boss uses its fixed HP (no scaling) and is immune to
	-- weapon fire — only `Clear` damages it (handled in combat). UNLIKE the old
	-- design, normal waves do NOT fully stop: a relentless ESCORT swarm keeps
	-- pouring in alongside the boss. The escort serves the climax three ways —
	--   1) contact-damage VOLUME that pressures even a heavily-defended tank,
	--   2) it forces the player to keep Clearing, and every Clear also chips the
	--      boss, so the boss is a real MULTI-CLEAR FIGHT rather than a stalemate,
	--   3) it makes the boss the wall most runs end at instead of a victory lap.
	if s.tick == bossTick then
		local edef = enemies.get(c.boss)
		local id = allocEntityId(s)
		-- NOTE: no `rng_spawn` draw here — the boss uses `SPAWN_RING[0]`.
		table.insert(s.enemies, newEnemy(id, c.boss, edef.base_hp, ring[1]))
		-- fall through: the escort swarm below also spawns on this tick.
	end
	if s.tick >= bossTick then
		-- Boss escort: a dense grunt/raider flood at the peak HP tier. Cadences
		-- are tight so the board stays full (keeps the player's Clear cycling onto
		-- the boss). Deterministic ring picks via `rng_spawn`, same as normal waves.
		local hpMult = Waves.enemyHpMult(s.tick, tl)
		local escort = c.escort
		for i = 1, #escort do
			local ws = escort[i]
			if ws.cadence_ticks ~= 0 and s.tick % ws.cadence_ticks == 0 then
				local ringIdx = Rng.below(s.rng_spawn, nring)
				local pos = ring[ringIdx + 1]
				local edef = enemies.get(ws.enemy)
				local hp = Fixed.toInt(Fixed.scaleI64(hpMult, edef.base_hp))
				local id = allocEntityId(s)
				table.insert(s.enemies, newEnemy(id, ws.enemy, hp, pos))
			end
		end
		return
	end

	-- Enemy HP scales with match time (identity until the first ramp).
	local hpMult = Waves.enemyHpMult(s.tick, tl)
	-- Process wave entries in their fixed catalog order so the rng_spawn draws
	-- happen in a deterministic sequence.
	local waves = c.waves
	for i = 1, #waves do
		local ws = waves[i]
		if ws.cadence_ticks ~= 0 then
			-- Escalation gate: this entry is dormant until its start_tick,
			-- rescaled onto the active timeline (identity on Steam).
			if s.tick >= gateTick(tl, ws.start_tick) then
				if s.tick % ws.cadence_ticks == 0 then
					local ringIdx = Rng.below(s.rng_spawn, nring)
					local pos = ring[ringIdx + 1]
					local edef = enemies.get(ws.enemy)
					local hp = Fixed.toInt(Fixed.scaleI64(hpMult, edef.base_hp))
					local id = allocEntityId(s)
					table.insert(s.enemies, newEnemy(id, ws.enemy, hp, pos))
				end
			end
		end
	end
end

return Waves
