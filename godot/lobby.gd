# Multiplayer LOBBY — the front door to a networked match (docs/07 §7.3). Drives
# the host-authoritative session lifecycle through the `StLobby` gdext node (the
# deterministic `net::lobby` model): host opens a lobby → players join → everyone
# readies → host starts → the match launches.
#
# AUTHORITY SPLIT (do not blur this):
#   • Membership, ready flags, and phase are OWNED by StLobby. This script only
#     READS them (member_count/member_peer/member_ready/is_host_member/phase/…)
#     and drives transitions via host/add_member/leave/set_ready/try_start. None
#     of that lifecycle logic is reimplemented here.
#   • COSMETICS (each member's theme + skin + display name) are engine-side and
#     ARE assigned here, keyed by peer id, mirroring match.gd's approach. Player 0
#     (you) gets your live theme + selected skin; peers get a deterministic varied
#     spread of themes × skins so the lobby reads like real people made choices.
extends Node2D

# Max seats StLobby allows (host peer 0 + peers 1..7).
const MAX_PEERS := 8
# Game-speed labels/multipliers by StLobby.game_speed() code (0 Normal ×1.0 ·
# 1 Fast ×1.5 · 2 Faster ×2.0 · 3 Hyper ×3.0 = 30/45/60/90 ticks/s). Labels
# are translation-table keys. The AUTHORITY for the value is StLobby (the
# deterministic net::lobby ruleset — host-only, locked once started); this
# script only cycles and displays it.
const SPEED_NAMES := ["Normal", "Fast", "Faster", "Hyper"]
const SPEED_MULTS := ["1.0", "1.5", "2.0", "3.0"]
# After a simulated peer joins, auto-ready it this many seconds later so the host
# can actually reach phase Ready without a human on the other end. A "ready all"
# key (R) also exists for impatience.
const PEER_AUTO_READY_DELAY := 0.9

var lobby                       # StLobby instance (host-authoritative model)
var font: Font

# Per-peer cosmetics, keyed by peer id → {"theme": int, "skin": String,
# "name": String}. Peer 0 tracks YOUR live cosmetics; peers get a fixed varied
# spread. Resolved tank textures are cached per peer in `_tank_tex`.
var _cos := {}
var _tank_tex := {}             # peer id → Texture2D (resolved by explicit path)

# Pending auto-ready timers: peer id → seconds remaining. A peer joins un-ready,
# then flips ready when its timer elapses (simulating a remote player clicking
# "ready"). Removed peers drop their pending timer.
var _pending_ready := {}

# Flavor names for simulated peers (cosmetic only). Their skin spread comes
# from the shared ArtTheme.PEER_SKIN_ROTATION (same roster match.gd uses).
const PEER_NAMES := [
	"Ironhide", "Boomstick", "Gunny", "Discoteq",
	"Wavebreak", "Pinball", "Brokey", "Frank",
	"Houndog", "Frosty", "Sir Toots", "Spooks",
]

func _ready() -> void:
	font = ArtTheme.ui_font(false)   # Barlow + Noto SC fallback (renders CJK)
	# Menu music bed (render-only; set_music dedupes across menu scenes).
	Audio.set_music("menu_theme.wav")
	randomize()
	# Open a lobby with us as host (peer 0): seated + ready, phase Filling.
	lobby = StLobby.host()
	_assign_cos_for_peer(0)
	# Seat a few simulated players so the lifecycle is demonstrable out of the box.
	_add_peer()
	_add_peer()
	queue_redraw()

# --- Cosmetics (engine-only; keyed by peer id, never feeds the sim) -----------
# Peer 0 = YOU: live ArtTheme.active + Profile.selected, name "YOU". Peers cycle
# BOTH themes (offset from yours so the first peer already contrasts) across the
# stable skin rotation — exactly match.gd's deterministic varied spread, but
# keyed by peer id (not array index) so it survives joins/leaves.
func _assign_cos_for_peer(peer: int) -> void:
	if peer == 0:
		_cos[0] = {"theme": ArtTheme.active, "skin": Profile.selected, "name": "YOU"}
	else:
		var skins: Array = ArtTheme.PEER_SKIN_ROTATION
		var theme_count: int = ArtTheme.themes.size()
		var theme_idx: int = (ArtTheme.active + peer) % theme_count
		var skin_id: String = skins[(peer - 1) % skins.size()]
		var nm: String = PEER_NAMES[(peer - 1) % PEER_NAMES.size()]
		_cos[peer] = {"theme": theme_idx, "skin": skin_id, "name": nm}
	_tank_tex[peer] = ArtTheme.tank_tex_for(_cos[peer]["theme"], _cos[peer]["skin"])

