# Standing Tank Defense — Playtest Guide

A free, last-tank-standing **tower-defense / survival** game. You command **one
immobile tank**. You can't move — you survive by **buying weapons and upgrades**
from a shop that refreshes every round, and holding out against escalating waves
longer than everyone else.

This guide gets you from zero to playing, then explains every screen and control.

---

## 1. Get it running

You need two things installed:

- **Godot 4.3+** (tested on 4.4.1) — the game engine. Download from
  <https://godotengine.org/download>. It's a single executable, no installer.
- **Rust** (stable toolchain) — to build the simulation core. Get it from
  <https://rustup.rs>.

Then, from the repository:

```bash
# 1. Build the simulation core (the GDExtension the game loads).
cd godot/rust
cargo build            # first build downloads deps; takes a couple minutes
cd ..

# 2. Launch the game: open the `godot/` folder in Godot 4 and press Play (F5).
#    (Or from a terminal:  <path-to-godot> --path godot )
```

That's it — you'll boot to the **Tank Select** screen.

> If Godot says it can't find `StSim`, the Rust build didn't finish or landed in
> the wrong place. Confirm `godot/rust/target/debug/libstanding_tank_gdext.*`
> exists, then reopen the project.

---

## 2. The 60-second version

1. On **Tank Select**, press **Enter** to deploy with the starter tank.
2. You're now in a single arena. Enemies stream in from the ring toward your tank.
3. A **shop bar** sits along the bottom. Press **1–8** (or click a card) to buy.
   **All 8 slots roll independently from one weighted pool** — weapons and
   economy / passives / spikes appear mixed; each card's colored pip and label
   show its category, and rarer cards (rare/epic) are less common.
4. Keep buying as gold comes in. The shop **refreshes every round (~30s)**; press
   **R** to reroll it sooner. (The shop **closes for good at 15:00** — it flees
   when the boss arrives — so spend before the bell.)
5. When the screen gets swamped, press **Space** to **Clear** (a board wipe — and
   the *only* thing that damages the boss).
6. Survive the 15-minute ramp (enemies step up **+20% at 10:00**), then kill the
   **boss** with repeated Clears while the waves keep escalating ("swift end").
   When your tank dies you get a **run summary** — press **Enter** to redeploy
   and try again.

---

## 3. Screens & controls

### Tank Select (start screen)
A gallery of every tank skin. Unlocked ones are pickable; locked ones show the
achievement that unlocks them.

| Key | Action |
| --- | --- |
| **Arrows / WASD** | Move selection |
| **Enter / Space** | Deploy with the selected tank |
| **Click** a tank | Select it (click again to deploy) |
| **C** | Open the **Challenge picker** |
| **L** | Open the **multiplayer Lobby** |
| **M** | Open the **Multi-arena net demo** |
| **T** | Cycle the **art theme** (Grimdark ⇄ Gaslamp) |
| **G** | Toggle **language** (English ⇄ 简体中文) |
| **U** | *Dev:* unlock every skin (to preview the gallery) |
| **R** | *Dev:* relock everything (reset progress) |
| **Esc** | Quit |

### Single-arena play (the main game)
Your tank sits at the center. The **HUD** (top-left) shows your **HP**, **gold**
(and income per tick), and the **round**. Top-right is your **Arsenal** — what
you currently own. The **shop bar** is along the bottom.

**Shop rules**
- **8 offers** per round. **Every slot rolls independently** from one weighted
  pool — first weapon-vs-modifier, then rarity (**50 / 30 / 15 / 5** for
  common / uncommon / rare / epic) — so the board is a mix, not fixed halves.
  The colored **category pip** on each card (weapon / economy / spike / passive)
  tells you what it is.
- **Buy:** press the slot's number **1–8**, or click the card. Buy as many as you
  can afford.
- **Hover** a card for its lore + mechanical effect, its cost, and rarity.
- **R — Reroll:** get a fresh set of offers (first reroll each round may be free;
  after that it costs gold).
- The shop **auto-refreshes every round (~30 seconds).** Slots persist until then,
  so you can keep buying from the same board.

| Key | Action |
| --- | --- |
| **1–8** | Buy that shop slot |
| **R** | Reroll the shop |
| **Space** | **Clear** — board wipe; the only thing that hurts the boss |
| **T** | Cycle the art theme |
| **N** | Mute / unmute sound (works on the run-summary panel too) |
| **M** | Jump to the multi-arena net demo |
| **Esc** | Back to Tank Select |

