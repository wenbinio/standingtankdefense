# Art-theme loader (autoload singleton `ArtTheme`). A "theme" is a drop-in folder
# under res://art/themes/<name>/ holding the same fixed filenames (the locked art
# contract), so swapping themes is just swapping a base path — no code changes.
# Press T in-game to cycle live.
extends Node

# Default first. Add new themes here; each must mirror the filename contract.
var themes := ["grimdark", "gaslamp_bulwark"]
var active := 0

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
	# here doesn't mutate the same instance referenced by ui_theme.tres et al.
	f = f.duplicate() as FontFile
	var noto := _cjk()
	if noto:
		f.fallbacks = [noto]
	_ui_font_cache[bold] = f
	return f

func base() -> String:
	return "res://art/themes/%s/" % themes[active]

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

func tex(rel: String) -> Texture2D:
	return load(base() + rel)

# The player tank, honoring the profile's selected skin. Skins live at
# tank/skins/<id>.svg per theme; "" (and any skin missing from the current
# theme) falls back to the theme's default player_tank.svg.
func tank_tex() -> Texture2D:
	var f: String = Profile.skin_def(Profile.selected).file
	if f != "":
		var p := base() + "tank/" + f
		if ResourceLoader.exists(p):
			return load(p)
	return tex("tank/player_tank.svg")

func theme_name() -> String:
	return themes[active]

func cycle() -> void:
	active = (active + 1) % themes.size()
