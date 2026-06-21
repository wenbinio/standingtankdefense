# Player profile (autoload `Profile`): persistent cosmetic unlocks + the selected
# tank skin. COSMETIC ONLY — none of this ever feeds the deterministic sim; it
# lives purely in the engine/render layer. Saved to user://profile.cfg.
# Achievements are granted at match-end from the authoritative sim stats.
extends Node

const SAVE_PATH := "user://profile.cfg"

# --- Skin roster -------------------------------------------------------------
# `file` is relative to the active theme's tank/ folder ("" = the theme's own
# default player_tank.svg). `unlock` = "" means unlocked by default; otherwise
# it names the achievement that grants it.
const SKINS: Array[Dictionary] = [
	{"id": "ol_reliable",     "name": "Ol' Reliable",     "file": "",                            "unlock": "",             "blurb": "The trusty, dusty starter cannon."},
	{"id": "spicy_meatball",  "name": "Spicy Meatball",   "file": "skins/spicy_meatball.svg",    "unlock": "bloodletter",  "blurb": "Molten orb. BELCHES fire."},
	{"id": "chilly_willy",    "name": "Chilly Willy",     "file": "skins/chilly_willy.svg",      "unlock": "long_watch",   "blurb": "Snowman. Carrot goes BOOM."},
	{"id": "sir_toots",       "name": "Sir Toots-a-Lot",  "file": "skins/sir_toots.svg",         "unlock": "war_profiteer","blurb": "A brass steam-teapot of war."},
	{"id": "lord_spookington","name": "Lord Spookington", "file": "skins/lord_spookington.svg",  "unlock": "sole_survivor","blurb": "A floaty eldritch eyeball. Hi."},
]

# --- Achievements (evaluated against a match-result record) -------------------
# rec = { "damage": int, "gold": int, "round": int, "won": bool }
const ACHIEVEMENTS: Array[Dictionary] = [
	{"id": "bloodletter",   "name": "Bloodletter",   "desc": "Deal 1,000,000 damage in a match."},
	{"id": "long_watch",    "name": "Long Watch",    "desc": "Survive to round 20."},
	{"id": "war_profiteer", "name": "War Profiteer", "desc": "Earn 500,000 gold in a match."},
	{"id": "sole_survivor", "name": "Sole Survivor", "desc": "Win a match (last tank standing)."},
]

var earned := {}                 # ach_id -> true
var selected := "ashen_vigil"
var last_unlocks: Array = []     # ach ids granted by the most recent record_match()

func _ready() -> void:
	_load()

# --- queries -----------------------------------------------------------------
func has_ach(id: String) -> bool:
	return earned.has(id)

func skin_def(id: String) -> Dictionary:
	for s in SKINS:
		if s.id == id:
			return s
	return SKINS[0]

func ach_def(id: String) -> Dictionary:
	for a in ACHIEVEMENTS:
		if a.id == id:
			return a
	return {}

func is_unlocked(skin_id: String) -> bool:
	var u: String = skin_def(skin_id).unlock
	return u == "" or earned.has(u)

func select(skin_id: String) -> bool:
	if not is_unlocked(skin_id):
		return false
	selected = skin_id
	_save()
	return true

# --- achievement evaluation at match end ------------------------------------
func _qualifies(ach_id: String, rec: Dictionary) -> bool:
	match ach_id:
		"bloodletter":   return int(rec.get("damage", 0)) >= 1000000
		"long_watch":    return int(rec.get("round", 0)) >= 20
		"war_profiteer": return int(rec.get("gold", 0)) >= 500000
		"sole_survivor": return bool(rec.get("won", false))
	return false

# Returns the list of newly-unlocked achievement ids (for a toast).
func record_match(rec: Dictionary) -> Array:
	last_unlocks = []
	for a in ACHIEVEMENTS:
		if not earned.has(a.id) and _qualifies(a.id, rec):
			earned[a.id] = true
			last_unlocks.append(a.id)
	if not last_unlocks.is_empty():
		_save()
	return last_unlocks

# --- dev helpers (no real match needed to preview the gallery) ---------------
func unlock_all() -> void:
	for a in ACHIEVEMENTS:
		earned[a.id] = true
	_save()

func reset() -> void:
	earned = {}
	selected = "ashen_vigil"
	_save()

# --- persistence -------------------------------------------------------------
func _load() -> void:
	var cf := ConfigFile.new()
	if cf.load(SAVE_PATH) != OK:
		return
	selected = cf.get_value("profile", "selected", "ashen_vigil")
	for id in cf.get_value("profile", "earned", []):
		earned[id] = true
	if not is_unlocked(selected):   # a skin that lost its unlock falls back
		selected = "ashen_vigil"

func _save() -> void:
	var cf := ConfigFile.new()
	cf.set_value("profile", "selected", selected)
	cf.set_value("profile", "earned", earned.keys())
	cf.save(SAVE_PATH)
