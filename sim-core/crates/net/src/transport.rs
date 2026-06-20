//! Transport abstraction. Director/client produce [`Outbound`] messages and
//! consume [`Inbound`] messages; a transport (the test [`super::hub::Hub`] or a
//! Steam-sockets adapter) routes them. Channels map to Steam send-flags
//! (`docs/04 §4.1` / `docs/07 §7.2`).

/// A network participant. By convention the director is [`DIRECTOR`] (peer 0);
/// players are peers 1..=N.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PeerId(pub u32);

/// The match director's peer id.
pub const DIRECTOR: PeerId = PeerId(0);

/// Delivery class. Maps to Steam reliable/unreliable lanes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    /// Reliable, ordered: lifecycle, seeds, inputs+acks, death/placement.
    Control,
    /// Unreliable, newest-wins: time beacons, digests, leaderboard.
    Telemetry,
    /// Reliable, large, rare: snapshots.
    Bulk,
}

/// A message arriving at a participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inbound {
    pub from: PeerId,
    pub channel: Channel,
    pub bytes: Vec<u8>,
}

/// A message a participant wants to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outbound {
    pub to: PeerId,
    pub channel: Channel,
    pub bytes: Vec<u8>,
}
