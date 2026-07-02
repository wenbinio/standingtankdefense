# Art-theme loader (autoload singleton `ArtTheme`). A "theme" is a drop-in folder
# under res://art/themes/<name>/ holding the same fixed filenames (the locked art
# contract), so swapping themes is just swapping a base path — no code changes.
# Press T in-game to cycle live.
extends Node

# Default first. Add new themes here; each must mirror the filename contract.
var themes := ["grimdark", "gaslamp_bulwark"]
var active := 0

# --- Sprite manifest (THE single source of truth for entity art) --------------
# One entry per entity kind: theme-relative path + draw sizes. `size` is the
# single-arena (Main) blit size in px; `net_size` is the multi-arena net-view
# (Match) cell blit size. Art agents extend/retarget THESE tables — no other
# file may hardcode an entity sprite path or size.
# (Reserved for the art rebuild: entries may grow {frames, fps} for flipbooks.)
const ENEMY_MANIFEST: Array[Dictionary] = [
	{"path": "enemies/squeakzilla_rat.svg", "size": 74.0, "net_size": 20.0},   # 0 Squeakzilla
	{"path": "enemies/fanged_death.svg", "size": 74.0, "net_size": 20.0},     # 1 Fanged Death
	{"path": "enemies/boss_hippo.svg", "size": 230.0, "net_size": 56.0},      # 2 boss (The Hippocrate)
	{"path": "enemies/doomduck.svg", "size": 74.0, "net_size": 20.0},         # 3 Doomduck
	{"path": "enemies/bacon_warthog.svg", "size": 74.0, "net_size": 20.0},    # 4 Bacon
	{"path": "enemies/bandit_rider.svg", "size": 74.0, "net_size": 20.0},     # 5 Honk
	{"path": "enemies/bonk_golem.svg", "size": 118.0, "net_size": 30.0},      # 6 Bonk
	{"path": "enemies/noperope_cobra.svg", "size": 74.0, "net_size": 20.0},   # 7 Nope Rope
	{"path": "enemies/poisonspitter.svg", "size": 74.0, "net_size": 20.0},    # 8 Croak
	{"path": "enemies/firebreather.svg", "size": 74.0, "net_size": 20.0},     # 9 Spicy
	{"path": "enemies/icebreather.svg", "size": 74.0, "net_size": 20.0},      # 10 Popsicle
	{"path": "enemies/target_dummy.svg", "size": 74.0, "net_size": 20.0},     # 11 Dodo
]
const MINION_MANIFEST: Array[Dictionary] = [
	{"path": "minions/larvae.svg", "size": 64.0, "net_size": 18.0},           # 0 Larvae
	{"path": "minions/spores.svg", "size": 64.0, "net_size": 18.0},           # 1 Spores
]
const TANK_MANIFEST: Dictionary = {"path": "tank/player_tank.svg", "size": 124.0, "net_size": 40.0}
const PROJECTILE_MANIFEST: Dictionary = {"path": "projectiles/magic_orb.svg", "size": 26.0}
# Environment art shared by Main and Match (was duplicated as literals in both).
const ENV_MANIFEST: Dictionary = {"ground": "env/arena_ground.svg", "ring": "env/spawn_ring.svg"}

# Fixed, hand-picked rotation of skin ids for simulated peers (stable order,
# visibly varied). Shared by match.gd's peer spread and lobby.gd's seats.
const PEER_SKIN_ROTATION: Array[String] = [
	"deadeye", "spicy_meatball", "octo_blaster", "disco_doom",
	"tidal_terry", "bouncy_boi", "stone_broke", "franken_tank",
	"gore_hound", "chilly_willy", "sir_toots", "lord_spookington",
]

