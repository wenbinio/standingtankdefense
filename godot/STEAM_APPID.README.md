# `steam_appid.txt` — testing App ID (480 / Spacewar)

`steam_appid.txt` must contain **only** the raw numeric App ID on the first line
(no comments — `SteamAPI_Init` parses the integer directly, and a comment line
would break init). So the explanation lives here instead of inside the file.

- **Current value: `480`** — Spacewar, Valve's public **test** App ID. This is the
  **intentional development/testing App ID**: it lets `SteamAPI_Init` succeed
  against any running Steam client without a registered app, so the netcode and
  the `adapters/steam-transport` adapter can be brought up and tested now.
- The adapter pins this in-process via `SteamBootstrap::init()` →
  `init_app(TEST_APP_ID = 480)`, so testing does **not** depend on this file being
  in the binary's CWD; the file is kept for tools/SDK that read it directly.
- **For a release build**, switch to the registered **free App ID** for Standing
  Tank Defense (partner site, `docs/07 §7.7`) by calling
  `SteamBootstrap::init_app(real_app_id)` and updating this file.

## Which file the shipping build uses

`SteamAPI_Init` reads `steam_appid.txt` from the **current working directory of
the running game binary**. For the shipping build that is the **Godot export
directory** — i.e. the copy next to the game executable, which corresponds to
**`godot/steam_appid.txt`** in this repo (it ships alongside the Godot project /
exported binary).

The **repo-root** `steam_appid.txt` exists for convenience when running tools or
a headless dedicated director from the repo root during development. Both carry
the same testing App ID `480`; only the Godot-export copy is shipped.

> Note: `steam_appid.txt` is a **dev/test convenience**. A released Steam build
> gets its App ID from Steam itself (the file is normally absent from the final
> depot); keep it for local runs, drop or ignore it in the published depot.
