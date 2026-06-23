# Localization — Simplified Chinese (zh-CN)

A ready-to-wire Simplified-Chinese localization package for **Standing Tank Defense**.

## Contents

- `game_translations.csv` — a Godot-format translation table, **670 unique strings**:
  - enemy / boss / weapon / modifier **names** (transcreated puns, e.g. `The Hippocrate → 伪誓河马` — "false-oath hippo", keeping the Hippocrates + hypocrite gag),
  - weapon & modifier **descriptions** (flavor transcreated; mechanical tips keep all numbers/%/terms exact),
  - all **UI** strings (HUD, shop, results, lobby, skin-select, achievements/challenges, skin names like `Sir Toots-a-Lot → 放屁爵士`).

Format: `keys,en,zh_CN`. The **key is the English source string**, so `tr("English text")` works with no re-keying, and English stays the natural fallback for any locale.

## Wiring (integration step — not yet applied to the game)

1. **Import:** Godot auto-imports a CSV whose header begins with `keys` into per-locale `.translation` resources (`game_translations.en.translation`, `game_translations.zh_CN.translation`).
2. **Register** in `project.godot`:
   ```
   [internationalization]
   locale/translations=PackedStringArray("res://locale/game_translations.zh_CN.translation","res://locale/game_translations.en.translation")
   ```
3. **Select locale** at runtime, e.g. a language toggle in `SkinSelect`:
   ```gdscript
   TranslationServer.set_locale("zh_CN")  # or "en"
   ```
4. **Wrap display strings in `tr()`**:
   - UI literals in the GDScript front-end (`main.gd`, `match.gd`, `lobby.gd`, `challenge_select.gd`, `skin_select.gd`, `profile.gd`).
   - **Rust-sourced content** (enemy/weapon/modifier names + descriptions arrive from the sim via the GDExtension) — wrap them at the GDScript display site, e.g. `tr(shop_name)`, `tr(desc_line)`. The sim core stays English-only and engine-independent; translation happens only at the render boundary.

## Notes

- Scope: this package is the translation **data** + wiring guide. The `tr()` plumbing and a language selector are the follow-on integration task.
- Mechanical tip strings preserved numbers/operators verbatim (e.g. `<=600`, `+20%`), so balance text stays accurate in zh-CN.
- Traditional Chinese (zh-TW) can be added later as an extra column.
