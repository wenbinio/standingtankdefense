# 04 — Protocol & Messages

The wire-level spec for the architecture in [`03-network-architecture.md`](03-network-architecture.md): transport, channels, the message catalog, the match state machine, and sequence diagrams. Concrete framing is given as a recommendation; the *shape* (which messages exist, who's authoritative, what's reliable) is the spec.

## 4.1 Transport & channels

**Recommendation:** the shipping (free Steam) build uses **Steamworks `ISteamNetworkingSockets`** over **Steam Datagram Relay** — it's UDP-style, reliable/unreliable per message, with NAT traversal, encryption, and auth handled for free, addressed by SteamID. See [`07-steamworks-integration.md`](07-steamworks-integration.md) for the API and send-flag mapping. (Underneath, this is the same technology as Valve **GameNetworkingSockets**; **ENet**/**QUIC** are equivalent fallbacks for non-Steam/dedicated builds.) The channel semantics below are what matter and are transport-independent.

Three logical channels:

| Channel | Delivery | Carries |
| --- | --- | --- |
| **CONTROL** | reliable, ordered | lifecycle, seeds, input events + acks, death/placement |
| **TELEMETRY** | unreliable, newest-wins | time beacons, client digests/heartbeats, leaderboard deltas |
| **BULK** | reliable, ordered, rare | snapshots (correction / reconnect), content manifest |

Rule of thumb: **anything that changes authoritative truth is CONTROL/reliable; anything that's a periodic estimate is TELEMETRY/unreliable; anything large and occasional is BULK.**

## 4.2 Conventions

- All gameplay time is an integer **`tick`** (30 Hz). Wall-clock appears only in time beacons.
- IDs: `match_id`, `player_id` (slot 0..7), `entity_id` (per-arena, monotonic), `seq` (per-player input counter).
- Encoding: compact binary (FlatBuffers/Protobuf/bitpacked). JSON shown below for readability only.
- Every CONTROL message from a client carries `(player_id, seq)`; the server acks by `seq`.

## 4.3 Match state machine

```
        ┌────────┐  all ready / host start   ┌────────────┐
        │ LOBBY  │ ────────────────────────► │ COUNTDOWN  │
        └────────┘                           └─────┬──────┘
             ▲                                      │ T-0, master seed locked
             │ rematch                              ▼
        ┌────┴─────┐   ≤1 alive OR boss done  ┌──────────┐  round end   ┌──────────┐
        │RESOLUTION│ ◄────────────────────────│  ROUND   │◄────────────►│   SHOP   │
        └──────────┘                          │ (combat) │  round start └──────────┘
                                              └────┬─────┘
                                                   │ ~15:00 reached
                                                   ▼
                                              ┌──────────┐
                                              │   BOSS   │ (Samwise; Clear-only)
                                              └──────────┘
```

ROUND and SHOP overlap in practice (the shop opens each round while combat continues); they're shown separate for clarity. `RESOLUTION` is entered the instant the alive-count or boss condition trips.

## 4.4 Message catalog

> **Authoritative source of truth.** The list of messages that actually go on
> the wire is the `net::wire::Msg` enum and its companion
> `net::wire::TRANSMITTED_MESSAGES` constant (`sim-core/crates/net/src/wire.rs`).
> The table in §4.4.0 below must enumerate **exactly** those variants; the test
> `sim-core/crates/net/tests/wire_doc_sync.rs` fails the build if the enum, the
> const, and this catalog drift apart. The JSON payloads shown in later
> subsections are *illustrative readability sketches* — the implemented codec is
> compact little-endian binary with the fields listed per variant; not every
> sketched field exists yet (see "Implemented fields" notes).

### 4.4.0 Transmitted messages (the wire `Msg` set)

These nine variants are the complete set of messages encoded onto the wire by
the M2+ codec. **If it isn't in this table, it isn't sent.** Lobby/ready
lifecycle and seed-derived events (rounds/shop/boss) are deliberately *absent* —
see §4.4.7.

