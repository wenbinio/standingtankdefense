# CLAUDE.md

Guidance for Claude Code working in this repository.

## Project

**Standing Tank Defense** — a free, multiplayer, last-man-standing, randomized tower-defense / survival game with **one immobile tank** per player, inspired by the WC3 map *Tower Survivors*. Currently a **specification package** (`docs/`), moving toward implementation.

Read these before acting; do not re-derive what they already decide:
- `README.md` — overview + doc index.
- `docs/03-network-architecture.md` — **the centerpiece**; the architecture everything serves.
- `docs/05-data-model.md` — schemas + determinism rules.
- `docs/06-roadmap-risks-testing.md` — milestones (build in this order), risks, test plan.
- `docs/07-steamworks-integration.md` / `docs/08-engine-choice.md` — deployment + stack.
- `research/tower-survivors-map/` — the finalized source-map extraction (don't re-extract).

## Operating model: I am the central (orchestrator) agent

Default to **delegating to subagents**, not doing the work inline. My job is to **scope tightly, dispatch, then own integration and optimization across agents.** I hold the architecture and the seams; agents fill in the pieces.

### Scope every delegated task tightly
A good agent task names all of:
1. **Goal & done-condition** — one outcome, with an objective check (a test passes, a checksum matches, a file exports a named symbol).
2. **Boundaries** — exact files/modules to touch; what *not* to touch; the interface (types/messages) it must conform to.
3. **Invariants it must not break** (see below) and the relevant spec section to follow.
4. **Return contract** — what to report back (diff summary, the public interface it exposed, test output, open questions) so I can integrate without re-reading everything.

Prefer **several small, independent agents over one broad one.** When tasks are independent, dispatch them **in parallel in a single message**. When they share an interface, I define that interface first, then fan out against it. Sequence only on real dependencies.

### What stays central (I do this, not an agent)
- Owning interfaces/contracts **between** components (the sim↔net↔render seams, the `[04]` message schema, the data schemas in `[05]`).
- **Integration**: merging agent outputs, resolving interface drift, making the parts actually compose and build.
- **Optimization across agents**: deduplicating overlapping work, tightening hot paths, removing redundancy an isolated agent couldn't see.
- Final verification, commits, and pushes (see Git below).
- Any cross-cutting decision; never let an agent silently re-decide a locked choice.

### Integration discipline
- Define shared types/messages **before** fanning out work that depends on them.
- After agents return, reconcile their public interfaces against the spec, build/test the combined result, and resolve conflicts myself rather than redelegating blindly.
- Relay only what matters from an agent's result; an agent's final message is for me, not the user.

## Locked decisions — do not silently revisit (flag if you must)

- **Engine:** Godot 4 (render/UI/input) + a **decoupled, deterministic Rust simulation core** (fixed-point) exposed via GDExtension. (`docs/08`)
- **Net model:** server/host-authoritative **match director** + **per-player sharded sims**; ship **inputs + seeds, not entities**. No WC3-style global lockstep, no entity-replication netcode (Mirror/Photon/etc.). (`docs/03`)
- **Deployment:** free Steam app, **host-authoritative over Steam Datagram Relay**; dedicated/community servers are the optional ranked upgrade. (`docs/07`)
- **Build order:** netcode-first milestones M0→M5; M0 (deterministic single-arena sim + checksum harness) gates everything. (`docs/06`)

## Invariants every agent must respect

- **Determinism is sacred.** The sim core: fixed 30 Hz tick, integer/fixed-point math in anything feeding `state_checksum`, seeded PRNG streams only, stable entity iteration order. No wall-clock, no platform RNG, no floats in the hot path. (`docs/05 §5.6`)
- **Sim core stays engine-independent** — it never imports Godot/engine types; the engine only *reads* sim state to render.
- **Authority:** clients/peers are never trusted for anything competitively load-bearing; the director validates inputs and owns RNG/seeds and death/placement.
- **Content is data**, validated against `[05]` schemas and hashed into `content_hash`.

## Git

- Develop on branch `claude/tower-survivors-spec-20q1kk`. Never push elsewhere without explicit permission.
- Commit/push only when work is complete and verified. Use `git push -u origin <branch>`; retry network failures with backoff.
- Do not create PRs unless explicitly asked.
- Keep model identifiers out of commits/PRs/code; chat only.
