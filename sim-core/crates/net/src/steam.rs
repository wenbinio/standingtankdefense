//! Steam transport adapter **plan** (`docs/07`). NOT built in this sandbox (no
//! Steam SDK / `steamworks` crate offline), so this module documents — in code —
//! how the `Inbound`/`Outbound` message interface that `Director` and `Client`
//! already use maps onto Steam `ISteamNetworkingSockets` over SDR. The
//! deterministic in-process [`crate::hub::Hub`] is the test-time stand-in for
//! exactly this interface, so the Steam adapter is a drop-in replacement.
//!
//! ## Adapter loop (per driver iteration), on a real Steam build
//! 1. `ISteamNetworkingSockets::ReceiveMessagesOnPollGroup` → decode each into an
//!    [`crate::transport::Inbound`] `{ from: SteamID→PeerId, channel, bytes }`.
//! 2. Hand those to `Director::tick` / `Client::tick` (unchanged).
//! 3. For each returned [`crate::transport::Outbound`], call
//!    `SendMessageToConnection(conn_for(to), bytes, steam_send_flags(channel))`.
//!
//! Identity, lobby discovery, and routing come from Steam (`docs/07 §7.2`):
//! peers are addressed by `SteamNetworkingIdentity` (SteamID), NAT/relay/crypto
//! are handled by SDR, and `PeerId` is just the lobby slot index.

use crate::transport::Channel;

// Mirrors `steamnetworkingtypes.h` `k_nSteamNetworkingSend_*` bit values so the
// mapping is unambiguous when the real adapter is wired up.
pub const STEAM_SEND_UNRELIABLE: i32 = 0;
pub const STEAM_SEND_NO_NAGLE: i32 = 1;
pub const STEAM_SEND_RELIABLE: i32 = 8;

/// Steam send flags for a logical [`Channel`] (`docs/04 §4.1`, `docs/07 §7.2`):
/// Control = reliable/ordered, Telemetry = unreliable newest-wins (+NoNagle so
/// beacons/digests aren't delayed), Bulk = reliable (chunked by Steam).
pub const fn steam_send_flags(channel: Channel) -> i32 {
    match channel {
        Channel::Control => STEAM_SEND_RELIABLE,
        Channel::Telemetry => STEAM_SEND_UNRELIABLE | STEAM_SEND_NO_NAGLE,
        Channel::Bulk => STEAM_SEND_RELIABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_flag_mapping() {
        assert_eq!(steam_send_flags(Channel::Control), STEAM_SEND_RELIABLE);
        assert_eq!(steam_send_flags(Channel::Bulk), STEAM_SEND_RELIABLE);
        let tele = steam_send_flags(Channel::Telemetry);
        assert_eq!(tele & STEAM_SEND_NO_NAGLE, STEAM_SEND_NO_NAGLE);
        assert_eq!(tele & STEAM_SEND_RELIABLE, 0, "telemetry must be unreliable");
    }
}
