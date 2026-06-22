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

/// One participant's endpoint onto the message bus — the seam every transport
/// implements. The director and each client own a `Transport`: each driver
/// iteration they `send` their [`Outbound`]s (addressed by `to`) and `poll` the
/// [`Inbound`]s that have arrived for them. Director/Client logic is written
/// against this trait, never against a concrete transport, so the bus is a
/// drop-in choice:
///
/// - **Tests**: [`super::hub::HubEndpoint`] over the deterministic in-process
///   [`super::hub::Hub`] (latency/stall injection, reproducible ordering).
/// - **Production**: the Steam `ISteamNetworkingSockets` / SDR adapter
///   (`super::steam`, `docs/07`) — addressed by SteamID, NAT/relay/crypto by SDR.
///
/// Channel→delivery semantics (reliable/unreliable) are the transport's job;
/// for Steam they map to `k_nSteamNetworkingSend_*` (`super::steam::steam_send_flags`).
pub trait Transport {
    /// Queue this participant's outbound messages for delivery.
    fn send(&mut self, outs: Vec<Outbound>);
    /// Take all inbound messages that have arrived for this participant so far.
    fn poll(&mut self) -> Vec<Inbound>;
}
