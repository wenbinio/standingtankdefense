# 07 — Steamworks Integration (free P2P deployment)

The shipping target is a **free Steam app**. That has one dominant consequence for the architecture: **there is no server budget**, so we cannot lean on paid dedicated servers in the common case. Steamworks lets us keep the [`03`](03-network-architecture.md) authority model *without* paying for servers, by running the authoritative role on a **player's host machine** but routing everything over **Valve's relay backbone (Steam Datagram Relay)** so it behaves like a hosted server for connectivity, NAT traversal, encryption, and DDoS protection — all free to us.

This doc maps the abstract architecture onto concrete Steamworks APIs and reconciles the authority model for a free P2P game.

## 7.1 Deployment model: host-authoritative over SDR (free), dedicated optional

[`03`](03-network-architecture.md) specified "authoritative server, sharded sims." For the free release we bind "the authority" to a **host (listen-server) player** rather than a rented box:

```
   ┌─────────────────────────── Steam Lobby (ISteamMatchmaking) ───────────────────────────┐
   │   lobby metadata · member list · ready flags · "connect-to" host SteamID               │
   └──────────────────────────────────────────────────────────────────────────────────────┘
                                          │ join
   Player B ───┐                          ▼                          ┌─── Player H = HOST
   Player C ───┼──── ISteamNetworkingSockets / SDR relay ────────────┤  runs Match Director
   Player … ───┘     (Valve backbone: NAT punch, encrypt, auth)      │  + N shadow-sims
                                                                      └────────────────────
```

- The **host runs the match director and the per-arena shadow-sims** ([`03 §3.4/§3.9`](03-network-architecture.md)). This is affordable precisely because that work is light and deterministic — the heavy rendering/simulation each *client* does for its own arena is local.
- All peer↔host traffic goes through **`ISteamNetworkingSockets`**, which by default uses **SDR**: connections are addressed by **SteamID**, not IP, so there's no port-forwarding, NAT is punched/relayed by Valve, traffic is encrypted and authenticated, and the host's real IP is hidden (DDoS resistance).
- **Optional dedicated/community servers** (running the same director binary headless) remain a drop-in for a trusted **ranked** mode (R2 in [`06`](06-roadmap-risks-testing.md)); the protocol is identical — only *who* hosts the director changes.

> This is the inverse of [`03`](03-network-architecture.md)'s "dedicated primary, P2P fallback" framing: for the **free** product the primary is **host-authoritative-over-SDR**, dedicated is the optional upgrade. The authority *model* (server-authoritative meta + shadow-sim validation + sharded arenas) is unchanged — only the host of that authority moves.

## 7.2 API mapping

| Architecture concept ([`03`](03-network-architecture.md)/[`04`](04-protocol-and-messages.md)) | Steamworks API | Notes |
| --- | --- | --- |
| Transport, channels | **`ISteamNetworkingSockets`** (connections + **Poll Groups**) | Per-peer connection; messages tagged reliable/unreliable |
| Easy P2P alt | `ISteamNetworkingMessages` | Sendto-by-SteamID convenience layer over the same stack |
| Relay / NAT / encryption | **Steam Datagram Relay (SDR)** | Free routing over Valve backbone; addresses are `SteamNetworkingIdentity` |
| Lobby / matchmaking | **`ISteamMatchmaking`** (lobbies) | Public/friends/private; lobby data holds host SteamID, settings, ready state |
| Identity / anti-impersonation | **`GetAuthSessionTicket` / `BeginAuthSession`** | Binds a `player_id` to a verified SteamID; VAC-eligible |
| Leaderboards / stats / achievements | **`ISteamUserStats`** | Placement, Last Stand, kills, challenge score |
| Save (challenge meta) | **`ISteamRemoteStorage`** / Auto-Cloud | Replaces the WC3 "codeless save/load" hack ([`01 §1.6`](01-source-analysis.md)) |
| Rich presence / invites | `ISteamFriends` | "Join game" from friends list |

### Channel mapping
The three logical channels from [`04 §4.1`](04-protocol-and-messages.md) map onto Steam message send flags:

| Logical channel | Steam send flag | Used for |
| --- | --- | --- |
| CONTROL (reliable, ordered) | `k_nSteamNetworkingSend_Reliable` | lifecycle, seeds, inputs+acks, death/placement |
| TELEMETRY (unreliable, newest-wins) | `k_nSteamNetworkingSend_Unreliable` (+ `NoNagle` for beacons) | time beacons, digests, leaderboard deltas |
| BULK (reliable, large, rare) | `k_nSteamNetworkingSend_Reliable` (chunked) | snapshots, content manifest |

Use a **lane / channel id** per logical channel and a single Poll Group on the host so the director services all peers in one receive loop.

## 7.3 Match lifecycle on Steam

