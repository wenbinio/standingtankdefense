# Audio subsystem (autoload `Audio`). RENDER-ONLY — it is driven entirely by
# sim-state deltas the renderer already reads (new projectiles, vanished enemy
# ids, tank HP drops, round/economy changes). It NEVER touches or feeds the
# deterministic sim core; it only plays sound in response to what the renderer
# already observes, exactly like the drawing code. (Randomization below uses a
# render-local RNG; nothing here is checksummed or fed back.)
#
# Layout: Master -> {SFX, Music} (see audio/default_bus_layout.tres).
#  - Master carries a hard limiter as a clipping backstop for burst moments.
#  - SFX events go through a small POOL of AudioStreamPlayers so rapid-fire
#    events (e.g. many shots / deaths in one tick) don't cut each other off.
#    Every pooled play gets slight pitch/volume randomization to fight sample
#    repetition (per-event scalable; UI/stinger sounds narrowed or exempt).
#  - Music runs on two "decks" of (base, intense) player pairs on the Music
#    bus: set_music() crossfades between decks, set_music_layers() +
#    set_intensity() do vertical mixing within the active deck, and duck()
#    dips the whole Music bus under important stingers.
#  - Master/SFX/Music volumes + mute are user prefs, persisted in the [audio]
#    section of user://profile.cfg (reload-before-save so the [profile]
#    section owned by Profile is never clobbered).
#
# Works headless: Godot's headless build uses a Dummy audio driver, so play()
# calls are safe no-ops at the device level. We still guard every stream load so
# a missing asset can never throw.
extends Node

const SFX_DIR := "res://audio/sfx/"
const MUSIC_DIR := "res://audio/music/"
const POOL_SIZE := 12                  # concurrent SFX voices
# Until real music stems land, this bed stands in for any missing layer track.
const PLACEHOLDER_MUSIC := "ambient_bed.wav"

# Event -> list of stream resource paths (a list allows simple round-robin
# variants, e.g. two fire zaps). Keys are the canonical event names main.gd /
# match.gd emit. Missing files are skipped silently at load time.
const EVENTS := {
	&"fire":           ["fire.wav", "fire_b.wav"],
	&"hit":            ["hit.wav"],
	&"enemy_death":    ["enemy_death.wav"],
	&"boss_spawn":     ["boss_spawn.wav"],
	&"tank_hit":       ["tank_hit.wav"],
	&"tank_destroyed": ["tank_destroyed.wav"],
	&"buy":            ["buy.wav"],
	&"reroll":         ["reroll.wav"],
	&"clear":          ["clear.wav"],
	&"round_start":    ["round_start.wav"],
	&"victory":        ["victory.wav"],
	&"defeat":         ["defeat.wav"],
	&"ui_move":        ["ui_move.wav"],
}

# Per-event minimum spacing (seconds) so a burst of identical deltas in one tick
# doesn't stack into a roar. Keyed by event; default 0 (no throttle). Singles
# only — play_many() is the aggregated path and supersedes this window.
const MIN_GAP := {
	&"fire":        0.04,
	&"hit":         0.04,
	&"enemy_death": 0.03,
	&"tank_hit":    0.06,
}

# --- per-play randomization (anti-repetition) ---------------------------------
# Full-scale spans; RAND_SCALE narrows or disables them per event so UI blips
# and one-shot stingers stay recognizable. Anything not listed gets 1.0.
const PITCH_SPAN := 0.08               # +/-8% pitch_scale at scale 1.0
const VOL_JITTER_DB := 1.5             # +/-1.5 dB at scale 1.0
const RAND_SCALE := {
	&"ui_move":     0.0,               # UI: exempt — identical blips read as UI
	&"victory":     0.0,               # stingers: play as authored
	&"defeat":      0.0,
	&"buy":         0.25,              # economy: a hint of life, still "clicky"
	&"reroll":      0.25,
	&"clear":       0.25,
	&"round_start": 0.25,
}

# --- mass-event scaling (play_many) --------------------------------------------
# One voice whose gain grows ~1 dB per doubling of count (log curve), capped,
# with a slight downward pitch so big wipes read as *heavier*, not just louder.
const MASS_GAIN_PER_DOUBLING_DB := 1.0
const MASS_GAIN_MAX_DB := 4.0          # cap: reached at count >= 16
const MASS_PITCH_PER_DOUBLING := 0.03
const MASS_PITCH_FLOOR := 0.88         # never deeper than -12%

