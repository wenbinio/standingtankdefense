# 01 — Source Analysis: Warcraft III "Tower Survivors"

What we learned by **opening the actual map** (`TowerSurvivors v1.58.w3x`, author **Nethalythic**) and what it means for Standing Tank Defense. The raw extracted catalog (every weapon, upgrade, and unit, with numbers) lives in [`appendix-A-map-extraction.md`](appendix-A-map-extraction.md); this doc is the interpretation.

## 1.1 How we got the data

- The map is a **MoPaQ (MPQ) archive**. The internal `(listfile)` is encrypted, but the well-known WC3 filenames are readable directly.
- We extracted `war3map.wts` (the trigger-string table — **3,770 strings**), `war3map.w3i` (map info), `war3mapMisc.txt` (gameplay constants), and the object-data tables (`war3map.w3u`/`w3a`/`w3t`/…). The full in-game tooltips — which contain exact damage, cooldown, range, cost, and rarity — live in the `.wts`, so we have authoritative numbers without needing to decompile the triggers.
- The compiled script (`war3map.j`/`.lua`) was the one thing we could **not** pull cleanly (the script block is protected/encrypted in this map — consistent with the author distributing an anti-cheat build via Discord; see the in-map note "*Make sure you download your map from the Discord if you don't want to risk playing cheat versions*"). The triggers govern **flow** (wave timing, economy tick, last-man-standing arbitration). We reconstruct that flow from the changelog text, tooltips, and game-speed/credits strings, and label anything still uncertain *(inferred)*.

## 1.2 What the map is — confirmed

A **roguelike tower defense** for **2–8 players**, by Nethalythic, explicitly in the *Vampire Survivors / Halls of Torment* lineage. The player owns **one immobile "Survivor's Tower"** that auto-attacks; the only verbs are **buy**, **upgrade**, **reroll**, and one manual ability (**Clear**). Confirmed structural facts from the map:

| Fact | Value | Where confirmed |
| --- | --- | --- |
| Players | 2–8 | `war3map.wts` STRING 2 (`2-8`) |
| Author | Nethalythic | STRING 4 |
| Match length | **~15 minutes**, then boss | Challenge text: "*Survive 15 minutes … then kill Samwise*" |
| Boss | **Samwise** (giant kobold); fixed HP/damage, does **not** scale | Changelog STRING 6871 |
| Boss kill rule | Only the tower's **Clear** ability damages it | Carried mechanic; shop "flees in fear of Samwise's impending arrival" (STRING 7829) |
| Enemy scaling | Steps up at **10 min** and **15 min** ("to bring the game to a swift end") | Changelog STRING 6871 |
| Shop cadence | New shop **every round (every 30 s)** | Many upgrades: "*triggers every round, when a new shop is made available*" |
| Reroll | Starts with **5** rerolls; reroll cost rises each use; Free Reroll exists | STRING 1192 / 1406 / 6871 |
| Game speed | Normal/Fast/Faster/Hyper, **only Player 1 (Red)** can set it | STRING 7876 |

> Earlier drafts of this spec guessed "20 min / 40 rounds." The shipped v1.58 is **~15 min of 30-second rounds + Samwise**. We use that.

## 1.3 Combat model — confirmed from tooltips

This is the part the `.wts` nails down precisely (see Appendix A for the full 79-weapon table).

**Damage types form an armor matrix.** Five base types — **Normal, Piercing, Magic, Siege, Chaos** — each strong/weak against armor classes per `war3mapMisc.txt` (e.g. `DamageBonusPierce=2.00,1.00,…` → Piercing does 2× vs the first armor class). On top of the base type, weapons carry **status flavors**: **Poison, Frost, Fire, Spikes**. Crucially, every damage tooltip states: *"Different damage increases are multiplicative with each other."* — so the stacking model is **multiplicative across distinct sources, additive within a source.**

**Attack types** (the projectile/targeting behavior):
- **Single Target** — one target.
- **Splash (R)** — AoE radius R on impact.
- **Bounce (N Targets)** — chains to N enemies.
- **Barrage (N Targets)** — fires at N enemies at once.
- **Wave (+R Range)** — a sweeping wave, often "rotating clockwise/counterclockwise."
- **Area (R)** / **Area (Enemies In Range)** — persistent area damage.

**Per-weapon stats** are `{damage, DPS, attack cooldown, range, ability}`. Notable patterns we must preserve:
- **Range is tiered**: 300 / 600 / 900 / 1200. Upgrades target tiers ("+25% Damage for 300 and 600 Range Weapons").
- **Cooldown tiers**: 0.2, 0.25, 0.33, 0.5, 1.0, 2.0, 3.0… and **`N/A`** (no cooldown — *does not benefit from Attack Speed*). This is a real edge case the sim and the "+Attack Speed" modifier must special-case.
- Status mechanics with exact rules, e.g. **Frost** = "2% move/attack-speed reduction per stack, max 25 stacks," → at 25 stacks an upgrade can **Freeze** (+50% frost damage taken); **Fire** = "+0.5% all damage taken per stack, enemies explode on death."

**Targeting is randomized** (no armor-type preference). Random target selection is therefore an explicit RNG draw — it must come from a deterministic stream (see [`05-data-model.md`](05-data-model.md)).

## 1.4 Economy & roguelike systems — confirmed