# Re-derive YOUR cosmetics AND the whole peer spread after a theme cycle, so the
# offset-from-you rotation stays consistent (mirrors match.gd's re-assign on T).
func _refresh_all_cos() -> void:
	for peer in _cos.keys():
		_assign_cos_for_peer(peer)

# (Theme-path / tank-texture / theme-tag helpers now live on ArtTheme:
#  theme_base / tex_of / tank_tex_for / theme_tag — one copy for all screens.)

# --- Lobby mutations (drive StLobby; cosmetics follow the returned peer id) ----
# Seat the next free peer via StLobby, assign its cosmetics, and queue its auto-
# ready. Returns the new peer id, or -1 if the lobby is full.
func _add_peer() -> int:
	if lobby == null:
		return -1
	var peer: int = lobby.add_member()
	if peer < 0:
		return -1
	_assign_cos_for_peer(peer)
	_pending_ready[peer] = PEER_AUTO_READY_DELAY
	queue_redraw()
	return peer

# Remove the highest-numbered simulated peer (never peer 0 — you can't kick
# yourself; Esc leaves the whole lobby). Drops its cosmetics + pending timer.
func _remove_last_peer() -> void:
	if lobby == null:
		return
	var top := -1
	for i in lobby.member_count():
		var p: int = lobby.member_peer(i)
		if p != 0 and p > top:
			top = p
	if top < 0:
		return
	if lobby.leave(top):
		_cos.erase(top)
		_tank_tex.erase(top)
		_pending_ready.erase(top)
		queue_redraw()

# Toggle YOUR (peer 0) ready flag through StLobby.
func _toggle_my_ready() -> void:
	if lobby == null:
		return
	var mine := _my_ready()
	lobby.set_ready(0, not mine)
	queue_redraw()

# Force-ready every seated peer immediately (the "ready all" convenience).
func _ready_all() -> void:
	if lobby == null:
		return
	for i in lobby.member_count():
		lobby.set_ready(lobby.member_peer(i), true)
	_pending_ready.clear()
	queue_redraw()

# Is peer 0 (you) currently ready? Read from StLobby, never tracked locally.
func _my_ready() -> bool:
	for i in lobby.member_count():
		if lobby.member_peer(i) == 0:
			return lobby.member_ready(i)
	return false

# --- START (host only; gated on phase Ready == 1) -----------------------------
# Surfaced reason text after a try_start attempt (blank when fine / not attempted).
var _start_msg := ""
var _start_msg_t := 0.0

func _attempt_start() -> void:
	if lobby == null:
		return
	if lobby.phase() != 1:
		# Not in Ready phase: tell the host why (mirror try_start's reasons).
		if lobby.member_count() < 2:
			_flash(tr("Need at least 1 other player to start."))
		else:
			_flash(tr("Not everyone is ready yet. [R] readies all."))
		return
	var code: int = lobby.try_start(randi())
	if code == 0:
		Audio.play(&"ui_click")
		# Host-authoritative plan: hand the seed + PEER count to the match. We store
		# peers-only (total members minus the host) so Match's `lobby_players + 1`
		# reconstitutes the full host + peers roster. plan_player_count() is the
		# authoritative total; >= 2 here since try_start required ≥1 peer.
		Session.lobby_seed = lobby.plan_seed()
		Session.lobby_players = maxi(lobby.plan_player_count() - 1, 1)
		# The host-set pace rides the same hand-off: Match passes it verbatim
		# to StMatch.new_match_at_speed (cadence only, fixed for the match).
		Session.lobby_speed = lobby.game_speed()
		get_tree().change_scene_to_file("res://Match.tscn")
	elif code == -1:
		_flash(tr("Start rejected: not all players ready."))
	elif code == -2:
		_flash(tr("Start rejected: not enough players."))
	elif code == -3:
		_flash(tr("Start rejected: match already started."))
	else:
		_flash(tr("Start rejected (code %d).") % code)

