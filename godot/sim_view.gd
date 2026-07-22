# SimView — the typed, READ-ONLY view over the Rust sim bindings.
#
# Wraps the packed-array accessors of `StSim` (single-arena) and `StMatch`
# (per-player shadow arena in the net view) behind named methods, so no render
# code ever index-pokes `tank()[2]` / `economy()[0]` / `meta[i*3+1]` /
# `arena[7]` again. All defensive size() guards live HERE, once.
#
# Strictly read-only: stepping the sim (`sim.step(code, slot)` / `m.step()`)
# stays with the owning coordinator (main.gd / match.gd) — SimView can never
# write the sim, so nothing routed through it can touch determinism.
#
# Construct with the static helpers:
#   var view := SimView.of_sim(sim)        # StSim, single arena
#   var view := SimView.of_match(m, i)     # StMatch, player i's shadow arena
#
# ---------------------------------------------------------------------------
# EVENTS: `take_events()` drains the P1.1 sim→render event stream (see the
# EVENT RECORD LAYOUT table in godot/rust/src/lib.rs) and decodes the flat
# 6-int records into small typed Dictionaries ONCE. Read-and-clear: the owning
# coordinator (main.gd) drains exactly once per tick, right after step, and
# fans the same Array out to every consumer (juice, audio). Nobody re-queries.
# ---------------------------------------------------------------------------
class_name SimView
extends RefCounted

# Event kinds (mirror the EVENT RECORD LAYOUT in godot/rust/src/lib.rs).
const EV_ENEMY_KILLED := 1        # {x, y, enemy_kind, boss, bounty, fire_radius}
const EV_ENEMY_DESPAWNED := 2     # {id} — left WITHOUT dying (no death FX!)
const EV_IMPACT := 3              # {x, y, damage, damage_type, splash_radius}
const EV_PROJECTILE_SPAWNED := 4  # {weapon_kind, x, y, target_x, target_y}
const EV_TANK_HIT := 5            # {damage}
const EV_ROUND_START := 6         # {round}
const EV_BOSS_SPAWNED := 7        # {id}
const EV_HAZARD_PLACED := 8       # {x, y, radius, ticks, damage_type}
const EV_HAZARD_EXPIRED := 9      # {id}
const EV_FREEZE_PROC := 10        # {id}
const EV_SHIELD_BROKE := 11       # {}
const EV_GOLD_BOUNTY := 12        # {amount}

# Ints per flat event record (matches EVENT_RECORD_WIDTH in lib.rs).
const _EV_W := 6

# Exactly one of these is set. `_sim` is an StSim; `_match` an StMatch with
# `_pi` the player index whose shadow arena this view reads.
var _sim = null
var _match = null
var _pi := 0

static func of_sim(sim) -> SimView:
	var v := SimView.new()
	v._sim = sim
	return v

static func of_match(m, player_index: int) -> SimView:
	var v := SimView.new()
	v._match = m
	v._pi = player_index
	return v

# StMatch layout constants (see godot/rust/src/lib.rs doc comments).
# arena(i) = [x, y, hp, max_hp, revives, round, tick, dead]
const _ARENA_LEN := 8

func _arena() -> PackedInt64Array:
	return _match.arena(_pi) if _match != null else PackedInt64Array()

# True when this view has a readable arena behind it (a match player's shadow
# can be briefly empty before its first snapshot).
func is_valid() -> bool:
	if _sim != null:
		return true
	return _match != null and _arena().size() >= _ARENA_LEN

