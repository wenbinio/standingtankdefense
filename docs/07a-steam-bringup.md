# 07a — Steam bring-up runbook

Companion to [`07`](07-steamworks-integration.md). Walks the **§7.7 integration
checklist** item-by-item and, for each, states (a) the **code location** that
expresses it (or that it is TODO) and (b) what still **requires a real Steam
environment** to finish — App ID registration, a vendored Steamworks SDK, a
running Steam client, and a multi-peer live test.

The production transport lives **outside** the `sim-core/` workspace at
[`adapters/steam-transport/`](../adapters/steam-transport/) — the same isolation
`godot/rust` uses — so the zero-dependency offline CI gate
(`cargo test --workspace` from `sim-core/`) is unaffected. That crate needs the
`steamworks` crate (fetched from crates.io) and the Steamworks SDK redistributable
(linked by `steamworks-sys`): in a truly offline / no-SDK environment its
`cargo build` fails at the `steamworks` dependency — never in the adapter's own
code. (Where those prerequisites ARE present, it builds clean and its unit tests
pass — see "How to verify" below.)

## Honest status summary

| Layer | State |
| --- | --- |
| `Channel → send flags` mapping | **Done**, single source of truth in `net::steam::steam_send_flags`, re-exported by the adapter. |
| `Transport` send/poll/decode core | **Done** in `SteamTransport` (against `steamworks` 0.11 API). |
| PeerId ↔ SteamID/connection map | **Done** (`peers` + `by_steam`, slot = lobby index). |
| Host listen socket + accept loop | **Done** (poll-group drain marked TODO pending live SDK method name). |
| Client connect + host migration reconnect | **Done** (`new_client`, `migrate_to_host`). |
| `SteamAPI_Init` / relay warmup | **Done** (`SteamBootstrap::init`). |
| Lobby create/join/data | **TODO — needs real Steam** (see below). |
| Auth tickets (`GetAuthSessionTicket`/`BeginAuthSession`) | **Sketched** (`bind_player_identity`), runtime wiring TODO. |
| Connection-status callbacks → reconnect | **Sketched** (`on_connection_status_changed`), registration TODO. |
| Stats/leaderboards/Cloud | **TODO** (out of transport scope; UI/meta layer). |
| Replay capture for re-sim | **Exists in sim/net** (seed+input_log); Steam submit is TODO. |

## §7.7 checklist, item by item

### 1. Link the Steamworks SDK; init `SteamAPI_Init`; ship `steam_appid.txt`
- **Code**: `SteamBootstrap::init()` in `adapters/steam-transport/src/lib.rs`
  calls `Client::init()` (reads `steam_appid.txt`) and warms SDR relay access.
  Placeholder App ID files: [`steam_appid.txt`](../steam_appid.txt) (repo root)
  and [`godot/steam_appid.txt`](../godot/steam_appid.txt), both `480` (Spacewar).
  Which ships: the **Godot-export** copy — see
  [`godot/STEAM_APPID.README.md`](../godot/STEAM_APPID.README.md).
- **Needs real Steam**: a **vendored Steamworks SDK** for `steamworks-sys` to
  link (set `STEAM_SDK_LOCATION` / vendor per the crate README); the registered
  **free App ID** on the partner site to replace `480`; a running Steam client
  for `Client::init()` to succeed.

### 2. Lobby create/join/list + lobby data (host SteamID, content_hash, ruleset, ready)
- **Code**: **TODO**. The transport intentionally does NOT own matchmaking — it
  is handed a host `SteamId` (`SteamTransport::new_client(client, host)`). Lobby
  management (`ISteamMatchmaking::CreateLobby`/`JoinLobby`, `SetLobbyData`) belongs
  in the Godot/UI layer, which discovers the host SteamID and passes it here.
- **Needs real Steam**: `ISteamMatchmaking` against a live Steam client;
  multi-peer test to confirm lobby data (host SteamID, `content_hash`, ruleset,
  ready flags) round-trips. No offline stand-in.

### 3. `ISteamNetworkingSockets` connections + Poll Group on host; channel → send flags
- **Code**: `SteamTransport::new_host` opens the P2P listen socket;
  `accept_pending_connections` accepts peers and assigns lobby slots;
  `new_client` opens the single connection to the host; `Transport::send`/`poll`
  run the adapter loop. Channel→flags via `send_flags_for` → `net`'s
  `steam_send_flags` (single source of truth).
