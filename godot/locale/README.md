# Localization — Simplified Chinese (zh-CN)

The Simplified-Chinese localization package for **Standing Tank Defense** — **wired up and live in the game** (press **G** on Tank Select to toggle English ⇄ 简体中文; the choice persists in the profile).

## Contents

- `game_translations.csv` — a Godot-format translation table, **670 unique strings**:
  - enemy / boss / weapon / modifier **names** (transcreated puns, e.g. `The Hippocrate → 伪誓河马` — "false-oath hippo", keeping the Hippocrates + hypocrite gag),
  - weapon & modifier **descriptions** (flavor transcreated; mechanical tips keep all numbers/%/terms exact),
  - all **UI** strings (HUD, shop, results, lobby, skin-select, achievements/challenges, skin names like `Sir Toots-a-Lot → 放屁爵士`).

Format: `keys,en,zh_CN`. The **key is the English source string**, so `tr("English text")` works with no re-keying, and English stays the natural fallback for any locale.

## Wiring (live — how it's hooked up)

1. **Import:** Godot auto-imports the CSV (header begins with `keys`) into per-locale `.translation` resources (`game_translations.en.translation`, `game_translations.zh_CN.translation`).
2. **Registered** in `project.godot` under `[internationalization]` (`locale/translations` lists both `.translation` resources).
3. **Locale selection** at runtime: the **[G]** toggle in `skin_select.gd` calls `TranslationServer.set_locale(...)` and persists the choice via `Profile.set_locale_pref()` (`user://profile.cfg`).
4. **Display strings go through `tr()`**:
   - UI literals in the GDScript front-end (`main.gd`, `match.gd`, `lobby.gd`, `challenge_select.gd`, `skin_select.gd`).
   - **Rust-sourced content** (enemy/weapon/modifier names + descriptions arrive from the sim via the GDExtension) is wrapped at the GDScript display site, e.g. `tr(shop_name)`, `tr(desc_line)`. The sim core stays English-only and engine-independent; translation happens only at the render boundary.

## Notes

- Mechanical tip strings preserved numbers/operators verbatim (e.g. `<=600`, `+20%`), so balance text stays accurate in zh-CN.
- New display strings must be added to the CSV **keyed by their exact English text**, or they silently fall back to English.
- Traditional Chinese (zh-TW) can be added later as an extra column.