# --- events --------------------------------------------------------------------
# Drain this tick's sim→render events. READ-AND-CLEAR at the binding: call it
# exactly once per tick (main.gd, right after sim.step) and pass the returned
# Array to every consumer. Each entry is a small Dictionary with "kind" set to
# an EV_* constant plus the kind's named fields (see the constants above).
# Positions are integer world units in the same space as enemies_pos().
func take_events() -> Array:
	var raw: PackedInt64Array
	if _sim != null:
		raw = _sim.take_events()
	elif _match != null:
		raw = _match.take_events(_pi)
	else:
		return []
	var out: Array = []
	@warning_ignore("integer_division")
	var n := raw.size() / _EV_W
	for i in n:
		var o := i * _EV_W
		var kind := int(raw[o])
		match kind:
			EV_ENEMY_KILLED:
				# c packs enemy_kind + 65536*boss_flag.
				out.append({
					"kind": kind, "x": raw[o + 1], "y": raw[o + 2],
					"enemy_kind": int(raw[o + 3]) & 0xFFFF,
					"boss": (int(raw[o + 3]) >> 16) != 0,
					"bounty": raw[o + 4], "fire_radius": raw[o + 5],
				})
			EV_IMPACT:
				out.append({
					"kind": kind, "x": raw[o + 1], "y": raw[o + 2],
					"damage": raw[o + 3], "damage_type": int(raw[o + 4]),
					"splash_radius": raw[o + 5],
				})
			EV_PROJECTILE_SPAWNED:
				out.append({
					"kind": kind, "weapon_kind": int(raw[o + 1]),
					"x": raw[o + 2], "y": raw[o + 3],
					"target_x": raw[o + 4], "target_y": raw[o + 5],
				})
			EV_TANK_HIT:
				out.append({"kind": kind, "damage": raw[o + 1]})
			EV_ROUND_START:
				out.append({"kind": kind, "round": int(raw[o + 1])})
			EV_HAZARD_PLACED:
				out.append({
					"kind": kind, "x": raw[o + 1], "y": raw[o + 2],
					"radius": raw[o + 3], "ticks": int(raw[o + 4]),
					"damage_type": int(raw[o + 5]),
				})
			EV_GOLD_BOUNTY:
				out.append({"kind": kind, "amount": raw[o + 1]})
			EV_SHIELD_BROKE:
				out.append({"kind": kind})
			_:
				# EnemyDespawned / BossSpawned / HazardExpired / FreezeProc all
				# carry a single entity id in slot a.
				out.append({"kind": kind, "id": raw[o + 1]})
	return out

# --- match progress ----------------------------------------------------------
func tick() -> int:
	if _sim != null:
		return _sim.tick()
	var a := _arena()
	return a[6] if a.size() > 6 else 0

func round_num() -> int:
	if _sim != null:
		return _sim.round()
	var a := _arena()
	return a[5] if a.size() > 5 else 0

func is_dead() -> bool:
	if _sim != null:
		return _sim.is_dead()
	var a := _arena()
	return a.size() > 7 and a[7] != 0

# --- pacing / abilities (single-arena; StMatch exposes none of this, so a
# match-wrapped view returns inert defaults) --------------------------------------
# clear_state() = [ready_in_ticks, cooldown_total_ticks].
# Ticks until Clear is ready again; 0 = ready NOW.
func clear_ready_in() -> int:
	if _sim == null:
		return 0
	var cs: PackedInt64Array = _sim.clear_state()
	return cs[0] if cs.size() > 0 else 0

# Full Clear cooldown length in ticks (denominator for a cooldown fill; >= 1).
func clear_cooldown_total() -> int:
	if _sim == null:
		return 1
	var cs: PackedInt64Array = _sim.clear_state()
	return maxi(int(cs[1]), 1) if cs.size() > 1 else 1

# timing() = [ticks_to_next_round, round_len_ticks, boss_spawn_tick].
# Ticks until the next round boundary (= the next shop refresh), 1..=round_ticks.
func ticks_to_next_round() -> int:
	if _sim == null:
		return 0
	var t: PackedInt64Array = _sim.timing()
	return t[0] if t.size() > 0 else 0

# Round length in ticks (30 s @ 30 Hz).
func round_ticks() -> int:
	if _sim == null:
		return 900
	var t: PackedInt64Array = _sim.timing()
	return maxi(int(t[1]), 1) if t.size() > 1 else 900

# The fixed tick the boss enters the arena (0 when unavailable — match view).
func boss_spawn_tick() -> int:
	if _sim == null:
		return 0
	var t: PackedInt64Array = _sim.timing()
	return t[2] if t.size() > 2 else 0

# First live boss this tick as {kind, hp_permille}, or {} when no boss is up.
# Zips the parallel enemy arrays once, here, so the HUD never index-pokes them.
func boss_info() -> Dictionary:
	var bosses := enemies_boss()
	for i in bosses.size():
		if bosses[i] != 0:
			var kinds := enemies_kind()
			var hp := enemies_hp_permille()
			return {
				"kind": int(kinds[i]) if i < kinds.size() else 2,
				"hp_permille": int(hp[i]) if i < hp.size() else 0,
			}
	return {}