# --- music ducking --------------------------------------------------------------
const DUCK_EVENTS := {                 # plays that auto-duck the music bed
	&"victory": true,
	&"defeat": true,
	&"boss_spawn": true,
	&"tank_destroyed": true,
}
const DUCK_DB := -6.0
const DUCK_ATTACK := 0.05
const DUCK_HOLD := 0.6
const DUCK_RELEASE := 0.8

# --- music crossfade / vertical layering ----------------------------------------
const CROSSFADE_SEC := 1.5
const INTENSITY_FADE_SEC := 0.5
const SILENT_DB := -60.0

var _streams := {}                     # StringName event -> Array[AudioStream]
var _variant_idx := {}                 # StringName event -> next variant index
var _pool: Array[AudioStreamPlayer] = []
var _next := 0                         # round-robin index into _pool
var _last_play := {}                   # StringName -> last play time (sec)
var _rng := RandomNumberGenerator.new()  # render-local; never feeds the sim

var _muted := false
var _master_db := 0.0                  # user volumes in dB (0 = unity)
var _sfx_db := 0.0
var _music_db := -8.0                  # music bed sits under SFX by default

# Music decks: two (base, intense) player pairs; crossfades swap decks,
# intensity mixes the intense layer in on top of base within a deck.
var _decks: Array = []                 # [[base, intense], [base, intense]]
var _deck_db: Array[float] = [SILENT_DB, SILENT_DB]
var _active_deck := 0
var _active_track := ""                # "base|intense" key to skip redundant swaps
var _intensity := 0.0
var _intensity_db := SILENT_DB         # dB offset of intense layer vs. base
var _duck_db_now := 0.0                # current duck dip applied to Music bus
var _fade_tween: Tween
var _intensity_tween: Tween
var _duck_tween: Tween

func _ready() -> void:
	_rng.randomize()
	# Lay out the buses up front so SFX/Music exist even if the project's saved
	# layout wasn't applied (e.g. older project.godot). Idempotent.
	_ensure_buses()
	_load_streams()
	_build_pool()
	_build_music_decks()
	_load_prefs()
	_apply_volume()

# --- public API --------------------------------------------------------------

# Play the SFX bound to `event` (a canonical event name). Cheap and safe to call
# every frame; unknown events and empty pools are no-ops. Honors a small
# per-event throttle so one-tick bursts don't stack. Big stingers auto-duck the
# music bed.
func play(event: StringName) -> void:
	if _muted:
		return
	var list: Array = _streams.get(event, [])
	if list.is_empty():
		return
	var now := _now()
	var gap: float = MIN_GAP.get(event, 0.0)
	if gap > 0.0 and now - float(_last_play.get(event, -1000.0)) < gap:
		return
	_last_play[event] = now
	# pick the next variant (round-robin) and the next free-ish pool voice
	var vi: int = _variant_idx.get(event, 0)
	_variant_idx[event] = (vi + 1) % list.size()
	_play_stream(list[vi], 0.0, 1.0, float(RAND_SCALE.get(event, 1.0)))
	if DUCK_EVENTS.has(event):
		duck()

# Count-scaled playback for aggregated mass events (e.g. all EnemyKilled in one
# tick): ONE voice, louder (+~1 dB per doubling, capped at +4 dB) and slightly
# deeper with count, so a 50-kill wipe sounds bigger than one kill instead of
# identical. count == 1 falls through to plain play() (keeps MIN_GAP behavior);
# the aggregated voice itself skips MIN_GAP (call sites batch per tick) but
# refreshes the window so trailing singles don't double up on top of it.
func play_many(event: StringName, count: int) -> void:
	if count <= 0:
		return
	if count == 1:
		play(event)
		return
	if _muted:
		return
	var list: Array = _streams.get(event, [])
	if list.is_empty():
		return
	_last_play[event] = _now()
	var vi: int = _variant_idx.get(event, 0)
	_variant_idx[event] = (vi + 1) % list.size()
	var doublings := _log2(float(count))
	var gain := minf(MASS_GAIN_MAX_DB, MASS_GAIN_PER_DOUBLING_DB * doublings)
	var pitch := maxf(MASS_PITCH_FLOOR, 1.0 - MASS_PITCH_PER_DOUBLING * doublings)
	_play_stream(list[vi], gain, pitch, float(RAND_SCALE.get(event, 1.0)))
	if DUCK_EVENTS.has(event):
		duck()