# --- Per-theme UI palette ----------------------------------------------------
# A theme owns not just its art but its UI chrome colors, so the HUD/shop/net
# labels recolor when you switch themes (and each player's net-view cell paints
# in THEIR theme). Render-only; never feeds the sim. Keys (use via `ui()` /
# `ui_of()`), every theme must define all of them:
#   accent       primary accent — gold values, equipped marks, your-cell border
#   accent_dim   muted accent (e.g. "+N/t" income, costs you can't afford)
#   text         primary readable text
#   text_dim     secondary / metadata text
#   header       section headers (ARSENAL, ROUND, theme pill)
#   hp           HP bar fill + HP number
#   danger       death / destroyed / critical
#   coin         currency/gold figure (often == accent)
#   panel_bg     panel/bar background (carries its own alpha)
#   panel_border panel outline
const THEME_UI := {
	# Grim cold steel, tarnished blood-gold, crimson accents.
	"grimdark": {
		"accent":       Color(0.85, 0.67, 0.30),
		"accent_dim":   Color(0.52, 0.42, 0.22),
		"text":         Color(0.87, 0.84, 0.79),
		"text_dim":     Color(0.56, 0.55, 0.58),
		"header":       Color(0.80, 0.34, 0.28),
		"hp":           Color(0.78, 0.24, 0.22),
		"danger":       Color(1.00, 0.32, 0.24),
		"coin":         Color(0.88, 0.70, 0.32),
		"panel_bg":     Color(0.055, 0.05, 0.06, 0.93),
		"panel_border": Color(0.30, 0.20, 0.18),
	},
	# Warm brass + gaslight amber over cool aether-blue.
	"gaslamp_bulwark": {
		"accent":       Color(0.94, 0.77, 0.37),
		"accent_dim":   Color(0.60, 0.50, 0.28),
		"text":         Color(0.90, 0.92, 0.96),
		"text_dim":     Color(0.60, 0.64, 0.72),
		"header":       Color(0.62, 0.82, 0.95),
		"hp":           Color(0.33, 0.72, 1.00),
		"danger":       Color(0.97, 0.52, 0.32),
		"coin":         Color(0.92, 0.78, 0.36),
		"panel_bg":     Color(0.06, 0.07, 0.10, 0.93),
		"panel_border": Color(0.24, 0.30, 0.36),
	},
}

# --- UI fonts with CJK fallback ----------------------------------------------
# The HUD/menus are custom-drawn with draw_string() using Barlow (Latin-only).
# To render Simplified-Chinese (and any non-Latin) glyphs, every UI Font needs a
# Noto Sans SC fallback chained in. Centralized + cached here so all draw sites
# (main.gd, match.gd, lobby.gd, challenge_select.gd, skin_select.gd) share one
# CJK-capable font instance per weight.
const _BARLOW_BOLD := "res://art/fonts/BarlowSemiCondensed-SemiBold.ttf"
const _BARLOW_BODY := "res://art/fonts/BarlowSemiCondensed-Medium.ttf"
const _NOTO_SC := "res://fonts/NotoSansSC.ttf"

var _noto: FontFile = null
var _ui_font_cache := {}  # bold:bool -> FontFile (Barlow + Noto fallback)

# The shared CJK fallback face, loaded once.
func _cjk() -> FontFile:
	if _noto == null and ResourceLoader.exists(_NOTO_SC):
		_noto = load(_NOTO_SC) as FontFile
	return _noto

# A UI font (bold=header weight, else body) that renders Latin via Barlow and
# falls back to Noto Sans SC for CJK glyphs. Cached per weight.
func ui_font(bold: bool) -> Font:
	if _ui_font_cache.has(bold):
		return _ui_font_cache[bold]
	var path := _BARLOW_BOLD if bold else _BARLOW_BODY
	var f: FontFile = (load(path) as FontFile) if ResourceLoader.exists(path) else null
	if f == null:
		# No Barlow on disk: use Noto alone if present, else engine fallback.
		var only_cjk := _cjk()
		var fb: Font = only_cjk if only_cjk else ThemeDB.fallback_font
		_ui_font_cache[bold] = fb
		return fb
	# load() returns a shared cached resource; duplicate so setting fallbacks
	# here doesn't mutate the same cached instance other load() callers see.
	f = f.duplicate() as FontFile
	var noto := _cjk()
	if noto:
		f.fallbacks = [noto]
	_ui_font_cache[bold] = f
	return f

func base() -> String:
	return theme_base(active)

# Base folder for a SPECIFIC theme index (net view / lobby load peers' themes by
# explicit path without mutating `active`).
func theme_base(theme_idx: int) -> String:
	return "res://art/themes/%s/" % themes[clampi(theme_idx, 0, themes.size() - 1)]

