# 03 — Network Architecture (the centerpiece)

This is the load-bearing document. Standing Tank Defense is designed **networking-first** because the genre's defining property — a *Vampire-Survivors*-scale swarm per player, across up to 8 players — is exactly what breaks naïve netcode, and because the WC3 original ([`01`](01-source-analysis.md)) shipped on an architecture that fought the game instead of fitting it.

## 3.1 The one insight everything hangs on

**The arenas are independent.** Your enemies, projectiles, status stacks, and tank never interact with another player's arena. The *only* genuinely shared state is tiny and slow:

- the global round number and authoritative clock,
- who is still alive (and each death's tick),
- the leaderboard (survival time, kills, wave, gold),
- the seed schedule that makes every arena reproducible.

Everything else — the thousands of moving entities — is **local to one player**. That single fact lets us discard WC3's global lockstep and replace it with **N independently-advancing simulations coordinated by a small server-authoritative match director.** Your game advances on *your* inputs plus a server seed; it can **never** stall waiting on another player.

```
        ┌──────────────────────────────────────────────┐
        │            MATCH DIRECTOR (server)            │
        │  authoritative clock · seed schedule · alive  │
        │  /dead truth · leaderboard · placement        │
        └───▲────────▲────────▲────────▲────────▲───────┘
   small, low-rate  │        │        │        │  (seeds, acks, time beacons,
   reliable control │        │        │        │   leaderboard deltas, death events)
        ┌───────────┴┐  ┌────┴─────┐  ┌┴─────────┐  ┌┴──────────┐
        │  Arena P1  │  │ Arena P2 │  │ Arena P3 │  │  Arena Pn │   ← independent sims,
        │ (sim+view) │  │(sim+view)│  │(sim+view)│  │(sim+view) │     thousands of entities
        └────────────┘  └──────────┘  └──────────┘  └───────────┘     each, never streamed
              ▲ server keeps a validating shadow-sim of each arena (anti-cheat / death truth)
```

## 3.2 Topology & authority

**Authoritative server, per-player sharded simulation.** We reject pure P2P lockstep (WC3's model) and we reject naïve full-state streaming.

> **Deployment for the free Steam release:** the authority is *not* a rented box — it's a **host player** running the match director over **Steam Datagram Relay**, so there's no server bill while keeping this exact authority model. Dedicated/community servers run the same director binary as an optional **ranked** upgrade. The authority *model* below is unchanged; only *who hosts it* moves. Full mapping in [`07-steamworks-integration.md`](07-steamworks-integration.md). Throughout this doc, "server" = "whoever runs the match director" (a host player by default).

Authority is split by *what kind* of state it is:

| State | Authority | Rate | Reliability |
| --- | --- | --- | --- |
| Global clock, round #, seed schedule | **Server** (match director) | low (beacons) | reliable |
| Per-arena simulation (entities, projectiles, HP) | **Owning client simulates; server keeps a validating shadow** | tick-rate locally; **not streamed** | n/a (derived from seed+inputs) |
| Player inputs (buy/upgrade/reroll/Clear) | **Server orders & acks** | sporadic | reliable-ordered |
| Death / placement | **Server** (confirmed against shadow-sim) | event | reliable |
| Leaderboard / digests | Client reports, **server aggregates** | 2–5 Hz | unreliable (newest-wins) |

The key move: **the heavy simulation is never authoritative-on-the-wire.** It is *derived* identically on the client and the server's shadow from `seed + ordered inputs`, so we transmit the seed and the inputs, not the entities.

## 3.3 Simulation model: deterministic, shared-seed, inputs-on-the-wire

We keep the *one* thing WC3 got right — **send inputs, not entities** — and drop everything else.

- **Fixed simulation tick** (recommend **30 Hz**; integer `tick` counter). All gameplay advances in whole ticks.
- Each arena is a pure function: `state[t+1] = step(state[t], inputs_at[t], rng_streams)`.
- `rng_streams` are seeded by the server (§3.6). `inputs_at[t]` are the owning player's ordered, server-stamped actions.
- Because of this, **the client and the server's shadow-sim produce the same arena from the same seed+inputs.** The wire carries: seeds (once per round), input events (a handful per round), small periodic **digests/checksums**, and — only as a correction/recovery mechanism — full **snapshots**.

### Why this is the right model for *this* game
A survivors-like has **hundreds–thousands of entities per arena**. Streaming them (modern action-game style) would cost hundreds of KB/s per player even with delta compression and would dominate the bandwidth budget. Deterministic shared-seed simulation makes bandwidth a function of **inputs**, not entities — so 8 players with 10,000 total on-screen enemies cost about the same as 8 players with 100. That property is the whole reason the game scales (§3.8).

### Determinism, but only where it's cheap
WC3 required **bit-exact determinism across all peers simultaneously** — brittle, and one divergence killed the match. We need determinism only **pairwise: server-shadow ↔ that one client**, with a **snapshot resync** safety net. That means:

- Minor float nondeterminism is *survivable*: a digest mismatch triggers a targeted snapshot correction of one arena, invisible to everyone else.
- We still **engineer for determinism** to make corrections rare: fixed timestep, integer/fixed-point math in the hot path, a deterministic PRNG, **stable entity iteration order** (sort by entity id), and a banned-ops list (no wall-clock, no unordered hashmap iteration, no platform `rand()`, no float-dependent container ordering). See [`05-data-model.md`](05-data-model.md) §Determinism.

## 3.4 The match director (shared meta-layer)

A small authoritative service that owns the lobby→match→resolution lifecycle and the *shared* facts. It is deliberately tiny and low-rate, so it's cheap to host and easy to make correct.

Responsibilities:
1. **Lobby & slotting** — assign player slots, gate ready-up, pick the **master seed**.
2. **Clock** — own match time; emit periodic **time beacons** for client clock-sync (§3.5). Apply host-set game-speed uniformly.
3. **Seed schedule** — at each round boundary, broadcast `(round#, wave_table_id, per-player spawn seed)` so every arena spawns the correct wave deterministically.
4. **Input ordering** — receive player inputs, assign each an authoritative `(tick, seq)`, ack to the owner, apply to that player's shadow-sim. Inputs affect only the owner's arena, so they are **not** broadcast to other players (only summarized into the leaderboard).
5. **Liveness & truth** — ingest per-client digests; detect death (shadow-sim HP≤0, corroborated by the client's reported death); record death tick; resolve **placement** and **Last Stand**.
6. **Leaderboard** — aggregate digests into a standings feed (delta-encoded).

## 3.5 Time synchronization

The simulation is tick-based and local, so we don't need tight cross-client frame-sync. We need each client to agree on **match time** for round boundaries, scaling steps, and the boss.

- Server emits **time beacons** (as implemented: `server_tick` only — see `docs/04 §4.4.2`) a few times per second.
- Clients estimate offset/RTT with an NTP-like filter (track min-RTT samples, smooth the offset) and run their local sim against *estimated server tick*.
- Round/scaling/boss events are scheduled by **absolute server tick**, not "N seconds from when I received the message," so they fire coherently everywhere despite jitter.
- A client that drifts past a tolerance re-syncs from the next snapshot rather than fast-forwarding visibly.

## 3.6 Randomness & seeds (networked view)

(Design rationale in [`02 §2.8`](02-game-design.md); data layout in [`05`](05-data-model.md).)

- One **master seed** per match (server-chosen, unguessable). Per-player, per-purpose streams are derived: `stream = PRNG(hash(master_seed, player_id, purpose, round))`.
- **Purposes**: `spawn`, `targeting`, `shop/offer`, `reroll`, `proc`. Splitting streams means a reroll can't perturb spawn timing and vice-versa, which keeps replays stable and bugs local.
- The server reveals only the seeds a client needs, **when** it needs them (e.g. the round's spawn seed at round start), so clients can't precompute future shop offers to plan around RNG. The **shop stream is server-gated per offer** for the same reason.
- Because seeds are server-owned and streams advance only via ordered inputs, **save-scumming and loot-rerolling are impossible**, and any arena is **exactly replayable** for validation/reconnect.

## 3.7 Reconnection & resilience (what WC3 couldn't do)

The original had *no reconnect* and bolted on a "Codeless Save/Load" hack to smuggle state through sync natives ([`01 §1.6`](01-source-analysis.md)). We get reconnect almost for free:

- The match director holds, per player: **the seed schedule + the ordered input log + a recent authoritative snapshot**.
- On reconnect, the server sends `seeds + input log (+ latest snapshot)`; the client **re-simulates** from the snapshot to the current tick and rejoins. Catch-up is fast because between snapshots the input log is tiny.
- A disconnected player's arena keeps running on the **server shadow-sim**, so they can keep dying/surviving (and place) even while offline — or we apply a grace policy **[design choice]** (freeze vs. continue), configurable per match.
- **No player's disconnect affects any other arena.** Contrast WC3, where a drop could desync or stall the whole table.

## 3.8 Bandwidth & scaling budget

Because we ship inputs, not entities, traffic is dominated by low-rate control and digests, *independent of swarm size*:

| Channel | Direction | Rate | Rough size |
| --- | --- | --- | --- |
| Time beacons | S→C | 2–4 Hz | tens of bytes |
| Round seed / wave id | S→C | per round (~30 s) | tens of bytes |
| Input events (buy/upgrade/reroll/Clear) | C→S→ack | sporadic (a few/round) | tens of bytes each |
| Digest/heartbeat (HP, gold, kills, wave, checksum) | C→S | 2–5 Hz | ~100–300 B |
| Leaderboard delta | S→C | 1–2 Hz | scales with player count, not entities |
| Snapshot (correction/reconnect only) | S→C | rare | KB-range, one arena |

Order-of-magnitude: **single-digit KB/s per client** in steady state, flat as the swarm grows. A naïve entity-streaming design would be **1–2 orders of magnitude** higher and would *grow* with the late-game swarm — exactly when the game is most demanding. This budget is the quantitative justification for the whole architecture.

## 3.9 Anti-cheat

The dedicated server runs a **validating shadow-sim** of each arena (cheap: deterministic, input-driven):

- **Input legality** — every action checked against the server's economy/cooldown/offer state (enough gold, offer was actually available, reroll count valid, `Clear` off cooldown). Illegal inputs are rejected at ordering time.
- **State validation** — client digests/checksums are compared to the shadow-sim each tick-batch; a mismatch triggers a snapshot correction and increments a suspicion score. Repeated/large divergence → kick.
- **Server owns RNG** — clients can't fabricate favorable offers, crits, or bounty procs.
- **Cost control [design choice]** — full shadow-sim of every arena is affordable (the sim is light and deterministic), but for very large lobbies the server may run **spot-check validation** (verify checksums every tick, fully re-simulate only on suspicion or for the contested death/placement events). Death and placement are *always* fully validated because they're the only competitively load-bearing facts (§3.1).

This directly answers the source community's "cheat versions" problem (the author had to distribute an anti-cheat build via Discord): with server authority, the client is never trusted for anything that matters.

## 3.10 Host migration & P2P fallback

- **Free Steam release (default): host-as-server over SDR.** One client runs the match director; Steam Datagram Relay handles routing/NAT/encryption. Because the director's state is **small** (seeds, input logs, alive/dead, leaderboard) and replicated to a hot standby, the role can **migrate** on host loss with a brief pause, using Steam lobby-ownership transfer. Arenas keep their local sims through migration since they're seed+input derived. See [`07 §7.5`](07-steamworks-integration.md).
- **Optional dedicated/community server (ranked):** same director binary on a neutral host → no migration problem and stronger anti-cheat; clients only ever talk to the server.
- We never use WC3-style symmetric P2P where every peer simulates every world — that's the model we're explicitly replacing.

## 3.11 Why not the alternatives (explicit)

| Alternative | Verdict | Reason |
| --- | --- | --- |
| **WC3 P2P deterministic lockstep** | ✗ | Global tick gated on all players → slowest peer stalls everyone; desync fatal; no reconnect; no authority (cheats). Wrong for independent arenas. |
| **Full server-authoritative state streaming** (snapshots + client prediction, FPS-style) | ✗ as primary | Bandwidth grows with the swarm (hundreds of KB/s late game); the genre's entity counts make this the worst case. *Reused only* as our correction/reconnect snapshot path. |
| **Pure client-authoritative, no validation** | ✗ | Trivially cheatable; can't arbitrate placement fairly. |
| **Sharded deterministic sim + authoritative meta-director + shadow-sim validation** (this doc) | ✓ | Never stalls cross-player; bandwidth flat in entity count; fully reconnectable/replayable; server-authoritative where it counts. |

## 3.12 Cross-references & open networking questions

- Wire protocol, channels, message catalog, state machine, sequence diagrams → [`04-protocol-and-messages.md`](04-protocol-and-messages.md).
- Entity/content schemas, RNG stream layout, determinism rules → [`05-data-model.md`](05-data-model.md).
- Milestones, the netcode test plan (latency/chaos injection, determinism harness, soak), risk register → [`06-roadmap-risks-testing.md`](06-roadmap-risks-testing.md).

Open questions (tracked in [`06`](06-roadmap-risks-testing.md)): grace policy for disconnects; whether to ever expose a co-op shared-arena mode (which would re-introduce coupling); exact snapshot cadence vs. determinism confidence; transport choice (§[`04`](04-protocol-and-messages.md)).