# Swap the looping music bed with a ~1.5 s crossfade. `track` is a filename
# under audio/music/ (e.g. "ambient_bed.wav") or "" / null to fade out and
# stop. Unknown filenames are ignored (current music keeps playing). Loops
# automatically.
func set_music(track) -> void:
	var trackname := "" if track == null else String(track)
	if trackname != "" and _load_music(trackname, false) == null:
		return
	_start_deck(trackname, "", false)

# Vertical mixing: crossfade into a deck holding a base stem plus an intense
# stem whose level follows set_intensity(). Missing stems fall back to the
# ambient-bed placeholder so the system works before real stems exist.
func set_music_layers(
	base: String = PLACEHOLDER_MUSIC,
	intense: String = PLACEHOLDER_MUSIC,
) -> void:
	var b := base if base != "" else PLACEHOLDER_MUSIC
	var i := intense if intense != "" else PLACEHOLDER_MUSIC
	_start_deck(b, i, true)

# Vertical-mix intensity, 0.0 (base only) .. 1.0 (intense layer at full).
# Smoothed over INTENSITY_FADE_SEC to avoid zipper artifacts. Render-side
# callers may derive t from sim reads (enemy count / boss flag / round).
func set_intensity(t: float) -> void:
	t = clampf(t, 0.0, 1.0)
	if is_equal_approx(t, _intensity):
		return
	_intensity = t
	var target := maxf(linear_to_db(t), SILENT_DB) if t > 0.0 else SILENT_DB
	if _intensity_tween != null and _intensity_tween.is_valid():
		_intensity_tween.kill()
	_intensity_tween = create_tween()
	_intensity_tween.tween_method(_set_intensity_db, _intensity_db, target, INTENSITY_FADE_SEC)

# Scripted music duck: dip the Music bus by `db` (negative) over `attack`,
# hold, then recover over `release`. Overlapping ducks keep the deeper dip and
# restart the hold, so back-to-back stingers stay ducked. Auto-triggered by
# DUCK_EVENTS plays; also callable directly.
func duck(
	db: float = DUCK_DB,
	attack: float = DUCK_ATTACK,
	hold: float = DUCK_HOLD,
	release: float = DUCK_RELEASE,
) -> void:
	db = minf(db, 0.0)
	if _duck_tween != null and _duck_tween.is_valid():
		_duck_tween.kill()
	var depth := minf(db, _duck_db_now)
	_duck_tween = create_tween()
	_duck_tween.tween_method(_set_duck_db, _duck_db_now, depth, attack)
	_duck_tween.tween_interval(hold)
	_duck_tween.tween_method(_set_duck_db, depth, 0.0, release)

# Master volume, 0.0..1.0 (linear), persisted. 1.0 == unity (0 dB).
func set_master_volume(linear: float) -> void:
	_master_db = _user_db(linear)
	_apply_volume()
	_save_prefs()

# SFX bus volume, 0.0..1.0 (linear), persisted.
func set_sfx_volume(linear: float) -> void:
	_sfx_db = _user_db(linear)
	_apply_volume()
	_save_prefs()

# Music bus volume, 0.0..1.0 (linear), persisted. Default sits at -8 dB
# (~0.4 linear) so the bed rides under SFX, matching the original mix.
func set_music_volume(linear: float) -> void:
	_music_db = _user_db(linear)
	_apply_volume()
	_save_prefs()

func master_volume() -> float:
	return db_to_linear(_master_db)

func sfx_volume() -> float:
	return db_to_linear(_sfx_db)

func music_volume() -> float:
	return db_to_linear(_music_db)

# Toggle global mute (Master bus). Returns the new muted state. Persisted.
func toggle_mute() -> bool:
	_muted = not _muted
	_apply_volume()
	if _muted:
		# Stop the bed so it doesn't resume mid-loop on unmute oddly; restart on unmute.
		for deck in _decks:
			for p in deck:
				p.stop()
	else:
		# Restart the active deck's loaded layers together (keeps them in sync).
		if not _decks.is_empty():
			for p in _decks[_active_deck]:
				if p.stream != null:
					p.play()
	_save_prefs()
	return _muted

func is_muted() -> bool:
	return _muted

# --- internals ---------------------------------------------------------------

