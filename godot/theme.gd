# Art-theme loader (autoload singleton `ArtTheme`). A "theme" is a drop-in folder
# under res://art/themes/<name>/ holding the same fixed filenames (the locked art
# contract), so swapping themes is just swapping a base path — no code changes.
# Press T in-game to cycle live.
extends Node

# Default first. Add new themes here; each must mirror the filename contract.
var themes := ["grimdark", "gaslamp_bulwark"]
var active := 0

func base() -> String:
	return "res://art/themes/%s/" % themes[active]

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
