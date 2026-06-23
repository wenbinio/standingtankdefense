# Audio subsystem (autoload `Audio`). RENDER-ONLY — it is driven entirely by
# sim-state deltas the renderer already reads (new projectiles, vanished enemy
# ids, tank HP drops, round/economy changes). It NEVER touches or feeds the
# deterministic sim core; it only plays sound in response to what the renderer
# already observes, exactly like the drawing code.
#
# Layout: Master -> {SFX, Music} (see audio/default_bus_layout.tres).
#  - SFX events go through a small POOL of AudioStreamPlayers so rapid-fire
#    events (e.g. many shots / deaths in one tick) don't cut each other off.
#  - One dedicated player drives the looping music/ambient bed on the Music bus.
#  - Master volume + mute are user prefs, persisted via Profile if available.
#
# Works headless: Godot's headless build uses a Dummy audio driver, so play()
# calls are safe no-ops at the device level. We still guard every stream load so
# a missing asset can never throw.
extends Node

const SFX_DIR := "res://audio/sfx/"
const MUSIC_DIR := "res://audio/music/"
const POOL_SIZE := 12                  # concurrent SFX voices

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

var _streams := {}                     # StringName event -> Array[AudioStream]
var _variant_idx := {}                 # StringName event -> next variant index
var _pool: Array[AudioStreamPlayer] = []
var _next := 0                         # round-robin index into _pool
var _music: AudioStreamPlayer
var _muted := false
var _master_db := 0.0                  # user master volume in dB (0 = unity)

# Per-event minimum spacing (seconds) so a burst of identical deltas in one tick
# doesn't stack into a roar. Keyed by event; default 0 (no throttle).
const MIN_GAP := {
	&"fire":        0.04,
	&"hit":         0.04,
	&"enemy_death": 0.03,
	&"tank_hit":    0.06,
}
var _last_play := {}                    # StringName -> last play time (sec)

func _ready() -> void:
	# Lay out the buses up front so SFX/Music exist even if the project's saved
	# layout wasn't applied (e.g. older project.godot). Idempotent.
	_ensure_buses()
	_load_streams()
	_build_pool()
	_music = AudioStreamPlayer.new()
	_music.bus = "Music"
	add_child(_music)
	_load_prefs()
	_apply_volume()

# --- public API --------------------------------------------------------------

# Play the SFX bound to `event` (a canonical event name). Cheap and safe to call
# every frame; unknown events and empty pools are no-ops. Honors a small
# per-event throttle so one-tick bursts don't stack.
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
	var stream: AudioStream = list[vi]
	var p := _pick_player()
	p.stream = stream
	p.play()

# Swap the looping music bed. `track` is a filename under audio/music/ (e.g.
# "ambient_bed.wav") or "" / null to stop. Loops automatically.
func set_music(track) -> void:
	if _music == null:
		return
	if track == null or String(track) == "":
		_music.stop()
		_music.stream = null
		return
	var path := MUSIC_DIR + String(track)
	if not ResourceLoader.exists(path):
		return
	var s := load(path) as AudioStream
	if s == null:
		return
	_set_loop(s)
	_music.stream = s
	if not _muted:
		_music.play()

# Master volume, 0.0..1.0 (linear), persisted. 1.0 == unity (0 dB).
func set_master_volume(linear: float) -> void:
	linear = clampf(linear, 0.0, 1.0)
	_master_db = linear_to_db(linear) if linear > 0.0 else -80.0
	_apply_volume()
	_save_prefs()

func master_volume() -> float:
	return db_to_linear(_master_db)

# Toggle global mute (Master bus). Returns the new muted state. Persisted.
func toggle_mute() -> bool:
	_muted = not _muted
	_apply_volume()
	if _muted:
		# Stop the bed so it doesn't resume mid-loop on unmute oddly; restart on unmute.
		if _music: _music.stop()
	elif _music and _music.stream != null:
		_music.play()
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

func _apply_volume() -> void:
	var master := AudioServer.get_bus_index("Master")
	if master >= 0:
		AudioServer.set_bus_mute(master, _muted)
		AudioServer.set_bus_volume_db(master, _master_db)

func _now() -> float:
	return float(Time.get_ticks_msec()) / 1000.0

# Ensure a stream loops (used for the ambient bed). Handles the common stream
# types defensively so a swapped-in format still loops.
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

func _save_prefs() -> void:
	var cf := ConfigFile.new()
	cf.load("user://profile.cfg")          # keep existing profile sections intact
	cf.set_value("audio", "muted", _muted)
	cf.set_value("audio", "master_db", _master_db)
	cf.save("user://profile.cfg")
