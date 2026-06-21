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
	# Purist skins — one per weapon attack-class (buy only that class).
	{"id": "deadeye",         "name": "Deadeye Dan",      "file": "skins/deadeye.svg",           "unlock": "purist_single","blurb": "One-eyed single-shot sniper."},
	{"id": "spicy_meatball",  "name": "Spicy Meatball",   "file": "skins/spicy_meatball.svg",    "unlock": "purist_splash","blurb": "Splash-only. BELCHES fire."},
	{"id": "octo_blaster",    "name": "Octo-Blaster",     "file": "skins/octo_blaster.svg",      "unlock": "purist_barrage","blurb": "Eight arms, eight barrages."},
	{"id": "disco_doom",      "name": "Disco Doom",       "file": "skins/disco_doom.svg",        "unlock": "purist_area",  "blurb": "Pulsing aura of pure groove."},
	{"id": "tidal_terry",     "name": "Tidal Terry",      "file": "skins/tidal_terry.svg",       "unlock": "purist_wave",  "blurb": "A sweeping wave. Gnarly."},
	{"id": "bouncy_boi",      "name": "Bouncy Boi",       "file": "skins/bouncy_boi.svg",        "unlock": "purist_bounce","blurb": "Boing. Boing. Ricochet."},
	# Special challenges.
	{"id": "stone_broke",     "name": "Stone Broke",      "file": "skins/stone_broke.svg",       "unlock": "no_economy",   "blurb": "No income. All vibes."},
	{"id": "franken_tank",    "name": "Franken-Tank",     "file": "skins/franken_tank.svg",      "unlock": "jack_of_all",  "blurb": "A bit of every weapon type."},
	# Score-threshold skins.
	{"id": "gore_hound",      "name": "Gore Hound",       "file": "skins/gore_hound.svg",        "unlock": "bloodletter",  "blurb": "It has tasted 1,000,000 HP."},
	{"id": "chilly_willy",    "name": "Chilly Willy",     "file": "skins/chilly_willy.svg",      "unlock": "long_watch",   "blurb": "Snowman. Carrot goes BOOM."},
	{"id": "sir_toots",       "name": "Sir Toots-a-Lot",  "file": "skins/sir_toots.svg",         "unlock": "war_profiteer","blurb": "A brass steam-teapot of war."},
	{"id": "lord_spookington","name": "Lord Spookington", "file": "skins/lord_spookington.svg",  "unlock": "sole_survivor","blurb": "A floaty eldritch eyeball. Hi."},
]

# --- Achievements ------------------------------------------------------------
# Evaluated at match-end against a record built from the authoritative sim stats:
#   rec = { damage, gold, round, won, attack_mask, weapons_bought, economy_buys }
# attack_mask bit n == attack-class n (0 single / 1 splash / 2 barrage /
# 3 area / 4 wave / 5 bounce), set only by weapons the player BOUGHT.
const ATTACK_NAMES := ["Single-Target", "Splash", "Barrage", "Area", "Wave", "Bounce"]
const ALL_TYPES_MASK := 0b111111   # 63 — every attack class
const PURIST_MIN_WEAPONS := 3      # a purist run must commit to ≥3 buys

const ACHIEVEMENTS: Array[Dictionary] = [
	{"id": "purist_single",  "name": "One-Trick Sniper",  "desc": "Buy only Single-Target weapons (3+)."},
	{"id": "purist_splash",  "name": "Boom Enthusiast",   "desc": "Buy only Splash weapons (3+)."},
	{"id": "purist_barrage", "name": "Spray 'n' Pray",    "desc": "Buy only Barrage weapons (3+)."},
	{"id": "purist_area",    "name": "Aura Farmer",       "desc": "Buy only Area weapons (3+)."},
	{"id": "purist_wave",    "name": "Wavy Gravy",        "desc": "Buy only Wave weapons (3+)."},
	{"id": "purist_bounce",  "name": "Ricochet Rascal",   "desc": "Buy only Bounce weapons (3+)."},
	{"id": "no_economy",     "name": "Stone Broke",       "desc": "Reach round 10 with zero income buys."},
	{"id": "jack_of_all",    "name": "Jack of All Trades", "desc": "Buy a weapon of every attack type."},
	{"id": "bloodletter",    "name": "Bloodletter",       "desc": "Deal 1,000,000 damage in a match."},
	{"id": "long_watch",     "name": "Long Watch",        "desc": "Survive to round 20."},
	{"id": "war_profiteer",  "name": "War Profiteer",     "desc": "Earn 500,000 gold in a match."},
	{"id": "sole_survivor",  "name": "Sole Survivor",     "desc": "Win a match (last tank standing)."},
]

var earned := {}                 # ach_id -> true
var selected := "ol_reliable"
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
func _purist(rec: Dictionary, bit: int) -> bool:
	return int(rec.get("attack_mask", 0)) == (1 << bit) \
		and int(rec.get("weapons_bought", 0)) >= PURIST_MIN_WEAPONS

func _qualifies(ach_id: String, rec: Dictionary) -> bool:
	match ach_id:
		"purist_single":  return _purist(rec, 0)
		"purist_splash":  return _purist(rec, 1)
		"purist_barrage": return _purist(rec, 2)
		"purist_area":    return _purist(rec, 3)
		"purist_wave":    return _purist(rec, 4)
		"purist_bounce":  return _purist(rec, 5)
		"no_economy":     return int(rec.get("round", 0)) >= 10 and int(rec.get("economy_buys", 0)) == 0
		"jack_of_all":    return int(rec.get("attack_mask", 0)) == ALL_TYPES_MASK
		"bloodletter":    return int(rec.get("damage", 0)) >= 1000000
		"long_watch":     return int(rec.get("round", 0)) >= 20
		"war_profiteer":  return int(rec.get("gold", 0)) >= 500000
		"sole_survivor":  return bool(rec.get("won", false))
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
	selected = "ol_reliable"
	_save()

# --- persistence -------------------------------------------------------------
func _load() -> void:
	var cf := ConfigFile.new()
	if cf.load(SAVE_PATH) != OK:
		return
	selected = cf.get_value("profile", "selected", "ol_reliable")
	for id in cf.get_value("profile", "earned", []):
		earned[id] = true
	if not is_unlocked(selected):   # a skin that lost its unlock falls back
		selected = "ol_reliable"

func _save() -> void:
	var cf := ConfigFile.new()
	cf.set_value("profile", "selected", selected)
	cf.set_value("profile", "earned", earned.keys())
	cf.save(SAVE_PATH)