| Message | Dir | Channel | Implemented fields | Purpose |
| --- | --- | --- | --- | --- |
| **`MatchStart`** | S→C | CONTROL | `start_tick:u32`, `master_seed:u64` | Begin the match; client builds `ArenaState::new(master_seed, player_id)` at `start_tick`. |
| **`TimeBeacon`** | S→C | TELEMETRY | `server_tick:u32` | Authoritative clock beacon for drift correction. |
| **`InputAck`** | S→C | CONTROL | `seq:u32`, `apply_tick:u32` | Ack an input and pin its authoritative apply tick. |
| **`Snapshot`** | S→C | BULK | `tick:u32`, `bytes:Vec<u8>` | Authoritative serialized arena state (correction / reconnect). |
| **`DeathConfirmed`** | S→C | CONTROL | `player:u32`, `died_tick:u32`, `place:u32` | A player was eliminated; `place` is their final placement. |
| **`MatchResult`** | S→C | CONTROL | `places:Vec<(u32,u32)>` | Final standings as `(player, place)` pairs. |
| **`Join`** | C→S | CONTROL | `content_hash:u64` | Join handshake; `content_hash` gates version/content match. |
| **`Input`** | C→S | CONTROL | `seq:u32`, `action:InputCode` | A player action; the director assigns its `apply_tick`. |
| **`Digest`** | C→S | TELEMETRY | `tick:u32`, `checksum:u64` | Liveness + drift digest for the client's current tick. |

The `Input.action` payload is the `InputCode` tagged union — the only actions on
the wire today are: `Noop`, `BuyOffer(slot:u8)`, `Reroll`, `Clear`. (The richer
action menu sketched in §4.4.3 — `UseItem`, `BlackMarketPick`, `SetGameSpeed` —
is **not yet on the wire**; it is planned, not implemented.)

### 4.4.1 Lifecycle (CONTROL)

**`Join`** (C→S) — the implemented handshake. On the wire it carries a single
`content_hash:u64`:
```json
{ "type":"Join", "content_hash":"u64" }
```
`content_hash` gates version/content mismatch up front — the #1 desync cause in
WC3 was version drift; here it's a hard pre-match check. (The richer
`JoinRequest` sketch with `match_id` / `client_version` / `display_name` is a
future expansion; those identities are carried by the Steam lobby layer today —
see §4.4.7.)

**`MatchStart`** (S→C, broadcast) — implemented as `{ start_tick:u32, master_seed:u64 }`:
```json
{ "type":"MatchStart", "start_tick":150, "master_seed":"u64" }
```
The director sends the `start_tick` and the `master_seed` from which every
client deterministically builds its arena. (A future hardening replaces the raw
seed with a **commit** revealed per round so no client can precompute the whole
match's RNG; the current codec ships the seed directly. Lobby/ready/host
selection — formerly sketched here as `JoinAccept` / `LobbyState` / `Ready` — is
handled off-wire by the in-process `Lobby` state machine; see §4.4.7.)

### 4.4.2 Clock (TELEMETRY)

**`TimeBeacon`** (S→C, unreliable) — implemented as `{ server_tick:u32 }`:
```json
{ "type":"TimeBeacon", "server_tick":5400 }
```

Rounds, shop offers, and the boss are **not clock messages and are not
transmitted** — they are derived deterministically from the master seed + tick.
See §4.4.7.

### 4.4.3 Player inputs (CONTROL, reliable-ordered)

A single envelope covers every player action; `action` is the `InputCode` tagged
union. Inputs affect only the sender's arena.

**`Input`** (C→S) — implemented as `{ seq:u32, action:InputCode }`:
```json
{ "type":"Input", "seq":87, "action": { "kind":"BuyOffer", "slot":2 } }
```
`action` (`InputCode`) — **implemented** kinds:
- `Noop` — no-op / heartbeat input slot.
- `BuyOffer(slot:u8)` — purchase the weapon/upgrade in a shop slot.
- `Reroll` — refresh offers (consumes a reroll / charges gold).
- `Clear` — fire the manual `Clear` ability (the one real-time combat input; also the boss-damage action).

Planned, **not yet on the wire**: `UseItem { item_id }` (Magic Treasure /
Multiplication Gems / Black Market pick), `BlackMarketPick { weapon_id }`,
`SetGameSpeed { speed }` (host-only, server-validated).

**`InputAck`** (S→C) — implemented as `{ seq:u32, apply_tick:u32 }`:
```json
{ "type":"InputAck", "seq":87, "apply_tick":21752 }
```
The server stamps the **authoritative `apply_tick`** so client and shadow apply
the input at the same tick. (Planned extension: an authoritative
`result`/`gold_after`/`rerolls_after` economy echo for optimistic-UI rollback,
per §4.6 — not yet carried on the wire.)