func _flash(msg: String) -> void:
	Audio.play(&"ui_deny")     # every flash is a rejection/hint — voice it once
	_start_msg = msg
	_start_msg_t = 3.0
	queue_redraw()

# --- game speed (host control; [F] cycles Normal → Fast → Faster → Hyper) ------
# The rule lives in StLobby.set_game_speed (host-only, pre-start); this only
# drives it and surfaces the rejections. Non-hosts see the read-only readout in
# the footer (in this local demo YOU are always the host, peer 0).
func _cycle_speed() -> void:
	if lobby == null:
		return
	if lobby.host_peer() != 0:
		_flash(tr("Only the host can change game speed."))
		return
	var next: int = posmod(int(lobby.game_speed()) + 1, SPEED_NAMES.size())
	if not lobby.set_game_speed(next):
		_flash(tr("Game speed is locked once the match has started."))
	queue_redraw()

# --- input (InputMap actions; bindings in project.godot [input]) ---------------
func _unhandled_input(e: InputEvent) -> void:
	if not (e is InputEventKey or e is InputEventJoypadButton):
		return
	if e.is_action_pressed(&"ui_ready_toggle"):
		Audio.play(&"ui_click")
		_toggle_my_ready()
	elif e.is_action_pressed(&"ui_lobby_add"):
		Audio.play(&"ui_click")
		_add_peer()
	elif e.is_action_pressed(&"ui_lobby_remove"):
		Audio.play(&"ui_back")
		_remove_last_peer()
	elif e.is_action_pressed(&"ui_ready_all"):
		Audio.play(&"ui_click")
		_ready_all()
	elif e.is_action_pressed(&"ui_lobby_start"):
		_attempt_start()           # voices ui_click on success / ui_deny via _flash
	elif e.is_action_pressed(&"ui_speed_cycle"):
		Audio.play(&"ui_move")
		_cycle_speed()
	elif e.is_action_pressed(&"ui_theme_cycle"):
		ArtTheme.cycle()
		_refresh_all_cos()
		Audio.play(&"ui_click")
		queue_redraw()
	elif e.is_action_pressed(&"ui_back"):
		Audio.play(&"ui_back")
		get_tree().change_scene_to_file("res://SkinSelect.tscn")

# --- per-frame: advance peer auto-ready timers --------------------------------
func _process(delta: float) -> void:
	if lobby == null:
		return
	var fired := false
	for peer in _pending_ready.keys():
		_pending_ready[peer] -= delta
		if _pending_ready[peer] <= 0.0:
			lobby.set_ready(peer, true)
			fired = true
	if fired:
		for peer in _pending_ready.keys().duplicate():
			if _pending_ready[peer] <= 0.0:
				_pending_ready.erase(peer)
	if _start_msg_t > 0.0:
		_start_msg_t -= delta
		if _start_msg_t <= 0.0:
			_start_msg = ""
	# Redraw while anything is animating (pending timers or a fading message).
	if fired or not _pending_ready.is_empty() or _start_msg_t > 0.0:
		queue_redraw()

