# `steam_appid.txt` — placeholder

`steam_appid.txt` must contain **only** the raw numeric App ID on the first line
(no comments — `SteamAPI_Init` parses the integer directly, and a comment line
would break init). So the explanation lives here instead of inside the file.

- **Current value: `480`** — Spacewar, Valve's standard development placeholder
  App ID. It lets `SteamAPI_Init` succeed against any running Steam client during
  development.
- **MUST be replaced** by the registered **free App ID** for Standing Tank Defense
  before any public/Steam build, on the partner site (`docs/07 §7.7`).

## Which file the shipping build uses

`SteamAPI_Init` reads `steam_appid.txt` from the **current working directory of
the running game binary**. For the shipping build that is the **Godot export
directory** — i.e. the copy next to the game executable, which corresponds to
**`godot/steam_appid.txt`** in this repo (it ships alongside the Godot project /
exported binary).

The **repo-root** `steam_appid.txt` exists for convenience when running tools or
a headless dedicated director from the repo root during development. Both carry
the same placeholder `480`; only the Godot-export copy is shipped.

> Note: `steam_appid.txt` is a **dev/test convenience**. A released Steam build
> gets its App ID from Steam itself (the file is normally absent from the final
> depot); keep it for local runs, drop or ignore it in the published depot.