# A short uppercase tag for a theme index (per-cell/per-card theme pill).
func theme_tag(theme_idx: int) -> String:
	match themes[clampi(theme_idx, 0, themes.size() - 1)]:
		"grimdark":
			return "GRIMDARK"
		"gaslamp_bulwark":
			return "GASLAMP"
	return themes[theme_idx].to_upper()

# UI color for the ACTIVE theme (single-arena HUD/shop). Falls back to a neutral
# so a missing key never crashes a draw.
func ui(key: String) -> Color:
	return ui_of(active, key)

# UI color for a SPECIFIC theme index (net view paints each cell in its player's
# theme). `theme_idx` is an index into `themes`.
func ui_of(theme_idx: int, key: String) -> Color:
	var name: String = themes[clampi(theme_idx, 0, themes.size() - 1)]
	var pal: Dictionary = THEME_UI.get(name, {})
	return pal.get(key, Color(0.8, 0.8, 0.85))

# Guarded texture load for the ACTIVE theme: a missing file warns and returns a
# visible magenta placeholder instead of crashing the draw.
func tex(rel: String) -> Texture2D:
	return tex_of(active, rel)

# Guarded texture load for a SPECIFIC theme index.
func tex_of(theme_idx: int, rel: String) -> Texture2D:
	var p := theme_base(theme_idx) + rel
	if not ResourceLoader.exists(p):
		push_warning("ArtTheme: missing texture '%s' — using placeholder" % p)
		return _placeholder_tex()
	return load(p)

# The loud "art is missing" texture: solid magenta, cached.
var _placeholder: Texture2D = null
func _placeholder_tex() -> Texture2D:
	if _placeholder == null:
		var img := Image.create(32, 32, false, Image.FORMAT_RGBA8)
		img.fill(Color(1.0, 0.0, 1.0, 1.0))
		_placeholder = ImageTexture.create_from_image(img)
	return _placeholder

# --- Manifest-driven texture sets ---------------------------------------------
# Enemy textures for a theme, in enemy-kind order (indexable by sim kind).
func enemy_textures_of(theme_idx: int) -> Array:
	var out: Array = []
	for e in ENEMY_MANIFEST:
		out.append(tex_of(theme_idx, e["path"]))
	return out

func enemy_textures() -> Array:
	return enemy_textures_of(active)

# Minion textures for a theme, in minion-kind order.
func minion_textures_of(theme_idx: int) -> Array:
	var out: Array = []
	for m in MINION_MANIFEST:
		out.append(tex_of(theme_idx, m["path"]))
	return out

func minion_textures() -> Array:
	return minion_textures_of(active)

# Draw size (px) for an enemy kind; `net` picks the multi-arena cell size.
func enemy_draw_size(kind: int, net := false) -> float:
	var e: Dictionary = ENEMY_MANIFEST[kind] if kind >= 0 and kind < ENEMY_MANIFEST.size() else ENEMY_MANIFEST[0]
	return e["net_size"] if net else e["size"]

func minion_draw_size(kind: int, net := false) -> float:
	var m: Dictionary = MINION_MANIFEST[kind] if kind >= 0 and kind < MINION_MANIFEST.size() else MINION_MANIFEST[0]
	return m["net_size"] if net else m["size"]

func tank_draw_size(net := false) -> float:
	return TANK_MANIFEST["net_size"] if net else TANK_MANIFEST["size"]

func projectile_draw_size() -> float:
	return PROJECTILE_MANIFEST["size"]

# The player tank, honoring the profile's selected skin. Skins live at
# tank/skins/<id>.svg per theme; "" (and any skin missing from the current
# theme) falls back to the theme's default player_tank.svg.
func tank_tex() -> Texture2D:
	return tank_tex_for(active, Profile.selected)

# Tank texture for an arbitrary (theme, skin) pair by the same resolution rule —
# the ONE copy of the lookup that match.gd / lobby.gd / skin_select.gd /
# challenge_select.gd used to each reimplement. Never mutates `active`.
func tank_tex_for(theme_idx: int, skin_id: String) -> Texture2D:
	var f: String = Profile.skin_def(skin_id).file
	if f != "":
		var p := theme_base(theme_idx) + "tank/" + f
		if ResourceLoader.exists(p):
			return load(p)
	return tex_of(theme_idx, TANK_MANIFEST["path"])

func theme_name() -> String:
	return themes[active]

func cycle() -> void:
	active = (active + 1) % themes.size()
