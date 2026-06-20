# 06 — Roadmap, Risks & Testing

How to build Standing Tank Defense in de-risking order, what can go wrong (networking-weighted), and how we prove the netcode works. Deployment target is a **free Steam app over Steamworks P2P / Steam Datagram Relay** — see [`07-steamworks-integration.md`](07-steamworks-integration.md).

## 6.1 Milestones (vertical slices, netcode-first)

Each milestone is a *playable* slice; we add network surface before content depth, because networking is the expensive-to-retrofit risk.

### M0 — Deterministic single-arena sim (no network) ✅ **DONE**
- Fixed 30 Hz tick; integer/fixed-point combat; one tank, a few weapons, one wave, the offer/shop loop, gold, death.
- **Exit test**: run the same seed + scripted input log twice → **identical `state_checksum`** every tick (the determinism harness, §6.3). This gates everything else.
- **Status:** implemented in [`sim-core/`](../sim-core/) — `determinism` + `sim` + `harness` crates, 48 tests passing, gate `PASS`. A golden-checksum test runs across the CI OS matrix to prove *cross-platform* bit-identical results, not just same-machine reproducibility. Built orchestrator-style: central seams (`ids`/`state`/`content`/`lib`/determinism) hand-authored, behavior modules (`combat`/`waves`, `economy`/`shop`/`input`) implemented by parallel agents against fixed interfaces, integrated centrally.

### M1 — Shadow-sim & checksums in-process
- Run two instances of the sim (the "client" and the "shadow") in one process from the same seed+inputs; diff checksums each tick.
- Build `Snapshot` serialize/deserialize + replay-from-snapshot.
- **Exit test**: inject a deliberate divergence → detected within one digest interval → corrected by snapshot → checksums reconverge.

### M2 — Match director + transport (Steam)
- Stand up the match director (lobby, clock/time beacons, seed schedule, input ordering/ack, leaderboard, death/placement) over **SteamNetworkingSockets** ([`07`](07-steamworks-integration.md)).
- 2 players, 2 arenas, host-authoritative; CONTROL/TELEMETRY/BULK channels mapped to Steam reliable/unreliable lanes.
- **Exit test**: two clients play independent arenas; one's lag/stall never affects the other's tick rate (the core thesis, §6.3 latency injection).

### M3 — 8-player lobby, reconnect, host migration
- Full 8 slots; Steam lobby + SDR relay; reconnect (re-seed + replay); host migration to a hot standby.
- **Exit test**: kill the host mid-match → director role migrates → all 8 arenas continue; a dropped client rejoins and catches up to the live tick.

### M4 — Content depth & balance
- Import the extracted catalog (`research/.../catalog.json`): full weapon/modifier/enemy/wave set, rarities, status systems (Poison/Frost/Fire/Spikes), Mana Shield, Samwise boss, `Clear`.
- Steam stats/leaderboards/achievements for placement, Last Stand, and the challenge/score meta.

### M5 — Hardening & ship
- Anti-cheat posture for a free P2P game (§6.2, [`07 §7.6`](07-steamworks-integration.md)), soak/chaos tests, Steam release (free).

## 6.2 Risk register (networking-weighted)

| # | Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- | --- |
| R1 | **Non-determinism** in the sim → constant checksum mismatches → snapshot spam | High | High | Integer/fixed-point hot path; determinism harness in CI (M0 gate); banned-ops lint; stable iteration order ([`05 §5.6.1`](05-data-model.md)) |
| R2 | **Host-authority trust** (free P2P: host could cheat its own arena/placement) | Med | Med | Honest scoping: casual has low incentive; Steam auth tickets prevent impersonation; optional community **dedicated** servers for ranked; replay submission + server-side sanity checks for leaderboards ([`07 §7.6`](07-steamworks-integration.md)) |
| R3 | **Late-game swarm** tanks the host's CPU running N shadow-sims | Med | High | Sim is light & deterministic; spot-check validation (full re-sim only on suspicion/placement); entity caps & culling; budget per arena ([`03 §3.9`](03-network-architecture.md)) |
| R4 | **Host migration** loses match state | Med | High | Director state is tiny + replicated to hot standby; arenas are seed+input derived so they survive migration ([`03 §3.10`](03-network-architecture.md)) |
| R5 | **NAT / connectivity** failures between peers | Med | Med | Steam Datagram Relay (SDR) routes via Valve's backbone — no port-forwarding, NAT punch handled; relay fallback always available ([`07 §7.2`](07-steamworks-integration.md)) |
| R6 | **Content/version drift** between players → desync | Low | High | `content_hash` gate at join ([`04 §4.4.1`](04-protocol-and-messages.md)); Steam ensures a single shipped build |
| R7 | **Snapshot bandwidth** spikes if corrections are frequent | Low | Med | Corrections are per-*one*-arena & rare if R1 controlled; delta-encode snapshots; cap correction rate, escalate to kick |
| R8 | **Float determinism across CPUs/compilers** | Med | Med | Avoid floats in sim entirely (fixed-point); if unavoidable, pin rounding & forbid fast-math; the pairwise-determinism + snapshot net makes residual drift survivable ([`03 §3.3`](03-network-architecture.md)) |