### 4.4.4 Liveness & digests (TELEMETRY, unreliable)

**`Digest`** (C→S) — heartbeat + drift check, implemented as `{ tick:u32, checksum:u64 }`:
```json
{ "type":"Digest", "tick":21900, "checksum":"u64" }
```
`checksum` is the arena's `state_checksum` (`docs/05 §5.6`); the server compares
it to its shadow-sim (§4.6).

**`LeaderboardDelta`** — **planned / not yet on the wire.** Standings are
currently reconstructed by the director from `DeathConfirmed` / `MatchResult`;
a periodic changed-standings telemetry message is a future addition:
```json
// PLANNED — not transmitted
{ "type":"LeaderboardDelta", "tick":21900,
  "entries":[ {"player_id":5,"alive":false,"place":8,"died_tick":21010,"wave":9} ] }
```

### 4.4.5 Death, placement, resolution (CONTROL, reliable)

There is no client-sent `DeathReport` on the wire — death is **server-confirmed
only**. The director detects elimination against its shadow sim and broadcasts
`DeathConfirmed`; the formerly-sketched advisory `DeathReport` (C→S) is *not
transmitted* (the client never reports its own death authoritatively).

**`DeathConfirmed`** (S→C, broadcast) — implemented as `{ player:u32, died_tick:u32, place:u32 }`:
```json
{ "type":"DeathConfirmed", "player":3, "died_tick":22050, "place":4 }
```
(Planned field: `alive_remaining` for client HUD; not yet carried.)

**`MatchResult`** (S→C, broadcast) — implemented as `{ places:Vec<(u32,u32)> }`:
```json
{ "type":"MatchResult", "places":[ [7,1], [3,4] ] }   // (player, place) pairs
```
(Planned fields: `last_stand` / `killed_boss` flags and `win_cutoff_place`; not
yet carried — placement order in `places` is the authoritative result today.)

### 4.4.6 Snapshots & reconnect (BULK, reliable)

**`Snapshot`** (S→C) — authoritative full state of **one** arena at a tick,
implemented as `{ tick:u32, bytes:Vec<u8> }` where `bytes` is the serialized
authoritative arena (carrying its rng cursors so the client can continue
deterministically):
```json
{ "type":"Snapshot", "tick":22000, "bytes":"<serialized arena incl. rng cursors>" }
```
On reconnect/correction the client loads `bytes`, then replays any inputs after
the snapshot tick to catch up to the live tick (`docs/03 §3.7`).

**`SnapshotRequest`** (C→S) — **planned / not yet on the wire.** Today the
director *pushes* a `Snapshot` when it detects a checksum mismatch (§4.6) or on
reconnect; a client-initiated `{ "reason":"reconnect" | "desync" }` pull is a
future addition.

### 4.4.7 Not transmitted (derived from seed+tick, or handled off-wire)

The architecture in `docs/03` ships **inputs + seeds, not entities**, and shards
each player into an independent deterministic sim. Because of that, several
"messages" named in earlier drafts of this spec are intentionally **never sent**
— transmitting them would be redundant (the receiver can compute them) or would
leak authority the director must keep:

| Concept | Status | Why |
| --- | --- | --- |
| **`RoundStart`** | **DERIVED from seed+tick — not transmitted** | Round boundaries, wave tables, per-player spawn seeds, and scaling steps are a pure function of `master_seed` + `tick`. Every client computes them identically; sending them would be redundant and a desync surface. |
| **`ShopOffer`** | **DERIVED from seed+tick — not transmitted** | The shop offer set and reroll cost are produced by the seeded shop RNG stream at the current tick; client and shadow generate the same offers from the same seed. |
| **`BossSpawn`** | **DERIVED from seed+tick — not transmitted** | The boss spawn tick is fixed by the deterministic schedule (the ~15:00 transition); no event is needed. |
| **`JoinAccept` / `LobbyState` / `Ready`** | **Off-wire — handled by the in-process `Lobby`** | Pre-match slots, names, ready flags, host id, and ruleset/`content_hash` live in `net::lobby::Lobby` (`sim-core/crates/net/src/lobby.rs`, `docs/07 §7.3`), fed by the Steam matchmaking adapter. They are not `Msg` variants; only the resulting `MatchStart` crosses the wire. |
| **`LeaderboardDelta`** | **Planned — not yet on the wire** | Standings are derived from `DeathConfirmed`/`MatchResult` today; a periodic delta telemetry message is a future addition (§4.4.4). |
| **`SnapshotRequest`** | **Planned — not yet on the wire** | The director currently pushes corrective `Snapshot`s; a client-initiated request is a future addition (§4.4.6). |

