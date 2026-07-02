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
# EXTENSION POINT (do not implement yet): when the P1.1 event stream lands in
# the Rust binding, drain-and-expose it here as e.g.
#   func take_events() -> Array          # one-shot render/audio event queue
#   func enemies_status() -> Packed...   # per-enemy status-effect flags
# so every consumer (FX, audio, damage numbers) reads events through this one
# seam. Until those accessors exist in godot/rust, they MUST NOT be called.
# ---------------------------------------------------------------------------
class_name SimView
extends RefCounted

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

# Is any boss on screen this tick?
func has_boss() -> bool:
	for b in enemies_boss():
		if b != 0:
			return true
	return false

func projectiles_pos() -> PackedVector2Array:
	return _sim.projectiles_pos() if _sim != null else PackedVector2Array()

func minions_pos() -> PackedVector2Array:
	return _sim.minions_pos() if _sim != null else _match.minions_pos(_pi)

func minions_kind() -> PackedByteArray:
	return _sim.minions_kind() if _sim != null else _match.minions_kind(_pi)

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
