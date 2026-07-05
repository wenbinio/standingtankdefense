//! M2 networking: an authoritative **match director** + thin **clients** that
//! communicate ONLY via messages over an abstract transport.
//!
//! In production the transport is Steam `ISteamNetworkingSockets` over SDR
//! (`docs/07`); for tests it is a deterministic in-process [`hub::Hub`] with
//! latency/stall injection. Director and client never call each other directly —
//! they exchange [`wire::Msg`] — so they are independently implementable and the
//! whole thing is deterministically testable without Steam.
//!
//! ## Input-timing invariant (shared protocol rule — do not diverge)
//! An input the director receives at server tick `D` is scheduled to APPLY at
//! tick `D + INPUT_LEAD_TICKS` on BOTH the client and the director's per-player
//! shadow-sim. The director is authoritative for `apply_tick` and returns it in
//! the `InputAck`. Each side keeps a `tick -> action` schedule and, when
//! stepping tick `T`, applies the action scheduled for `T` (else `Noop`). This
//! keeps a healthy client bit-identical to its shadow; a stalled client that
//! misses the window is brought back by a `Snapshot` correction (`docs/03 §3.9`).
//!
//! Director and every client advance exactly one sim tick per driver iteration,
//! both starting at tick 0 on `MatchStart`, so their tick counters stay equal.

pub mod client;
pub mod director;
pub mod hub;
pub mod lobby;
pub mod replay;
pub mod results;
pub mod schedule;
pub mod steam;
pub mod transport;
pub mod wire;

pub use lobby::{JoinReject, Lobby, MatchPlan, Member, Phase, Ruleset, StartReject, MAX_PARTY};
pub use replay::{verify, Capture, ClaimedResult, Replay, VerifyFail, VerifyOutcome};
pub use results::{verify_submissions, MatchStats, PlayerStats, SubmittedResult};
pub use schedule::Schedule;
pub use transport::{Channel, Inbound, Outbound, PeerId, Transport, DIRECTOR};
pub use wire::{GameSpeed, InputCode, Msg, WireError};

/// Input delay in ticks (see invariant above). Must exceed normal one-way
/// delivery delay so a healthy client schedules an action before it is due.
pub const INPUT_LEAD_TICKS: u32 = 8;
/// Clients emit a `Digest` every this many ticks; the director validates it
/// against its shadow-sim and corrects via `Snapshot` on mismatch.
pub const DIGEST_INTERVAL: u32 = 30;
/// The director emits a `TimeBeacon` every this many ticks.
pub const BEACON_INTERVAL: u32 = 15;
/// Driver iterations before arena tick 0. Gives `MatchStart` time to reach every
/// client so all arenas begin stepping on the same global tick. Arena tick =
/// `iter - START_LEAD` (arenas step only once `iter >= START_LEAD`). Every
/// participant is constructed before the driver loop and ticked once per
/// iteration, so their `iter` (and thus arena tick) stay equal regardless of
/// message latency.
pub const START_LEAD: u32 = 4;
