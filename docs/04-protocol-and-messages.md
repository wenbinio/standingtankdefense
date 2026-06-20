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

### 4.4.1 Lifecycle (CONTROL)

**`JoinRequest`** (C→S)
```json
{ "type":"JoinRequest", "match_id":"...", "client_version":"1.0.0",
  "content_hash":"sha256:...", "display_name":"..." }
```
`content_hash` gates version/content mismatch up front — the #1 desync cause in WC3 was version drift; here it's a hard pre-match check.

**`JoinAccept`** (S→C)
```json
{ "type":"JoinAccept", "player_id":3, "slot_count":8,
  "tick_rate":30, "server_tick":0, "content_manifest_ref":"BULK:manifest@1" }
```

**`LobbyState`** (S→C, on change) — slots, names, ready flags, host id.

**`Ready`** (C→S) `{ "ready":true }`.

**`MatchStart`** (S→C, broadcast)
```json
{ "type":"MatchStart", "start_tick":150, "master_seed_commit":"sha256:..." }
```
The server broadcasts a **commit** (hash) of the master seed at countdown and reveals derived seeds per round — so no client can precompute the whole match's RNG, but the commit proves the server didn't change it later.

### 4.4.2 Clock & rounds (TELEMETRY for beacon, CONTROL for round)

**`TimeBeacon`** (S→C, 2–4 Hz, unreliable)
```json
{ "type":"TimeBeacon", "server_tick":5400, "server_unix_ns":169... }
```

**`RoundStart`** (S→C, CONTROL)
```json
{ "type":"RoundStart", "round":12, "start_tick":21600,
  "wave_table_id":"wave_12",
  "spawn_seed":"a3f1...",            // this player's spawn stream seed for round 12
  "scaling_step":"post_10min" }
```
Per-player `spawn_seed` (same wave table, individual jitter). Scaling steps (`base`/`post_10min`/`post_15min`) are carried so the sim applies the right multipliers deterministically.

**`BossSpawn`** (S→C, CONTROL) `{ "round":31, "spawn_tick":..., "boss_id":"samwise" }`.

### 4.4.3 Player inputs (CONTROL, reliable-ordered)

A single envelope covers every player action; `action` is a tagged union. Inputs affect only the sender's arena.

**`Input`** (C→S)
```json
{ "type":"Input", "player_id":3, "seq":87, "client_tick":21750,
  "action": { "kind":"BuyOffer", "offer_slot":2 } }
```
`action.kind` ∈:
- `BuyOffer { offer_slot }` — purchase the weapon/upgrade in a shop slot.
- `Reroll {}` — refresh offers (consumes a reroll / charges gold).
- `UseItem { item_id }` — Magic Treasure / Multiplication Gems / Black Market pick.
- `BlackMarketPick { weapon_id }` — choose the specific weapon (stateful offer).
- `Clear {}` — fire the manual `Clear` ability (the one real-time combat input; also the boss-damage action).
- `SetGameSpeed { speed }` — host (slot 0) only; server validates and re-broadcasts.

**`InputAck`** (S→C)
```json
{ "type":"InputAck", "seq":87, "apply_tick":21752, "result":"ok",
  "gold_after":1450, "rerolls_after":3 }
```
The server stamps the **authoritative `apply_tick`** (so client and shadow apply the input at the same tick) and returns authoritative economy fields. `result` ∈ `ok | rejected(reason)`; rejection reasons: `insufficient_gold`, `offer_not_available`, `no_rerolls`, `on_cooldown`, `illegal`.

**`ShopOffer`** (S→C, CONTROL) — the server-gated offer set for the current shop (so clients can't precompute):
```json
{ "type":"ShopOffer", "round":12, "shop_seq":4,
  "offers":[ {"slot":0,"id":"wpn_frost_bow","rarity":"uncommon","cost":1500},
             {"slot":1,"id":"upg_chaos_dmg_10","rarity":"common","cost":500} ],
  "reroll_cost":300 }
```

### 4.4.4 Liveness, digests, leaderboard (TELEMETRY, unreliable)

**`Digest`** (C→S, 2–5 Hz) — heartbeat + drift check
```json
{ "type":"Digest", "player_id":3, "tick":21900,
  "hp":18500, "max_hp":24000, "gold":1450, "kills":2104,
  "wave":12, "state_checksum":"7f3a9c01" }
```
`state_checksum` is a rolling hash of the arena's authoritative-relevant state; the server compares it to its shadow-sim (§4.6).

**`LeaderboardDelta`** (S→C, 1–2 Hz) — changed standings only
```json
{ "type":"LeaderboardDelta", "tick":21900,
  "entries":[ {"player_id":5,"alive":false,"place":8,"died_tick":21010,"wave":9},
              {"player_id":3,"alive":true,"survival_ticks":21900,"kills":2104} ] }
```

### 4.4.5 Death, placement, resolution (CONTROL, reliable)

**`DeathReport`** (C→S) — client reports its own death `{ "tick":22050, "cause":"overrun" }`. Advisory; the server confirms against its shadow.

**`DeathConfirmed`** (S→C, broadcast)
```json
{ "type":"DeathConfirmed", "player_id":3, "died_tick":22050, "place":4,
  "alive_remaining":3 }
```

**`MatchResult`** (S→C, broadcast)
```json
{ "type":"MatchResult",
  "placements":[ {"player_id":7,"place":1,"last_stand":true,"killed_boss":true},
                 {"player_id":3,"place":4} ],
  "win_cutoff_place":4 }     // places ≤ cutoff are wins (top half)
```

### 4.4.6 Snapshots & reconnect (BULK, reliable)

**`SnapshotRequest`** (C→S) `{ "reason":"reconnect" | "desync" }`.

**`Snapshot`** (S→C) — authoritative full state of **one** arena at a tick, plus the data to continue deterministically:
```json
{ "type":"Snapshot", "player_id":3, "tick":22000,
  "arena_state": { /* serialized authoritative arena */ },
  "rng_cursors": { "spawn":..., "targeting":..., "shop":..., "proc":..., "reroll":... },
  "input_log_from": 88 }   // client replays inputs ≥88 to reach 'now'
```
On reconnect the client loads `arena_state`, restores `rng_cursors`, then replays any inputs from `input_log_from` to catch up to the live tick (§3.7).

## 4.5 Sequence diagrams

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

- **Input validation** at ordering time (gold, offer availability, reroll count, cooldown) → `InputAck.result=rejected(reason)`; client rolls back its optimistic UI to the authoritative `gold_after`/`rerolls_after`.
- **Checksum mismatch** → server pushes a `Snapshot`; bumps suspicion. Thresholded suspicion → kick with `Disconnect{reason}`.
- **Version/content mismatch** → rejected at `JoinRequest` (`content_hash`), eliminating WC3's #1 desync cause before the match starts.
- **Lost TELEMETRY** is fine (newest-wins); lost CONTROL is retransmitted by the reliability layer; lost BULK blocks only the requesting client's correction, not the match.
- **Idempotency**: inputs are keyed by `(player_id, seq)`; duplicates are acked but applied once.

## 4.7 Optimistic local execution (feel)

To keep the UI responsive, the client may **optimistically** apply its own inputs locally the instant they're issued (deduct gold, show the new weapon) and reconcile to `InputAck`. Because inputs are cheap and the server almost always accepts a legal action, mispredictions are rare and limited to the player's own arena. This is *prediction of one's own inputs*, not speculative simulation of others — there are no "others" in your arena to mispredict.