## 4.5 Sequence diagrams

> These diagrams show the **conceptual end-to-end flow**, including steps that
> are *not* wire `Msg`s: lobby/`Ready`/`JoinAccept` exchanges run through the
> off-wire `Lobby` layer (§4.4.7), and `RoundStart` / `ShopOffer` /
> `LeaderboardDelta` are shown where the corresponding *derived* (seed+tick) or
> *planned* event occurs, not as transmitted packets. Only the messages in the
> §4.4.0 table actually cross the wire.

### Match start & a round
```
Client(P3)                         Match Director                     Shadow-sim(P3)
   │   JoinRequest ───────────────────►│                                   │
   │◄─────────────── JoinAccept        │                                   │
   │   Ready ─────────────────────────►│                                   │
   │◄──── LobbyState / MatchStart(commit)                                  │
   │◄──── TimeBeacon (x N) ────────────│  (clock sync converges)           │
   │◄──── RoundStart{round12, spawn_seed} ─────────────────► spawn wave ───►│  spawn wave
   │  ...local sim runs at 30Hz from spawn_seed...                          │ ...same...
   │◄──── ShopOffer{round12}           │                                   │
   │   Input{BuyOffer slot2} ─────────►│ validate gold/offer ─────────────►│ apply@tick
   │◄──── InputAck{apply_tick,gold}    │                                   │
   │   Digest{checksum} ──────────────►│ compare vs shadow  (match → ok)   │
   │◄──── LeaderboardDelta             │                                   │
```

### Desync correction (one arena, invisible to others)
```
Client(P3)                         Match Director
   │   Digest{checksum=AAAA} ─────────►│  shadow checksum=BBBB  → MISMATCH
   │◄──── Snapshot{arena_state,rng} ───│  (BULK)        suspicion++ 
   │   load + replay inputs ──────────►│  resume; no other arena affected
```

### Death & placement
```
Client(P3)                         Match Director                Other clients
   │   DeathReport{tick} ────────────►│ confirm vs shadow (HP≤0 @ tick)
   │◄──── DeathConfirmed{place=4} ◄───┤────── DeathConfirmed (broadcast) ──►│
   │                                  │ if alive_remaining ≤1 → MatchResult │
```

### Reconnect
```
Client(P3) [dropped, rejoining]       Match Director
   │   JoinRequest{match_id,player_id}►│  (arena kept running on shadow)
   │◄──── Snapshot{tick=now, rng, input_log_from} (BULK)
   │   replay → caught up ────────────►│  resume live; TimeBeacon/RoundStart resume
```

## 4.6 Validation & error handling

- **Input validation** at ordering time (gold, offer availability, reroll count, cooldown). The implemented `InputAck` carries only `(seq, apply_tick)`; a rejected input is simply *not applied* and the client reconciles against the authoritative `Snapshot`/digest. (The planned `result`/`gold_after`/`rerolls_after` echo, for tighter optimistic-UI rollback, is a future `InputAck` extension — §4.4.3.)
- **Checksum mismatch** → server pushes a `Snapshot`; bumps suspicion. Thresholded suspicion → kick (disconnect handled by the transport/lobby layer).
- **Version/content mismatch** → rejected at `Join` (`content_hash`), eliminating WC3's #1 desync cause before the match starts.
- **Lost TELEMETRY** is fine (newest-wins); lost CONTROL is retransmitted by the reliability layer; lost BULK blocks only the requesting client's correction, not the match.
- **Idempotency**: inputs are keyed by `(player_id, seq)`; duplicates are acked but applied once.

## 4.7 Optimistic local execution (feel)

To keep the UI responsive, the client may **optimistically** apply its own inputs locally the instant they're issued (deduct gold, show the new weapon) and reconcile to `InputAck`. Because inputs are cheap and the server almost always accepts a legal action, mispredictions are rare and limited to the player's own arena. This is *prediction of one's own inputs*, not speculative simulation of others — there are no "others" in your arena to mispredict.
