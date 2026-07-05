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
	{"id": "ol_reliable",     "name": "Hermit-Crab Fortress", "file": "",                        "unlock": "",             "blurb": "The default defender: a dug-in crab-keep (war-tortoise alt skin)."},
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
	{"id": "no_economy",     "name": "Stone Broke",       "desc": "Buy 6 weapons with zero income purchases."},
	{"id": "jack_of_all",    "name": "Jack of All Trades", "desc": "Buy a weapon of every attack type."},
	{"id": "bloodletter",    "name": "Bloodletter",       "desc": "Deal 1,000,000 damage in a match."},
	{"id": "long_watch",     "name": "Long Watch",        "desc": "Survive to round 20."},
	{"id": "war_profiteer",  "name": "War Profiteer",     "desc": "Earn 500,000 gold in a match."},
	{"id": "sole_survivor",  "name": "Sole Survivor",     "desc": "Win a match (last tank standing)."},
]

# --- Challenges --------------------------------------------------------------
# Optional self-imposed rules the preview's player-0 bot will honor, so the
# constraint achievements are earnable on demand. `code` matches the Rust
# `bot::Challenge::from_code` (0 = free play). `ach` is the achievement (and
# thus skin) the run targets. Cosmetic preview aid only.
const CHALLENGES: Array[Dictionary] = [
	{"name": "Purist: Single-Target", "code": 1, "ach": "purist_single",  "rule": "Bot buys ONLY Single-Target weapons."},
	{"name": "Purist: Splash",        "code": 2, "ach": "purist_splash",  "rule": "Bot buys ONLY Splash weapons."},
	{"name": "Purist: Barrage",       "code": 3, "ach": "purist_barrage", "rule": "Bot buys ONLY Barrage weapons."},
	{"name": "Purist: Area",          "code": 4, "ach": "purist_area",    "rule": "Bot buys ONLY Area weapons."},
	{"name": "Purist: Wave",          "code": 5, "ach": "purist_wave",    "rule": "Bot buys ONLY Wave weapons."},
	{"name": "Purist: Bounce",        "code": 6, "ach": "purist_bounce",  "rule": "Bot buys ONLY Bounce weapons."},
	{"name": "No Economy",            "code": 7, "ach": "no_economy",     "rule": "Bot never buys income, only weapons."},
	{"name": "Jack of All Trades",    "code": 8, "ach": "jack_of_all",    "rule": "Bot collects a weapon of every type."},
]

var earned := {}                 # ach_id -> true
var selected := "ol_reliable"
var active_challenge_code := 0   # transient: applied to player 0 on next deploy
var last_unlocks: Array = []     # ach ids granted by the most recent record_match()
var _locale := "en"              # persisted UI language ("en" / "zh_CN")
var _screen_shake := true        # persisted render pref: camera shake/zoom punch
var _game_speed := 0             # persisted single-player pace (index into SPEED_TPS)

# Single-player game-speed table: sim ticks per wall-clock second by speed code
# (0 Normal ×1.0 · 1 Fast ×1.5 · 2 Faster ×2.0 · 3 Hyper ×3.0). CADENCE ONLY:
# main.gd's accumulator just calls sim.step() more often — every tick stays a
# bit-identical 30 Hz sim tick, so this pref can never touch determinism.
# Mirrors net::GameSpeed (the lobby's host-set speed for netplay).
const SPEED_TPS := [30, 45, 60, 90]

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

# The skin granted by an achievement id (for challenge-reward previews).
func skin_for_ach(ach_id: String) -> Dictionary:
	for s in SKINS:
		if s.unlock == ach_id:
			return s
	return SKINS[0]

# --- UI language (cosmetic, render-layer only) -------------------------------
# Persisted UI locale. Never feeds the sim; the sim core stays English-only.
func locale() -> String:
	return _locale

func set_locale_pref(loc: String) -> void:
	_locale = loc
	_save()

# --- screen shake (cosmetic render pref, persisted) ----------------------------
# Consumed by main.gd at its camera-offset application point (the one place fx
# shake/zoom-punch reach the camera). Never feeds the sim.
func screen_shake() -> bool:
	return _screen_shake

func set_screen_shake(on: bool) -> void:
	_screen_shake = on
	_save()

# --- game speed (single-player pace pref, persisted; cadence-only) ------------
func game_speed() -> int:
	return _game_speed

func set_game_speed(code: int) -> void:
	_game_speed = clampi(code, 0, SPEED_TPS.size() - 1)
	_save()

# Ticks the single-player driver should run per wall-clock second (30/45/60/90).
func ticks_per_second() -> int:
	return SPEED_TPS[_game_speed]

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
		"no_economy":     return int(rec.get("weapons_bought", 0)) >= 6 and int(rec.get("economy_buys", 0)) == 0
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
	_locale = cf.get_value("profile", "locale", "en")
	_screen_shake = bool(cf.get_value("profile", "screen_shake", true))
	_game_speed = clampi(int(cf.get_value("profile", "game_speed", 0)), 0, SPEED_TPS.size() - 1)
	for id in cf.get_value("profile", "earned", []):
		earned[id] = true
	if not is_unlocked(selected):   # a skin that lost its unlock falls back
		selected = "ol_reliable"

func _save() -> void:
	# user://profile.cfg is shared with audio.gd (its [audio] section). Reload the
	# file before writing so we only overwrite [profile] and preserve the rest —
	# the same reload-before-save pattern audio.gd uses for its section.
	var cf := ConfigFile.new()
	cf.load(SAVE_PATH)   # ignore failure: a missing file just starts empty
	cf.set_value("profile", "selected", selected)
	cf.set_value("profile", "locale", _locale)
	cf.set_value("profile", "screen_shake", _screen_shake)
	cf.set_value("profile", "game_speed", _game_speed)
	cf.set_value("profile", "earned", earned.keys())
	cf.save(SAVE_PATH)