## 6.3 Test plan (proving the netcode)

### Determinism harness (the foundation, CI-gated)
- Replay engine: `(seed, input_log) → checksum_trace`. Same inputs must yield byte-identical traces across runs, OSes, and CPUs. Cross-platform CI matrix (Win/Linux/Mac) runs it on every commit. A divergence **fails the build** (R1/R8).

### Latency & chaos injection (the thesis test)
- A network-shim layer injects per-link **latency, jitter, loss, reorder, and stalls** on the Steam channels.
- **Core thesis assertion**: a 2 s stall or 300 ms RTT on player A's link leaves player B's local tick rate unchanged (no cross-arena coupling — the WC3 failure we're avoiding). Automated.
- Pathological cases: host on a bad link; one client at 40% packet loss; mid-match host kill.

### Reconnect / migration tests
- Drop & rejoin at random ticks → client catches up to live tick with matching checksum.
- Kill host at random ticks → migration completes, all arenas continue, placement still correct.

### Soak & scale
- 8-arena, full-content, 20-minute matches running for hours; watch for memory growth, checksum drift, snapshot-rate creep, host CPU under late-game swarm (R3).

### Anti-cheat / validation tests
- Forge illegal inputs (buy without gold, reroll past limit, Clear on cooldown) → rejected at ordering.
- Tampered client checksum → snapshot correction → suspicion → kick threshold.
- Placement/death always fully re-validated by the authority.

## 6.4 Recommended stack (concrete, revisable)

| Layer | Recommendation | Why |
| --- | --- | --- |
| Transport | **Steamworks `ISteamNetworkingSockets`** (+ SDR relay) | Free routing over Valve's backbone, NAT/encryption/auth built in; perfect for a free P2P game ([`07`](07-steamworks-integration.md)) |
| Matchmaking/lobby | **Steam lobbies** (`ISteamMatchmaking`) | No backend to host |
| Identity/anti-impersonation | **Steam auth tickets**, VAC-eligible | Free; ties placement to a SteamID |
| Meta (leaderboards/stats/achievements) | **Steam Stats & Leaderboards** | Last Stand / placement / challenge score, server-side stored |
| Save (challenge meta) | **Steam Cloud** | Replaces the WC3 "codeless save/load" hack outright |
| Sim language | determinism-friendly (Rust/C++/C#-with-fixed-point) | Bit-stable cross-platform sim (R1/R8) |
| Engine | **Godot 4 + Rust sim core (locked)** ([`08`](08-engine-choice.md)) | Sim is an engine-independent Rust crate; Godot renders; Steam via GodotSteam |

## 6.5 Open questions

1. **Disconnect grace policy** — when a client drops, does their arena keep running on the host shadow (can still place) or freeze? Per-match toggle? ([`03 §3.7`](03-network-architecture.md))
2. **Ranked vs. casual authority** — ship casual as host-authoritative P2P; offer optional **dedicated/community servers** for a trusted ranked mode? (R2)
3. **Snapshot cadence** — fixed interval vs. on-demand-only; tune against determinism confidence after M1.
4. **Co-op / shared-arena mode** — explicitly out of v1 (re-introduces cross-arena coupling, [`02 §2.9`](02-game-design.md)); revisit post-ship.
5. **Cross-arena interaction** ("send a creep to a rival") — would couple sims and break the bandwidth/independence guarantees; deliberately excluded, noted for a possible separate mode.
6. **Map script** — first-hand decompile of the protected `war3map` script (wave timings, exact economy tick) would refine the *(inferred)* flow numbers ([`research/tower-survivors-map/README.md`](../research/tower-survivors-map/README.md)).