func _ensure_buses() -> void:
	# Master is index 0 always. Add SFX / Music if a custom layout wasn't loaded.
	if AudioServer.get_bus_index("SFX") < 0:
		AudioServer.add_bus()
		var i := AudioServer.bus_count - 1
		AudioServer.set_bus_name(i, "SFX")
		AudioServer.set_bus_send(i, "Master")
	if AudioServer.get_bus_index("Music") < 0:
		AudioServer.add_bus()
		var i := AudioServer.bus_count - 1
		AudioServer.set_bus_name(i, "Music")
		AudioServer.set_bus_send(i, "Master")
		AudioServer.set_bus_volume_db(i, -8.0)
	# Clipping backstop: hard limiter on Master. The saved bus layout already
	# carries one; add it here too in case the layout wasn't applied.
	# Idempotent.
	var master := AudioServer.get_bus_index("Master")
	if master >= 0:
		var has_limiter := false
		for i in AudioServer.get_bus_effect_count(master):
			if AudioServer.get_bus_effect(master, i) is AudioEffectHardLimiter:
				has_limiter = true
				break
		if not has_limiter:
			AudioServer.add_bus_effect(master, AudioEffectHardLimiter.new())

func _load_streams() -> void:
	for ev in EVENTS:
		var arr: Array[AudioStream] = []
		for fname in EVENTS[ev]:
			var path: String = SFX_DIR + fname
			if ResourceLoader.exists(path):
				var s := load(path) as AudioStream
				if s != null:
					arr.append(s)
		if not arr.is_empty():
			_streams[ev] = arr

func _build_pool() -> void:
	for i in POOL_SIZE:
		var p := AudioStreamPlayer.new()
		p.bus = "SFX"
		add_child(p)
		_pool.append(p)

func _build_music_decks() -> void:
	for _d in 2:
		var pair: Array = []
		for _i in 2:
			var p := AudioStreamPlayer.new()
			p.bus = "Music"
			p.volume_db = SILENT_DB
			add_child(p)
			pair.append(p)
		_decks.append(pair)

# Prefer an idle voice; else steal the oldest in round-robin order so rapid
# events never get dropped just because every voice is mid-tail.
func _pick_player() -> AudioStreamPlayer:
	for _i in _pool.size():
		var p := _pool[_next]
		_next = (_next + 1) % _pool.size()
		if not p.playing:
			return p
	# all busy: take the next in rotation (steal)
	var q := _pool[_next]
	_next = (_next + 1) % _pool.size()
	return q

# Fire one pooled voice with per-play randomization. `rand_scale` narrows the
# jitter (1.0 = full +/-8% pitch, +/-1.5 dB; 0.0 = exact). Pitch and volume are
# always (re)set here because pool voices are reused.
func _play_stream(
	stream: AudioStream,
	base_db: float,
	base_pitch: float,
	rand_scale: float,
) -> void:
	var p := _pick_player()
	p.stream = stream
	p.volume_db = base_db + _rng.randf_range(-VOL_JITTER_DB, VOL_JITTER_DB) * rand_scale
	p.pitch_scale = base_pitch * (1.0 + _rng.randf_range(-PITCH_SPAN, PITCH_SPAN) * rand_scale)
	p.play()

# Crossfade into the inactive deck loaded with (base, intense). Empty base ==
# fade to silence. `fallback` routes missing files to the placeholder bed
# (layered mode); without it missing files were already rejected by the caller.
func _start_deck(base_track: String, intense_track: String, fallback: bool) -> void:
	if _decks.is_empty():
		return
	var key := base_track + "|" + intense_track
	if key == _active_track:
		return
	_active_track = key
	var base_s := _load_music(base_track, fallback)
	var int_s := _load_music(intense_track, fallback)
	var outgoing := _active_deck
	var incoming := 1 - _active_deck
	_active_deck = incoming
	var pair: Array = _decks[incoming]
	var base_p: AudioStreamPlayer = pair[0]
	var int_p: AudioStreamPlayer = pair[1]
	base_p.stop()
	int_p.stop()
	base_p.stream = base_s
	int_p.stream = int_s
	_apply_deck(incoming)
	if not _muted:
		if base_s != null:
			base_p.play()
		if int_s != null:
			int_p.play()
	# Crossfade decks. Killing any in-flight fade first makes mid-crossfade
	# swaps safe: the new fade starts from each deck's *current* gain, and the
	# reclaimed deck was already stopped/reloaded above.
	if _fade_tween != null and _fade_tween.is_valid():
		_fade_tween.kill()
	var target_in := 0.0 if base_s != null else SILENT_DB
	_fade_tween = create_tween().set_parallel(true)
	var in_cb := _set_deck_gain.bind(incoming)
	var out_cb := _set_deck_gain.bind(outgoing)
	_fade_tween.tween_method(in_cb, _deck_db[incoming], target_in, CROSSFADE_SEC)
	_fade_tween.tween_method(out_cb, _deck_db[outgoing], SILENT_DB, CROSSFADE_SEC)
	_fade_tween.chain().tween_callback(_stop_deck.bind(outgoing))