# --- tank ----------------------------------------------------------------------
func tank_hp() -> int:
	var a: PackedInt64Array = _sim.tank() if _sim != null else _arena()
	return a[2] if a.size() > 2 else 0

func tank_max_hp() -> int:
	var a: PackedInt64Array = _sim.tank() if _sim != null else _arena()
	return a[3] if a.size() > 3 else 0

# --- economy -------------------------------------------------------------------
# StSim economy() = [gold, income_per_tick, rerolls_remaining, reroll_cost];
# StMatch economy(i) = [gold, income_per_tick].
func _economy() -> PackedInt64Array:
	return _sim.economy() if _sim != null else _match.economy(_pi)

func gold() -> int:
	var e := _economy()
	return e[0] if e.size() > 0 else 0

func income() -> int:
	var e := _economy()
	return e[1] if e.size() > 1 else 0

func free_rerolls() -> int:
	var e := _economy()
	return e[2] if e.size() > 2 else 0

func reroll_cost() -> int:
	var e := _economy()
	return e[3] if e.size() > 3 else 0

# --- entities (parallel arrays; callers zip with their own size guards kept
# only where the arrays are iterated together) ---------------------------------
func enemies_pos() -> PackedVector2Array:
	return _sim.enemies_pos() if _sim != null else _match.enemies_pos(_pi)

func enemies_kind() -> PackedByteArray:
	return _sim.enemies_kind() if _sim != null else _match.enemies_kind(_pi)

# Single-arena only below (StMatch does not expose these; empty when wrapped
# around a match player).
func enemies_id() -> PackedInt64Array:
	return _sim.enemies_id() if _sim != null else PackedInt64Array()

func enemies_hp_permille() -> PackedInt32Array:
	return _sim.enemies_hp_permille() if _sim != null else PackedInt32Array()

func enemies_boss() -> PackedByteArray:
	return _sim.enemies_boss() if _sim != null else PackedByteArray()

# P3 HOOK (decode-only, no consumer yet): per-enemy status flag byte, parallel
# to enemies_pos() — bit0 frost · bit1 poison · bit2 fire · bit3 vuln ·
# bit4 stun · bit5 freeze. P3 uses it for status tints / looping status FX.
func enemies_status() -> PackedByteArray:
	return _sim.enemies_status() if _sim != null else PackedByteArray()

# Is any boss on screen this tick?
func has_boss() -> bool:
	for b in enemies_boss():
		if b != 0:
			return true
	return false

func projectiles_pos() -> PackedVector2Array:
	return _sim.projectiles_pos() if _sim != null else PackedVector2Array()

# Per-projectile stable id, parallel to projectiles_pos() (drives cross-tick
# render interpolation).
func projectiles_id() -> PackedInt64Array:
	return _sim.projectiles_id() if _sim != null else PackedInt64Array()

# Per-projectile weapon catalog index, parallel to projectiles_pos() (selects
# the projectile sprite / MultiMesh bucket).
func projectiles_kind() -> PackedInt64Array:
	return _sim.projectiles_kind() if _sim != null else PackedInt64Array()

# Per-projectile last known target position, parallel to projectiles_pos()
# (orients the sprite along its flight path).
func projectiles_target() -> PackedVector2Array:
	return _sim.projectiles_target() if _sim != null else PackedVector2Array()

# P3 HOOK (decode-only, no consumer yet): active hazards decoded from the flat
# 5-int records to [{x, y, radius, ticks_left, damage_type}, …]. P3 draws mine
# fields / burning-oil ground decals from these.
func hazards() -> Array:
	var out: Array = []
	if _sim == null:
		return out
	var raw: PackedInt64Array = _sim.hazards()
	@warning_ignore("integer_division")
	var n := raw.size() / 5
	for i in n:
		var o := i * 5
		out.append({
			"x": raw[o], "y": raw[o + 1], "radius": raw[o + 2],
			"ticks_left": int(raw[o + 3]), "damage_type": int(raw[o + 4]),
		})
	return out

func minions_pos() -> PackedVector2Array:
	return _sim.minions_pos() if _sim != null else _match.minions_pos(_pi)

