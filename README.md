# Standing Tank Defense

> A multiplayer, last‑man‑standing, randomized tower‑defense / survival game built around **one tank you cannot move** — you only choose how to arm it. Inspired by the Warcraft III custom map **Tower Survivors** (itself inspired by *Vampire Survivors* / *Halls of Torment*), reimagined as a standalone title with a **network architecture designed for it from day one**.

This repository currently holds the **planning / specification package**. No engine code yet — the goal of this pass is a buildable, opinionated spec with the multiplayer architecture nailed down first, because networking is the part that is expensive to retrofit.

---

## The pitch in one paragraph

Up to **8 players** each defend their **own** lane with a **single stationary tank**. You don't move and you don't aim — the tank auto‑fires. The entire game is **resource and build decisions**: as gold trickles in and random upgrade offers appear, you decide which weapons, modifiers, and economy upgrades to stack against an enemy tide that scales for ~20 minutes. Everyone fights their *own* swarm in parallel; the multiplayer layer is a **race to outlast everyone else**. Die in the first half of the lobby and you lose; finish in the surviving half and you win; be the last tank standing and you score a **Last Stand**.

## Why this design is interesting for networking

The Warcraft III original ran on WC3's **peer‑to‑peer deterministic lockstep**: every machine simulates the *entire* shared world and the simulation can't advance a tick until **every** player's input has arrived. That is the right tool for a shared‑battlefield RTS, but it is the *wrong* tool here and it caused the classic problems players complained about — one laggy player stutters the whole lobby, a single desync ends the match for everyone, and there is no reconnect.

The key realization that drives this whole spec: **the arenas are independent.** Your enemies, projectiles, and tank never touch another player's arena. The only things that are genuinely *shared* are tiny and slow‑changing: the round clock, who is still alive, and the leaderboard. So instead of one giant lockstepped world, we run **N sharded, independently‑advancing simulations** coordinated by a small **server‑authoritative match director**. Your game advances on *your* inputs plus a server‑issued RNG seed — it can never stall waiting on someone else, a local desync self‑heals from a server snapshot, and reconnection is just "replay my input log." See [`docs/03-network-architecture.md`](docs/03-network-architecture.md).

---

## Document index

| Doc | What's in it |
| --- | --- |
| [`docs/01-source-analysis.md`](docs/01-source-analysis.md) | Analysis of the WC3 **Tower Survivors** map, what the community said about it, and the networking lessons we're carrying forward. Includes a note on the download attempt. |
| [`docs/02-game-design.md`](docs/02-game-design.md) | The game design itself: pillars, the single tank, rounds & waves, weapons/upgrades/synergies/rarities, economy, bosses, and the last‑man‑standing win logic. |
| [`docs/03-network-architecture.md`](docs/03-network-architecture.md) | **The centerpiece.** Topology, authority model, the sharded‑simulation thesis, tick/determinism model, the explicit comparison vs WC3 lockstep, reconnection, host migration, anti‑cheat, and scaling/bandwidth budgets. |
| [`docs/04-protocol-and-messages.md`](docs/04-protocol-and-messages.md) | Wire protocol: transport & channels, the full message catalog, time synchronization, the match state machine, and sequence diagrams. |
| [`docs/05-data-model.md`](docs/05-data-model.md) | Simulation entity model and the content‑data schemas (weapons, modifiers, enemies, wave tables) plus the RNG‑stream design that makes randomization deterministic. |
| [`docs/06-roadmap-risks-testing.md`](docs/06-roadmap-risks-testing.md) | Milestones / vertical slices, the risk register (networking‑weighted), the determinism & netcode test plan, and open questions. |

## Status & scope of this spec

- **Engine/stack‑agnostic where it can be, opinionated where it matters.** The networking architecture is specified concretely; recommended concrete stacks are called out as recommendations, not requirements.
- **Assumptions are labeled.** Anything reconstructed from the WC3 map (rather than verified against decompiled triggers) is marked *(inferred)* so it can be corrected once someone with the `.w3x` open confirms numbers.
- **Open questions** are collected at the end of [`docs/06-roadmap-risks-testing.md`](docs/06-roadmap-risks-testing.md) rather than blocking the spec.

## Naming note

"Standing Tank Defense" comes from the repository name and the core fantasy (a *standing* — immobile — tank that *defends*). The WC3 source used a literal "tower"; we use a tank purely as the theme. Mechanics are identical in spirit.