**When you die:** a centered **run summary** appears (round reached, damage, gold,
weapons bought — each with your **personal best** beside it — plus how close you
came to the boss, what overwhelmed you, any achievements you just unlocked, and
your **next achievement goals**). Personal bests and your last runs persist
between sessions. The pause menu (**Esc** while alive) also offers a
confirm-gated **Restart Run**.
- **Enter / Space / click Redeploy** — start a fresh run immediately.
- **Esc** — back to Tank Select.

### Challenge picker (press **C** from Tank Select)
Self-imposed rules (e.g. *only Splash weapons*, *no economy*). Each clears to
unlock a reward skin. Your tank's bot honors the rule for the run, so the
matching achievement is earnable on demand.

| Key | Action |
| --- | --- |
| **↑/↓** | Choose a challenge (or **Free Play**) |
| **Enter / Space / click** | Deploy with that rule |
| **C / Esc** | Back |

### Lobby (press **L** from Tank Select)
The front door to a networked match: a **host-authoritative lobby** driven by the
real netcode's lobby state machine (you are the host, seat 0). Simulated peers
join, pick their own cosmetics, and ready up after a moment (until live Steam
matchmaking lands, peers are simulated); when everyone is ready, start the match
— it launches the multi-arena view with the lobby's seats.

| Key | Action |
| --- | --- |
| **Space / Y** | Toggle *your* ready flag |
| **A / +** | Add a (simulated) player |
| **X / −** | Remove the last player |
| **R** | Ready everyone (impatience button) |
| **Enter** | Start the match (host-only; needs ≥2 players, all ready) |
| **T** | Cycle the art theme |
| **Esc** | Back to Tank Select |

### Multi-arena net demo (press **M**)
The headline architecture, made visible: **8 independent arenas under one
authoritative director**, each its own player. **Your arena is the large featured
panel**; the other seven are smaller cells around it, each rendered in **that
player's own theme and tank skin** (so you can see everyone's cosmetics). Watch
players get knocked out until one is left standing.

| Key | Action |
| --- | --- |
| **T** | Cycle the art theme |
| **S / Esc** | Back to Tank Select |

### Themes & language
Press **T** on most screens to swap the whole look between **Grimdark** and
**Gaslamp Bulwark** — different art *and* UI colors (gold/blood vs brass/aether).
Press **G** on Tank Select to toggle the language (English ⇄ 简体中文), and **N**
in the arena to mute/unmute sound. Both stick between sessions.

---

## 4. How to play well

- **You can't move.** Positioning isn't the game — **shop decisions** are.
- **Weapons vs. economy:** weapons kill things now; economy compounds your gold
  so you can out-buy the late game. Most runs want some of both — pure-greed and
  pure-aggression are both viable but risky.
- **Clear (Space)** is your panic button and your **only answer to the boss**
  (~10 Clears to kill it) — but it's on a cooldown, so spend it wisely.
- **High-risk picks exist on purpose.** Some cheap cards are deliberately
  double-edged (they can even kill you). They're meant as gambles for a high
  ceiling — read the tooltip before you commit.
- Survive to round milestones and rack up damage/gold to unlock new tanks.

---

## 5. Unlocks

Tanks are cosmetic and earned through achievements — e.g. **purist runs** (buy
only one weapon type), **Jack of All Trades** (one of every type), **Bloodletter**
(1,000,000 damage), **Long Watch** (reach round 20), **Sole Survivor** (win a net
match). Unlock progress is saved between sessions.

---

## 6. (Advanced / optional) Steam test mode

Networked multiplayer ships over **Steam** (host-authoritative over Steam Datagram
Relay). For local bring-up testing it uses **App ID `480` (Spacewar)**, Valve's
public test app — so it works against any running Steam client **without
registering anything**. Building the Steam transport additionally needs the
Steamworks SDK; see **`docs/07a-steam-bringup.md`**. You do **not** need any of
this to play the single-player and net-demo flows above.

---

## 7. Troubleshooting

| Symptom | Fix |
| --- | --- |
| "Can't find StSim" / blank screen | The Rust core isn't built. Run `cargo build` in `godot/rust`, confirm `target/debug/libstanding_tank_gdext.*` exists, reopen the project. |
| Tank skins all look identical | You're on a theme that's missing that skin art, or progress was reset. Press **T** to switch theme; locked skins fall back to the default tank. |
| Game won't open in Godot | Use **Godot 4.3 or newer** (4.4.1 recommended). Older 4.x may need the extension's compatibility floor lowered. |
| No glow/bloom | Bloom needs the Vulkan (`forward_plus`) renderer; on a software/OpenGL fallback the game still plays, just without the glow. |

---

## 8. Telling us how it went

When you playtest, the most useful feedback: **how far did a run feel decided by
the shop vs. by luck?** Did any purchase feel like an obvious trap or an obvious
must-buy? Were the first ~30 seconds fair? Did anything read as unclear on screen?