- **Rarity ladder with costs**: **Common = 500 gold**, then **Uncommon → Rare → Epic**. *(Inferred: the source confirms only the names Common/Uncommon/Rare plus an unnamed 4th purple‑colored tier — "Epic" is our name for it, and the ~5000g top cost is our estimate; only the 500g Common cost appears in the source strings.)* Offers are presumably drawn by rarity weighting *(inferred — the draw algorithm lives in the encrypted script; a full sweep of the tooltip/changelog artifacts attests NO odds-manipulation mechanics anywhere, only deterministic agency: reroll, bonus roll, the Black Market picker, and purchase copy-multipliers)*. Many upgrades scope themselves by rarity ("+100% Damage for 500 Gold (Common) Weapons", "+1 copy of next **Rare** Weapon").
- **Two income channels, deliberately scoped**: flat **Gold Income** (+X/sec, sometimes +%) and **Kill Bounty** (+% bounty, plus a 5%-chance proc for bonus bounty — *"+200% Bounty Gold with 5% activation chance"*). Multipliers are scoped so they don't trivially compound.
- **Reroll economy**: 5 free rerolls to start; cost escalates; Free Reroll sources exist. This is the core "pull the slot machine" loop.
- **Meta-items**: **Magic Treasure** (gain gold, value grows +2/sec, scales with bounty), **Magic Coins/Treasures**, **Multiplication Gems** (+3 copies of the next Common upgrade), **Black Market** (pick a specific Uncommon weapon). These are *targeted* shop manipulations — important because they make the offer stream **stateful**, not just "reroll random."
- **Defensive subsystems** beyond HP: **Armor**, **HP Regen** (with retroactive multipliers), **Dodge** (diminishing returns), and a deep **Mana Shield** system (absorb pool that regenerates and grants on-depletion effects). Multiple "trade HP for gold" upgrades exist (e.g. "−1000 Max HP, +2000 Gold").
- **Challenge / score mode**: single-player skins impose restrictions ("only Poison damage", "no income") for score (e.g. "*Survive 15 minutes and survive longer than any other player, then kill Samwise — Score +250*"). Confirms a **scoring/meta-progression** layer on top of the match.

## 1.5 Win / lose & last-man-standing

The competitive rule is asymmetric (the spine of the multiplayer):
- A tank is **eliminated** at 0 HP; relative survival time decides placement.
- **Win = finish in the surviving (top) half; lose = bottom half.** Sole survivor = **Last Stand**.
- The highest-prestige objective ("Score +250") is literally: *survive 15 minutes, outlive every other player, then kill Samwise.*

For the network layer this is the key simplification: **competitively, the only fact that must be exactly agreed across machines is "who died, and in what order."** That is small and low-frequency.

## 1.6 The networking-relevant findings (this is why the source matters)

Two things in the credits/strings are directly about multiplayer and shape our architecture:

1. **The map runs on WC3's peer-to-peer deterministic lockstep.** Every client simulates the whole world; only inputs cross the wire; the sim can't advance a tick until *all* players' inputs arrive. Right for shared-battlefield RTS, **wrong for independent arenas** — and the documented cause of the genre's pain: one slow link stutters the whole lobby, any desync is fatal, and there's no reconnect.

2. **The map ships "Codeless Save and Load (Multiplayer) v3.0.1 by TriggerHappy"** and "GameStatus – Replay Detection." (CREDITS, STRING 2562.) That save/load library exists precisely *because* WC3 has no server authority — it smuggles per-player state through WC3's **sync natives** so a save string can be generated identically on all machines without desyncing. The very presence of this hack is evidence of what's missing: **a server-authoritative place to hold per-player state.** Standing Tank Defense provides that natively.

### What we keep vs. replace

| WC3 Tower Survivors property | Why it hurts here | Standing Tank Defense ([03](03-network-architecture.md)) |
| --- | --- | --- |
| One lockstepped shared world | Sim stalls on the slowest peer | **Shard the sim per player**; your arena advances on *your* inputs + a server seed |
| Full-world determinism across **all** peers | Any cross-peer divergence = fatal desync | Determinism required only **pairwise (server ↔ one client)**; snapshot resync heals drift |
| P2P, host is a peer, no authority | Host quality dictates the match; cheat builds exist (hence "download from Discord") | **Server-authoritative match director**; dedicated/relay primary, P2P fallback |
| No reconnect; save/load hacked in via sync natives | A blip is permanent; state lives nowhere trustworthy | Server holds **seed schedule + per-player input log** → reconnect = re-seed + replay |
| Inputs-not-entities on the wire | The one **great** property — tiny bandwidth despite huge swarms | **Keep it**: ship seeds + sparse input events + small digests, never stream the swarm |

The one thing WC3 got right for this genre — *send inputs, not entities* — we keep. The things it got wrong — *global lockstep, no authority, no recovery* — we replace, and we **can** because the arenas are independent, which the original never exploited.

## 1.7 Sources

- Tower Survivors — Hive Workshop: https://www.hiveworkshop.com/threads/tower-survivors.353429/
- Tower Survivors (versions) — EpicWar: https://www.epicwar.com/maps/337083/
- Survivor Challenge TD (standalone descendant) — Steam: https://store.steampowered.com/app/2443170/Survivor_Challenge_TD/
- Codeless Save and Load (Multiplayer) v3.0.1 by TriggerHappy — Hive Workshop: https://www.hiveworkshop.com/threads/codeless-save-and-load-multiplayer-v3-0-1.278664/
- Explaining Warcraft's lockstep architecture (desyncs) — Hive Workshop: https://www.hiveworkshop.com/threads/explaining-warcrafts-lockstep-architecture-for-mapping-avoid-desyncs.351561/
- Warcraft 3's Networking: P2P vs. Servers Explained: https://flavor365.com/warcraft-3s-networking-p2p-vs-servers-explained/
- _Primary source: the uploaded `TowerSurvivors v1.58.w3x` itself — see [`appendix-A-map-extraction.md`](appendix-A-map-extraction.md)._