- **Needs real Steam**: confirm the exact `steamworks` 0.11 **poll-group** method
  names (`create_poll_group` / `receive_messages_on_poll_group`) against the
  installed SDK — the code currently drains per-connection (semantically
  identical) with a TODO at the poll-group call site. SDR connectivity + a
  multi-peer live test.

### 4. Auth tickets: `GetAuthSessionTicket` (client) / `BeginAuthSession` (host) → bind player_id ↔ SteamID
- **Code**: `SteamTransport::bind_player_identity` is the named touchpoint;
  ticket bytes flow as a CONTROL message in the `[04]` handshake.
- **Needs real Steam**: wire `client.user().authentication_session_ticket()` and
  `begin_authentication_session(...)`, then resolve the
  `AuthSessionTicketResponse` / `ValidateAuthTicketResponse` callbacks. Requires a
  live Steam client and two real SteamIDs to validate end-to-end.

### 5. Director runs the [04] protocol over Steam messages; shadow-sims per arena
- **Code**: **Done in `net`** (`director.rs`/`client.rs`), transport-agnostic.
  Swapping `HubEndpoint` for `SteamTransport` is the only change — both implement
  `net::Transport`. No protocol code changes for Steam.
- **Needs real Steam**: a live multi-peer run to confirm the director services
  all peers through one receive loop with real latency/jitter.

### 6. Host migration: standby replication + lobby-owner-change + socket reconnect (§7.5)
- **Code**: `SteamTransport::migrate_to_host(new_host)` tears down the old
  director connection and reconnects to the new host SteamID. Standby state
  replication is a `net`/director concern (small: seeds, input logs, alive/dead).
- **Needs real Steam**: the `LobbyChatUpdate` / lobby-owner-change callback to
  detect the new owner and call `migrate_to_host`; a 3+ peer test that kills the
  host mid-match and confirms arenas keep running through the reconnect.

### 7. Stats/leaderboards/achievements + Steam Cloud for the meta save
- **Code**: **TODO**, out of transport scope (`ISteamUserStats` /
  `ISteamRemoteStorage` belong in the meta/UI layer).
- **Needs real Steam**: leaderboards/achievements configured on the partner site;
  live Steam client to write/read.

### 8. Replay (`seed`+`input_log`) capture for leaderboard sanity re-sim (§7.6)
- **Code**: the sim is deterministic and `net` already carries seeds + per-player
  input logs (the re-sim input). **Capture exists**; the Steam **submission**
  (attach replay to the leaderboard write) is **TODO** with item 7.
- **Needs real Steam**: leaderboard write API + a server/community re-sim harness.

### 9. Connectivity callbacks → reconnect path (04 §4.4.6); graceful "host left" UX
- **Code**: `SteamBootstrap::on_connection_status_changed` (sketch) +
  `SteamBootstrap::run_callbacks` (pump). On a director-connection problem the
  client surfaces "host left" and may call `migrate_to_host`.
- **Needs real Steam**: register `NetConnectionStatusChanged`, exercise a real
  disconnect to confirm the transition fires and the UX path triggers.

## Build / run requirements (consolidated)

1. **Vendored Steamworks SDK** — required for `steamworks-sys` to link. Without
   it, `cargo build` in `adapters/steam-transport` fails at the `steamworks`
   dependency. This is the ONLY reason it fails to build offline.
2. **Network access** — to fetch `steamworks`/`steamworks-sys` from crates.io.
3. **Registered free App ID** — replace `480` in both `steam_appid.txt` copies.
4. **Running Steam client** — for `SteamAPI_Init` and all live API calls.
5. **Multi-peer live test** — host + ≥2 clients over SDR to validate connect,
   per-channel delivery, auth binding, and a forced host migration.

## How to verify the offline gate is untouched

```
cd sim-core && cargo test --workspace   # must stay green; adapter is outside it
```

The adapter is built separately and is NOT part of that gate:

```
cd adapters/steam-transport && cargo build && cargo test
```

- With the `steamworks` crate + SDK available, this builds clean and runs the
  adapter's pure-logic unit tests (lane/frame round-trip, flags-match-`net`).
- In a truly offline / no-SDK environment it fails at the `steamworks`
  dependency (fetch/link) — never in the adapter's own code. That failure mode is
  expected and is the ONLY reason it would not build.