```
1. Host creates a Steam lobby (ISteamMatchmaking::CreateLobby).
2. Players find/join via friends, invites, or a public lobby list.
3. Lobby data carries: host SteamID, map/ruleset, content_hash, ready flags, game speed.
4. On start, each non-host opens an ISteamNetworkingSockets connection to the host SteamID
   (SDR resolves routing). Host accepts into its Poll Group.
5. Steam auth tickets exchanged → director binds player_id ↔ SteamID.
6. Director runs the [04] protocol: MatchStart → RoundStart(seeds) → inputs/acks →
   digests → death/placement → MatchResult.
7. On host loss → migration (§7.5). On match end → write Steam stats/leaderboards.
```

## 7.4 Why this fits a *free* game (cost & UX)

- **No backend bill**: SDR, lobbies, auth, stats, and cloud are provided by Steam at no per-match cost. The only "server" is a player's PC.
- **No NAT/port-forward support tickets**: SDR handles connectivity — historically the biggest friction for free P2P games.
- **Built-in trust primitives**: SteamID identity + VAC eligibility + ticket auth give a baseline anti-cheat/anti-smurf posture for free.
- **The architecture earns this**: because arenas are independent and the authority work is light ([`03 §3.1`](03-network-architecture.md)), a consumer PC can host 8 shadow-sims — something WC3's all-peers-simulate-everything model could never offload cleanly.

## 7.5 Host migration over Steam

(Mechanics in [`03 §3.10`](03-network-architecture.md).) On Steam specifically:
- The director replicates its **small** state (seeds, per-player input logs, alive/dead, leaderboard) to a designated **hot-standby peer** continuously.
- On host disconnect (detected via connection status callbacks / lobby owner change), Steam transfers **lobby ownership** to the next member; that member promotes its standby director, and peers **reconnect their sockets to the new host SteamID**.
- Each arena keeps running locally throughout (seed+input derived), so migration is a brief director pause, not an arena reset.

## 7.6 Anti-cheat posture for free P2P (honest trade-offs)

A host-authoritative free game **cannot** fully trust the host for *its own* arena — the host could tamper with the sim it runs. We're explicit about this rather than pretending otherwise:

- **What's protected regardless of host**: identity (Steam tickets stop impersonation), *other* players' arenas (each peer's inputs are validated by the host director, and a malicious host can't touch a non-host's local sim beyond what the protocol allows), and connectivity integrity (SDR).
- **What a malicious host could do**: cheat *their own* arena (infinite gold, god-mode) to win placement. Mitigations, in increasing strength:
  1. **Low incentive in casual** — it's a free co-located race; placement carries no economy.
  2. **Deterministic replay submission** — the winner's `(seed, input_log)` is compact; submit it with the leaderboard write so a **server-side (or community) re-sim** can sanity-check that the inputs legally produce the claimed result. Cheated arenas fail re-sim.
  3. **Optional dedicated/community-hosted ranked** — same binary, neutral host; the only mode where placement is authoritative-by-a-trusted-party (R2 in [`06`](06-roadmap-risks-testing.md)).
  4. **VAC** on the shipped build raises the bar on memory tampering.
- **Design alignment**: because the only competitively load-bearing fact is "who died when" ([`02 §2.7`](02-game-design.md)), the *re-sim* check is cheap and targeted — you only need to verify the contested placements, not stream a whole match.

## 7.7 Integration checklist

- [ ] Link the **Steamworks SDK**; init `SteamAPI_Init`; ship `steam_appid.txt` (free App ID).
- [ ] Lobby create/join/list + lobby data for host SteamID, `content_hash`, ruleset, ready flags.
- [ ] `ISteamNetworkingSockets` connections + Poll Group on host; map CONTROL/TELEMETRY/BULK to send flags (§7.2).
- [ ] Auth tickets: `GetAuthSessionTicket` (client) / `BeginAuthSession` (host) → bind `player_id`↔SteamID.
- [ ] Director runs the [`04`](04-protocol-and-messages.md) protocol over Steam messages; shadow-sims per arena.
- [ ] Host migration: standby replication + lobby-owner-change + socket reconnect (§7.5).
- [ ] Stats/leaderboards/achievements for placement, Last Stand, challenge score; Steam Cloud for the meta save.
- [ ] Replay (`seed`+`input_log`) capture for leaderboard sanity re-sim (§7.6).
- [ ] Connectivity callbacks → reconnect path ([`04 §4.4.6`](04-protocol-and-messages.md)); graceful "host left" UX.

## 7.8 References

- Steam Datagram Relay (SDR) overview — https://partner.steamgames.com/doc/features/multiplayer/steamdatagramrelay
- `ISteamNetworkingSockets` — https://partner.steamgames.com/doc/api/ISteamNetworkingSockets
- `ISteamNetworkingMessages` (P2P) — https://partner.steamgames.com/doc/api/ISteamNetworkingMessages
- Steam Matchmaking / lobbies — https://partner.steamgames.com/doc/features/multiplayer/matchmaking
- User Authentication & ownership (auth tickets) — https://partner.steamgames.com/doc/features/auth
- Stats & Leaderboards — https://partner.steamgames.com/doc/features/leaderboards
- Steam Cloud — https://partner.steamgames.com/doc/features/cloud