func minions_kind() -> PackedByteArray:
	return _sim.minions_kind() if _sim != null else _match.minions_kind(_pi)

# Per-minion stable id, parallel to minions_pos() (drives cross-tick render
# interpolation). Single-arena only, like the other id arrays.
func minions_id() -> PackedInt64Array:
	return _sim.minions_id() if _sim != null else PackedInt64Array()

# --- Black Market (single-arena) --------------------------------------------------
# The sim holds a Black Market pick ("Buy 1 Uncommon Weapon or Spikes Damage
# Upgrade of your choosing. The Black Market lasts until a choice is made.").
# While true, the UI offers the picker overlay and submits intent code 4
# (weapon) / 5 (upgrade) with the chosen CATALOG index. Match-wrapped views
# return inert defaults — the net view's bots redeem their picks instantly.
func black_market_pending() -> bool:
	return _sim != null and _sim.black_market_pending()

# Catalog indices of the legal picks (weapons = true → Uncommon weapons,
# false → Uncommon Spikes-damage upgrades). Parallel to the names below.
func black_market_choices(weapons: bool) -> PackedInt64Array:
	return _sim.black_market_choices(weapons) if _sim != null else PackedInt64Array()

# Display names for black_market_choices(weapons), same order. English catalog
# strings — tr() them at the draw boundary like the shop names.
func black_market_choice_names(weapons: bool) -> PackedStringArray:
	return _sim.black_market_choice_names(weapons) if _sim != null else PackedStringArray()

# --- shop (single-arena) --------------------------------------------------------
# One dictionary per offer, decoding shop_meta's flat [cost, flags, rarity, …]
# (flags bit0 = is_weapon, bit1 = affordable) and shop_desc's [flavor, tip, …]:
#   {name, cost, is_weapon, affordable, rarity, flavor, tip}
func shop_offers() -> Array:
	var out: Array = []
	if _sim == null:
		return out
	var names: PackedStringArray = _sim.shop_names()
	var meta: PackedInt64Array = _sim.shop_meta()
	var desc: PackedStringArray = _sim.shop_desc()
	for i in names.size():
		var cost: int = meta[i * 3] if meta.size() > i * 3 else 0
		var flags: int = meta[i * 3 + 1] if meta.size() > i * 3 + 1 else 0
		var rarity: int = int(meta[i * 3 + 2]) if meta.size() > i * 3 + 2 else 0
		out.append({
			"name": names[i],
			"cost": cost,
			"is_weapon": (flags & 1) != 0,
			"affordable": (flags & 2) != 0,
			"rarity": rarity,
			"flavor": desc[i * 2] if i * 2 < desc.size() else "",
			"tip": desc[i * 2 + 1] if i * 2 + 1 < desc.size() else "",
		})
	return out

# --- arsenal ---------------------------------------------------------------------
func arsenal_lines() -> PackedStringArray:
	return _sim.arsenal_lines() if _sim != null else PackedStringArray()

# --- arsenal (typed, grouped) -----------------------------------------------------
# Decoded, cached views over arsenal_meta()/synergies_meta() (see the record
# layouts in godot/rust/src/lib.rs). The sim's weapon list and self-scaling
# rule list are append-only, so `arsenal_rev()` is a monotone edge: the dict
# decode below runs only when it moves (a purchase), never per frame.
var _ars_rev := -9223372036854775807   # sentinel: force first decode
var _ars_entries: Array = []
var _ars_synergies: Array = []

# Monotone revision of the owned arsenal + self-scaling rules (0 for a
# match-wrapped view, which exposes no arsenal detail).
func arsenal_rev() -> int:
	return _sim.arsenal_rev() if _sim != null else 0

# Owned weapon stacks as typed dicts, name-sorted (the marshal order):
#   {name, kind, count, class_id, damage_type, rarity}
# class_id: 0 SINGLE · 1 SPLASH · 2 BARRAGE · 3 AREA · 4 WAVE · 5 BOUNCE.
func arsenal_entries() -> Array:
	_refresh_arsenal()
	return _ars_entries

# Owned per-weapon-count self-scalers ("+X% Piercing per Bow") as typed dicts:
#   {name, kind, damage_type, per_milli, count, bonus_milli}
# per_milli/bonus_milli are milli-percent (1000 = +1%); bonus = per × count,
# already resolved against the current arsenal. Includes rules whose source
# weapon is unowned (count 0, bonus 0) — display filters those.
func arsenal_synergies() -> Array:
	_refresh_arsenal()
	return _ars_synergies