func _set_deck_gain(db: float, deck: int) -> void:
	_deck_db[deck] = db
	_apply_deck(deck)

func _apply_deck(deck: int) -> void:
	var pair: Array = _decks[deck]
	var base_p: AudioStreamPlayer = pair[0]
	var int_p: AudioStreamPlayer = pair[1]
	base_p.volume_db = _deck_db[deck]
	int_p.volume_db = _deck_db[deck] + _intensity_db

func _stop_deck(deck: int) -> void:
	if deck == _active_deck:
		return  # a newer crossfade reclaimed this deck; leave it alone
	for p in _decks[deck]:
		p.stop()

func _set_intensity_db(db: float) -> void:
	_intensity_db = db
	_apply_deck(0)
	_apply_deck(1)

func _set_duck_db(db: float) -> void:
	_duck_db_now = db
	_apply_music_bus()

func _apply_volume() -> void:
	var master := AudioServer.get_bus_index("Master")
	if master >= 0:
		AudioServer.set_bus_mute(master, _muted)
		AudioServer.set_bus_volume_db(master, _master_db)
	var sfx := AudioServer.get_bus_index("SFX")
	if sfx >= 0:
		AudioServer.set_bus_volume_db(sfx, _sfx_db)
	_apply_music_bus()

func _apply_music_bus() -> void:
	var music := AudioServer.get_bus_index("Music")
	if music >= 0:
		AudioServer.set_bus_volume_db(music, _music_db + _duck_db_now)

# Load (and loop-flag) a music stream. Returns null for "" or, when `fallback`
# is off, for missing files. With `fallback` on, missing stems resolve to the
# placeholder bed so layered mode works before real assets exist.
func _load_music(track: String, fallback: bool) -> AudioStream:
	if track == "":
		return null
	var path := MUSIC_DIR + track
	if not ResourceLoader.exists(path):
		if fallback and track != PLACEHOLDER_MUSIC:
			return _load_music(PLACEHOLDER_MUSIC, false)
		return null
	var s := load(path) as AudioStream
	if s == null:
		return null
	_set_loop(s)
	return s

func _user_db(linear: float) -> float:
	linear = clampf(linear, 0.0, 1.0)
	return linear_to_db(linear) if linear > 0.0 else -80.0

func _log2(x: float) -> float:
	return log(x) / log(2.0)

func _now() -> float:
	return float(Time.get_ticks_msec()) / 1000.0

# Ensure a stream loops (used for the music bed/stems). Handles the common
# stream types defensively so a swapped-in format still loops.
func _set_loop(s: AudioStream) -> void:
	if s is AudioStreamWAV:
		(s as AudioStreamWAV).loop_mode = AudioStreamWAV.LOOP_FORWARD
	elif s is AudioStreamOggVorbis:
		(s as AudioStreamOggVorbis).loop = true
	elif s is AudioStreamMP3:
		(s as AudioStreamMP3).loop = true

# --- prefs (cosmetic, render-layer only; never feeds the sim) ----------------

func _load_prefs() -> void:
	# Persist via Profile's ConfigFile if that autoload exposes the helpers; else
	# fall back to a tiny standalone config. Either way this is pure UX state.
	var cf := ConfigFile.new()
	if cf.load("user://profile.cfg") == OK:
		_muted = bool(cf.get_value("audio", "muted", false))
		_master_db = float(cf.get_value("audio", "master_db", 0.0))
		_sfx_db = float(cf.get_value("audio", "sfx_db", 0.0))
		_music_db = float(cf.get_value("audio", "music_db", -8.0))

func _save_prefs() -> void:
	var cf := ConfigFile.new()
	cf.load("user://profile.cfg")          # keep existing profile sections intact
	cf.set_value("audio", "muted", _muted)
	cf.set_value("audio", "master_db", _master_db)
	cf.set_value("audio", "sfx_db", _sfx_db)
	cf.set_value("audio", "music_db", _music_db)
	cf.save("user://profile.cfg")
