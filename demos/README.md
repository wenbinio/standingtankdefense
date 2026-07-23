# Demos

Prebuilt, ready-to-run binaries for trying **Standing Tank Defense** without building from source.

## `StandingTankDefense-win64.zip` — Windows (x86-64)

Built from commit `80bd39b`. Contents: `StandingTankDefense.exe` + `standing_tank_gdext.dll` + `README.txt`.

**Run:**
1. Download and unzip **all files together** (keep the `.dll` beside the `.exe`).
2. Double-click `StandingTankDefense.exe`. If SmartScreen warns (unsigned): *More info → Run anyway*.

**Controls:** `[G]` language (EN / 简体中文) · `[N]` mute · `[T]` art theme · `[M]` 8-arena netcode demo · `[Esc]` menu

> **This zip is stale — it predates the current game.** It was built before the [`09`](../docs/09-rebuild-plan.md)
> presentation rebuild, the source-fidelity program (the restored 15-minute arc + re-anchored catalog),
> and the fun/retention wave (records/near-miss, arsenal panel, DPS meter, SP difficulty + boss-kill victory,
> the human-playable net seat). It does **not** reflect any of that. To play the current game, **build from
> source** — see [`../TUTORIAL.md`](../TUTORIAL.md).
>
> This is a convenience copy and will lag `main` until refreshed. The canonical, reproducible
> build is produced by `.github/workflows/windows-release.yml` — tag `vX.Y.Z` to publish a GitHub Release.