func _refresh_arsenal() -> void:
	if _sim == null:
		return
	var rev: int = _sim.arsenal_rev()
	if rev == _ars_rev:
		return
	_ars_rev = rev
	_ars_entries = []
	_ars_synergies = []
	var names: PackedStringArray = _sim.arsenal_names()
	var meta: PackedInt64Array = _sim.arsenal_meta()
	for i in names.size():
		var o := i * 5
		if o + 4 >= meta.size():
			break
		_ars_entries.append({
			"name": names[i],
			"kind": int(meta[o]),
			"count": int(meta[o + 1]),
			"class_id": int(meta[o + 2]),
			"damage_type": int(meta[o + 3]),
			"rarity": int(meta[o + 4]),
		})
	var sn: PackedStringArray = _sim.synergy_names()
	var sm: PackedInt64Array = _sim.synergies_meta()
	for i in sn.size():
		var o := i * 5
		if o + 4 >= sm.size():
			break
		_ars_synergies.append({
			"name": sn[i],
			"kind": int(sm[o]),
			"damage_type": int(sm[o + 1]),
			"per_milli": int(sm[o + 2]),
			"count": int(sm[o + 3]),
			"bonus_milli": int(sm[o + 4]),
		})

# --- damage attribution (DPS meter) ----------------------------------------------
# Reserved pseudo source ids (mirror sim-core state::DMG_SRC_*): rows with no
# single owning weapon.
const DMG_SRC_SPIKES := 0xFFFD
const DMG_SRC_CLEAR := 0xFFFE
const DMG_SRC_OTHER := 0xFFFF

# One dict per attribution row, decoding damage_meta's flat
# [source, damage_type, count, total, …] (see lib.rs) zipped with the parallel
# names:  {name, source, damage_type, count, total}
# Rows arrive in ascending source order (weapons first, pseudo rows last);
# damage_type is 255 on pseudo rows. Ranking/percentages are the caller's job.
# Built on demand (the overlay rebuilds at most ~1/s while held) — no caching.
func damage_rows() -> Array:
	var out: Array = []
	var names: PackedStringArray
	var meta: PackedInt64Array
	if _sim != null:
		names = _sim.damage_names()
		meta = _sim.damage_meta()
	elif _match != null:
		names = _match.damage_names(_pi)
		meta = _match.damage_meta(_pi)
	else:
		return out
	for i in names.size():
		var o := i * 4
		if o + 3 >= meta.size():
			break
		out.append({
			"name": names[i],
			"source": int(meta[o]),
			"damage_type": int(meta[o + 1]),
			"count": int(meta[o + 2]),
			"total": int(meta[o + 3]),
		})
	return out

# --- stats -----------------------------------------------------------------------
# stats() = [damage_dealt, gold_earned, bought_attack_mask, weapons_bought,
# economy_purchases]. Exposed both raw-by-name and as the Profile.record_match
# payload (minus "won", which only the caller can decide).
func _stats() -> PackedInt64Array:
	return _sim.stats() if _sim != null else _match.stats(_pi)

func damage_dealt() -> int:
	var st := _stats()
	return st[0] if st.size() > 0 else 0

func gold_earned() -> int:
	var st := _stats()
	return st[1] if st.size() > 1 else 0

func weapons_bought() -> int:
	var st := _stats()
	return st[3] if st.size() > 3 else 0

# The record_match() payload for this arena (caller adds "won").
func stats_record() -> Dictionary:
	var st := _stats()
	return {
		"damage": st[0] if st.size() > 0 else 0,
		"gold": st[1] if st.size() > 1 else 0,
		"round": round_num(),
		"attack_mask": st[2] if st.size() > 2 else 0,
		"weapons_bought": st[3] if st.size() > 3 else 0,
		"economy_buys": st[4] if st.size() > 4 else 0,
	}

# --- match-only (director-authoritative, per player) ------------------------------
func placement() -> int:
	return _match.placement(_pi) if _match != null else 0

func weapon_count() -> int:
	return _match.weapon_count(_pi) if _match != null else 0