# --- render -------------------------------------------------------------------
func _draw() -> void:
	var vp: Vector2 = get_viewport_rect().size
	# Dim backdrop in YOUR active theme's panel bg, so the lobby matches the game.
	draw_rect(Rect2(Vector2.ZERO, vp), Color(0.035, 0.04, 0.055))

	if lobby == null:
		draw_string(font, Vector2(36, 60), tr("Lobby unavailable (StLobby not loaded)."),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 20, Color(0.85, 0.45, 0.4))
		return

	var my_accent: Color = ArtTheme.ui("accent")
	var my_text: Color = ArtTheme.ui("text")
	var my_dim: Color = ArtTheme.ui("text_dim")

	# --- header: phase + counts (all read from StLobby) ---
	var n: int = lobby.member_count()
	var ready_n := 0
	for i in n:
		if lobby.member_ready(i):
			ready_n += 1
	var phase: int = lobby.phase()
	var phase_txt: String = [tr("FILLING"), tr("READY"), tr("STARTED")][phase] if phase >= 0 and phase < 3 else "?"
	draw_string(font, Vector2(36, 48), tr("MULTIPLAYER LOBBY"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 30, Color(0.9, 0.93, 0.98))
	draw_string(font, Vector2(420, 48),
		tr("host peer %d  ·  %d / %d seated  ·  %d / %d ready  ·  phase %s")
		% [lobby.host_peer(), n, MAX_PEERS, ready_n, n, phase_txt],
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16, my_dim)

	# --- member cards: a responsive grid, each painted in its OWN theme ---
	var cols: int = 4 if n > 3 else maxi(n, 1)
	var rows: int = int(ceil(float(n) / cols))
	var mx := 36.0
	var top := 84.0
	var bottom := 132.0
	var pad := 14.0
	var cw := (vp.x - 2.0 * mx - pad * (cols - 1)) / cols
	var ch := (vp.y - top - bottom - pad * (rows - 1)) / maxi(rows, 1)
	for i in n:
		var col := i % cols
		var row := i / cols
		var r := Rect2(mx + col * (cw + pad), top + row * (ch + pad), cw, ch)
		_draw_member_card(i, r)

	# --- footer: START button + control hints ---
	_draw_footer(vp, phase, n, ready_n, my_accent, my_text, my_dim)

# One member card. Identity (peer/host/ready) comes from StLobby by index `i`;
# the cosmetics come from `_cos`/`_tank_tex` keyed by that index's peer id. The
# whole card paints in THAT member's theme palette via ArtTheme.ui_of.
func _draw_member_card(i: int, r: Rect2) -> void:
	var peer: int = lobby.member_peer(i)
	var is_ready: bool = lobby.member_ready(i)
	var is_host: bool = lobby.is_host_member(i)
	var is_you: bool = peer == 0

	var cos: Dictionary = _cos.get(peer, {"theme": 0, "skin": "ol_reliable", "name": "P%d" % peer})
	var theme_idx: int = cos["theme"]

	# Per-card palette (this member's theme).
	var c_accent: Color = ArtTheme.ui_of(theme_idx, "accent")
	var c_text: Color = ArtTheme.ui_of(theme_idx, "text")
	var c_dim: Color = ArtTheme.ui_of(theme_idx, "text_dim")
	var c_header: Color = ArtTheme.ui_of(theme_idx, "header")
	var c_border: Color = ArtTheme.ui_of(theme_idx, "panel_border")
	var c_bg: Color = ArtTheme.ui_of(theme_idx, "panel_bg")
	# Gold accent for YOU regardless of your theme, so your card always reads gold.
	var gold := Color(0.95, 0.78, 0.36)

	# Card body in this theme's panel bg.
	draw_rect(r, c_bg)

	# Tank thumbnail (this member's theme+skin art).
	var tex: Texture2D = _tank_tex.get(peer)
	if tex:
		var ts := minf(r.size.x * 0.62, r.size.y * 0.46)
		var tp := r.position + Vector2((r.size.x - ts) * 0.5, 16.0)
		draw_texture_rect(tex, Rect2(tp, Vector2(ts, ts)), false,
			Color(1.15, 1.18, 1.3) if is_ready else Color(0.7, 0.7, 0.76))

	# Name line (YOUR name in gold; peers in their theme text). Peer 0 is "YOU"
	# (translatable); simulated-peer flavor handles stay as-is.
	var nm: String = tr(cos["name"])
	draw_string(font, r.position + Vector2(10, r.size.y - 64), nm,
		HORIZONTAL_ALIGNMENT_LEFT, r.size.x - 20, 18, gold if is_you else c_text)

	# Peer + role line. "P%d" is scaffolding; the HOST tag is translatable.
	var role := "P%d" % (peer + 1)
	if is_host:
		role += "  ·  " + tr("HOST")
	draw_string(font, r.position + Vector2(10, r.size.y - 44), role,
		HORIZONTAL_ALIGNMENT_LEFT, r.size.x - 20, 13, c_dim)

	# READY check/✗ — green check when ready, dim ✗ when not.
	var rmark := tr("✓ READY") if is_ready else tr("✗ not ready")
	var rcol := Color(0.42, 0.85, 0.55) if is_ready else Color(0.78, 0.45, 0.42)
	draw_string(font, r.position + Vector2(10, r.size.y - 22), rmark,
		HORIZONTAL_ALIGNMENT_LEFT, r.size.x - 20, 14, rcol)

	# THEME pill, top-right — fill from accent (dimmed), border + text from header.
	var tag := tr(ArtTheme.theme_tag(theme_idx))
	var pill_fs := 11
	var tag_w: float = font.get_string_size(tag, HORIZONTAL_ALIGNMENT_LEFT, -1, pill_fs).x + 14.0
	var pill := Rect2(r.position.x + r.size.x - tag_w - 8.0, r.position.y + 8.0, tag_w, 18.0)
	draw_rect(pill, Color(c_accent.r, c_accent.g, c_accent.b, 0.22))
	draw_rect(pill, c_header, false, 1.0)
	draw_string(font, Vector2(pill.position.x + 7.0, pill.position.y + 13.0), tag,
		HORIZONTAL_ALIGNMENT_LEFT, -1, pill_fs, c_header)

	# "YOU" badge, top-left, in gold.
	if is_you:
		draw_string(font, r.position + Vector2(10, 22), tr("★ YOU"),
			HORIZONTAL_ALIGNMENT_LEFT, -1, 14, gold)

	# Card border — YOUR card gets a thicker gold double-frame; peers get their
	# own theme's panel_border (brightened to accent when that peer is ready).
	if is_you:
		draw_rect(r, gold, false, 4.0)
		draw_rect(Rect2(r.position + Vector2(3, 3), r.size - Vector2(6, 6)),
			Color(gold.r, gold.g, gold.b, 0.35), false, 1.5)
	else:
		draw_rect(r, c_accent if is_ready else c_border, false, 2.0)

func _draw_footer(vp: Vector2, phase: int, n: int, ready_n: int,
		my_accent: Color, my_text: Color, my_dim: Color) -> void:
	var fy := vp.y - 96.0
	# START button — host only; ENABLED only at phase Ready (1) with ≥1 peer.
	var can_start: bool = phase == 1
	var btn := Rect2(36, fy, 260, 44)
	var btn_fill := Color(0.16, 0.42, 0.22) if can_start else Color(0.13, 0.13, 0.16)
	var btn_text := Color(0.6, 0.95, 0.66) if can_start else Color(0.5, 0.5, 0.56)
	draw_rect(btn, btn_fill)
	draw_rect(btn, my_accent if can_start else Color(0.3, 0.3, 0.34), false, 2.0)
	var label := tr("▶ START MATCH  [Enter]") if can_start else tr("START  (all must ready)")
	draw_string(font, Vector2(btn.position.x + 14, btn.position.y + 29), label,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 16, btn_text)

	# Your-ready toggle state, beside the button. "[Space] toggle" is a control
	# hint suffix; the ready state itself is in the translation table.
	var youready := _my_ready()
	draw_string(font, Vector2(316, fy + 29),
		(tr("YOU: ✓ ready") if youready else tr("YOU: ✗ not ready")) + "   " + tr("[Space] toggle"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15,
		Color(0.42, 0.85, 0.55) if youready else my_text)

	# Host-set game speed readout, right-aligned on the button row: name +
	# multiplier + the driver cadence, all read live from StLobby. Hosts get
	# the [F] hint; non-hosts see it read-only. Measured width (CJK-safe).
	var sp: int = clampi(int(lobby.game_speed()), 0, SPEED_NAMES.size() - 1)
	var sp_fmt := tr("SPEED: %s  ·  ×%s  ·  %d ticks/s   [F] change") \
		if lobby.host_peer() == 0 else tr("SPEED: %s  ·  ×%s  ·  %d ticks/s")
	var sp_label: String = sp_fmt % [tr(SPEED_NAMES[sp]), SPEED_MULTS[sp], lobby.ticks_per_second()]
	var sp_w: float = font.get_string_size(sp_label, HORIZONTAL_ALIGNMENT_LEFT, -1, 15).x
	draw_string(font, Vector2(maxf(vp.x - 36.0 - sp_w, 20.0), fy + 29), sp_label,
		HORIZONTAL_ALIGNMENT_LEFT, -1, 15, my_accent if sp > 0 else my_dim)

	# Flash message (rejection reason / hint) — already tr()'d at its source.
	if _start_msg != "":
		draw_string(font, Vector2(36, fy - 10), _start_msg,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 15, Color(0.95, 0.7, 0.4))

	# Control hint line.
	draw_string(font, Vector2(36, vp.y - 22),
		tr("[Space] your ready   [A] add player   [X] remove player   [R] ready all   [Enter] start   [T] theme   [Esc] back"),
		HORIZONTAL_ALIGNMENT_LEFT, -1, 14, my_dim)
