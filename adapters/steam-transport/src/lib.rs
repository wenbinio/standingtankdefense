//! Production Steam transport adapter (`docs/07`).
//!
//! This is the real `net::Transport` implementation that the test-time
//! [`net::hub::HubEndpoint`] stands in for. The netcode (`Director` / `Client`)
//! is written ONLY against the [`net::Transport`] trait, so swapping the hub for
//! this adapter is a drop-in: same `send`/`poll` shape, same `Inbound`/`Outbound`
//! messages, same `Channel` semantics. The difference is purely the bus:
//!
//! - **Tests**: deterministic in-process [`net::hub::Hub`] (latency/stall sim).
//! - **Production (this crate)**: `ISteamNetworkingSockets` over Steam Datagram
//!   Relay (SDR) — peers addressed by SteamID, NAT/relay/crypto handled by Valve.
//!
//! ## Adapter loop (matches `net::steam` module docs)
//! Per driver iteration:
//! 1. `poll()` → `ReceiveMessagesOnPollGroup` (host) / `ReceiveMessagesOnConnection`
//!    (client) → decode each Steam message into [`net::Inbound`]
//!    `{ from: SteamID→PeerId, channel, bytes }`.
//! 2. The driver hands those to `Director::tick` / `Client::tick` (unchanged).
//! 3. `send(outs)` → for each [`net::Outbound`], `SendMessageToConnection(
//!    conn_for(to), bytes, steam_send_flags(channel))`.
//!
//! ## Channel recovery on the wire
//! The `steamworks` 0.11 `send_message` does not expose the per-message lane
//! index, and `Channel` (reliable/unreliable + nagle) is carried by the send
//! FLAGS, which the receiver does not see. So the logical channel is framed as a
//! single leading byte ([`Lane`]) on every payload and stripped on decode. This
//! keeps the adapter self-contained; a build on `steamworks` >= 0.13 can switch
//! to the native lane API (`send_message_on_lane` / `NetworkingMessage::channel`)
//! and drop the prefix.
//!
//! ## What is real vs TODO here
//! The message send/poll/decode core, the PeerId↔connection map, the poll group,
//! the channel→flags mapping, host accept and client connect/migration are all
//! REAL against the `steamworks` 0.11 API. The lobby, auth-ticket, and
//! connection-status-callback wiring need a live Steam client to exercise and are
//! marked `// TODO(steam-runtime):` with exactly what they need.

use std::collections::HashMap;

use net::transport::{Channel, Inbound, Outbound, PeerId, Transport};

// SINGLE SOURCE OF TRUTH for Channel -> Steam send flags. We re-export the
// mapping that lives in `net::steam` rather than re-deriving it, so the wire
// semantics can never drift between the test plan and this production adapter.
pub use net::steam::{
    steam_send_flags, STEAM_SEND_NO_NAGLE, STEAM_SEND_RELIABLE, STEAM_SEND_UNRELIABLE,
};

use steamworks::networking_sockets::{
    ListenSocket, NetConnection, NetPollGroup, NetworkingSockets,
};
use steamworks::networking_types::{
    ListenSocketEvent, NetConnectionEnd, NetworkingConfigEntry, NetworkingIdentity,
    NetworkingMessage, SendFlags,
};
use steamworks::{Client, ClientManager, SteamId};

/// Logical lane id framed as the leading byte of every payload so the receiver
/// can recover the [`Channel`] a message was sent on (the send FLAGS that carry
/// reliable/unreliable are invisible to the receiver). See module docs.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lane {
    Control = 0,
    Telemetry = 1,
    Bulk = 2,
}

impl Lane {
    pub fn from_channel(c: Channel) -> Lane {
        match c {
            Channel::Control => Lane::Control,
            Channel::Telemetry => Lane::Telemetry,
            Channel::Bulk => Lane::Bulk,
        }
    }
    pub fn to_channel(self) -> Channel {
        match self {
            Lane::Control => Channel::Control,
            Lane::Telemetry => Channel::Telemetry,
            Lane::Bulk => Channel::Bulk,
        }
    }
    fn from_u8(v: u8) -> Lane {
        match v {
            1 => Lane::Telemetry,
            2 => Lane::Bulk,
            _ => Lane::Control,
        }
    }
}

/// Frame a payload with its [`Lane`] byte for sending.
fn frame(channel: Channel, bytes: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(bytes.len() + 1);
    framed.push(Lane::from_channel(channel) as u8);
    framed.extend_from_slice(bytes);
    framed
}

/// Split a received frame back into `(channel, payload)`. Returns `None` on an
/// empty (malformed) frame.
fn unframe(data: &[u8]) -> Option<(Channel, Vec<u8>)> {
    let (&lane, rest) = data.split_first()?;
    Some((Lane::from_u8(lane).to_channel(), rest.to_vec()))
}

/// Translate `net`'s integer send-flag bitset (the single-source-of-truth
/// [`steam_send_flags`]) into the `steamworks` crate's typed [`SendFlags`].
/// Keeping the bit values in `net` and converting here means the *semantics*
/// (which channel is reliable) live in exactly one place.
fn send_flags_for(channel: Channel) -> SendFlags {
    let bits = steam_send_flags(channel);
    let mut flags = if bits & STEAM_SEND_RELIABLE != 0 {
        SendFlags::RELIABLE
    } else {
        SendFlags::UNRELIABLE
    };
    if bits & STEAM_SEND_NO_NAGLE != 0 {
        flags |= SendFlags::NO_NAGLE;
    }
    flags
}

/// Maximum messages drained from Steam per `poll()`. Bounds work per driver
/// iteration; leftover messages are picked up next poll.
const MAX_RECV_PER_POLL: usize = 256;

/// Per-peer connection bookkeeping. `PeerId` is the lobby slot index
/// (`docs/07 §7.2`); the director is `net::DIRECTOR` (slot 0).
struct PeerLink {
    steam_id: SteamId,
    conn: NetConnection<ClientManager>,
}

/// The production [`Transport`]. One instance per participant — a host instance
/// (accepts connections into a poll group, fans out to many peers) or a client
/// instance (a single connection to the host). Mirrors the per-endpoint shape of
/// [`net::hub::HubEndpoint`].
pub struct SteamTransport {
    sockets: NetworkingSockets<ClientManager>,
    role: Role,
    /// PeerId -> live connection + SteamID. On the host this holds every accepted
    /// peer; on the client it holds the single host entry (keyed `net::DIRECTOR`).
    peers: HashMap<PeerId, PeerLink>,
    /// Reverse map for decode: SteamID -> PeerId, so an arriving message's sender
    /// identity becomes the `from` PeerId the netcode expects.
    by_steam: HashMap<SteamId, PeerId>,
    /// Outbound messages whose target peer isn't connected yet (pre-handshake or
    /// mid-migration). Flushed once the connection appears. Keeps `send()` lossless
    /// across the brief reconnect window (`docs/07 §7.5`).
    pending: Vec<Outbound>,
}

enum Role {
    /// Host: owns the listen socket + poll group; accepts inbound connections and
    /// services every peer in one `receive_messages` loop (`docs/07 §7.2`).
    Host {
        listen: ListenSocket<ClientManager>,
        poll_group: NetPollGroup<ClientManager>,
        /// Next lobby slot to hand a freshly accepted peer (1..=N; 0 is director).
        next_slot: u32,
    },
    /// Client: a single connection to the host SteamID lives in `peers[DIRECTOR]`.
    Client { host: SteamId },
}

impl SteamTransport {
    /// Construct the HOST transport: open a P2P listen socket and a poll group so
    /// the director can service every peer in one receive loop (`docs/07 §7.2`).
    ///
    /// `client` is the live `steamworks::Client` (from [`SteamBootstrap`]).
    pub fn new_host(client: &Client) -> Result<SteamTransport, SteamError> {
        let sockets = client.networking_sockets();

        // P2P listen socket over SDR. `0` = virtual port; the options can carry
        // SDR/lane config (e.g. `configure_connection_lanes` for 3 lanes).
        let cfg: Vec<NetworkingConfigEntry> = Vec::new();
        let listen = sockets
            .create_listen_socket_p2p(0, cfg)
            .map_err(|_| SteamError::ListenSocket)?;
        let poll_group = sockets.create_poll_group();

        Ok(SteamTransport {
            sockets,
            role: Role::Host {
                listen,
                poll_group,
                next_slot: 1,
            },
            peers: HashMap::new(),
            by_steam: HashMap::new(),
            pending: Vec::new(),
        })
    }

    /// Construct the CLIENT transport: open a single connection to the host
    /// SteamID. SDR resolves routing — no IP/port (`docs/07 §7.1`).
    pub fn new_client(client: &Client, host: SteamId) -> Result<SteamTransport, SteamError> {
        let sockets = client.networking_sockets();

        let identity = NetworkingIdentity::new_steam_id(host);
        let cfg: Vec<NetworkingConfigEntry> = Vec::new();
        let conn = sockets
            .connect_p2p(identity, 0, cfg)
            .map_err(|_| SteamError::Connect)?;

        let mut peers = HashMap::new();
        let mut by_steam = HashMap::new();
        peers.insert(
            net::DIRECTOR,
            PeerLink {
                steam_id: host,
                conn,
            },
        );
        by_steam.insert(host, net::DIRECTOR);

        Ok(SteamTransport {
            sockets,
            role: Role::Client { host },
            peers,
            by_steam,
            pending: Vec::new(),
        })
    }

    /// Slot index (PeerId) this transport assigns to `steam_id`. On the host this
    /// is populated as peers are accepted; on the client every message resolves to
    /// the director.
    fn peer_for(&self, steam_id: SteamId) -> Option<PeerId> {
        self.by_steam.get(&steam_id).copied()
    }

    /// HOST: pump the listen socket's connection events, accepting new peers into
    /// the poll group and assigning each a lobby slot (`docs/07 §7.3` step 4).
    ///
    /// Called at the top of [`poll`]. On the client this is a no-op (its single
    /// connection's status is handled via connection-status callbacks).
    fn accept_pending_connections(&mut self) {
        let Role::Host {
            listen,
            poll_group,
            next_slot,
        } = &mut self.role
        else {
            return;
        };
        // `try_receive_event` is non-blocking; drain all queued events.
        while let Some(event) = listen.try_receive_event() {
            match event {
                ListenSocketEvent::Connecting(request) => {
                    // Authority gate: a real build checks the lobby membership /
                    // auth ticket (see BeginAuthSession in `bind_player_identity`)
                    // before accepting. For now accept all SDR P2P requests.
                    // TODO(steam-runtime): reject if SteamID not in the lobby
                    // member list, or if BeginAuthSession failed.
                    let _ = request.accept();
                }
                ListenSocketEvent::Connected(connected) => {
                    let steam_id = connected
                        .remote()
                        .steam_id()
                        .expect("SDR P2P peers always carry a SteamID identity");
                    let conn = connected.take_connection();
                    // Service this peer through the shared poll group so the host
                    // receives from all peers in one loop.
                    conn.set_poll_group(poll_group);

                    let slot = PeerId(*next_slot);
                    *next_slot += 1;

                    self.by_steam.insert(steam_id, slot);
                    self.peers.insert(slot, PeerLink { steam_id, conn });
                }
                ListenSocketEvent::Disconnected(closed) => {
                    // Peer dropped. Remove its mapping; the director treats the
                    // absence as a stalled/lost player and may trigger migration
                    // if the lost peer was the standby (`docs/07 §7.5`).
                    if let Some(steam_id) = closed.remote().steam_id() {
                        if let Some(slot) = self.by_steam.remove(&steam_id) {
                            self.peers.remove(&slot);
                        }
                    }
                }
            }
        }
    }

    /// Decode one Steam message into an [`Inbound`]. Returns `None` if the sender
    /// identity can't be resolved to a known PeerId or the frame is malformed.
    fn decode(&self, msg: &NetworkingMessage<ClientManager>) -> Option<Inbound> {
        let steam_id = msg.identity_peer().steam_id()?;
        let from = self.peer_for(steam_id)?;
        let (channel, bytes) = unframe(msg.data())?;
        Some(Inbound {
            from,
            channel,
            bytes,
        })
    }

    /// Flush any queued outbound messages whose connection is now available.
    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let drained = std::mem::take(&mut self.pending);
        self.send(drained);
    }

    /// MIGRATION hook (`docs/07 §7.5`). On host loss the lobby owner changes; the
    /// new standby-promoted host's SteamID is handed here, and the client
    /// reconnects its director socket to it. Arenas keep running locally
    /// (seed+input derived), so this is a brief director pause, not a reset.
    ///
    /// Only meaningful on a CLIENT transport.
    pub fn migrate_to_host(&mut self, new_host: SteamId) -> Result<(), SteamError> {
        let Role::Client { host } = &mut self.role else {
            return Err(SteamError::WrongRole);
        };
        if *host == new_host {
            return Ok(());
        }

        // Tear down the old director connection.
        if let Some(old) = self.peers.remove(&net::DIRECTOR) {
            self.by_steam.remove(&old.steam_id);
            old.conn
                .close(NetConnectionEnd::AppGeneric, Some("host migration"), false);
        }

        // Open a fresh connection to the new host SteamID.
        let identity = NetworkingIdentity::new_steam_id(new_host);
        let cfg: Vec<NetworkingConfigEntry> = Vec::new();
        let conn = self
            .sockets
            .connect_p2p(identity, 0, cfg)
            .map_err(|_| SteamError::Connect)?;

        *host = new_host;
        self.by_steam.insert(new_host, net::DIRECTOR);
        self.peers.insert(
            net::DIRECTOR,
            PeerLink {
                steam_id: new_host,
                conn,
            },
        );
        Ok(())
    }

    /// Bind `player_id` ↔ verified SteamID via Steam auth tickets
    /// (`docs/07 §7.2`/§7.3 step 5). The CLIENT calls `GetAuthSessionTicket` and
    /// sends the ticket; the HOST calls `BeginAuthSession(ticket, steam_id)` and
    /// only trusts the binding once the auth callback reports success. The ticket
    /// bytes flow as a CONTROL message in the `[04]` handshake, so this is the
    /// glue, not a blocking call.
    // TODO(steam-runtime): wire `client.user().authentication_session_ticket()`
    // on the client and `client.user().begin_authentication_session(...)` on the
    // host, then resolve the `AuthSessionTicketResponse` / `ValidateAuthTicketResponse`
    // callbacks to confirm the player_id<->SteamID binding before the director
    // accepts the player into the match.
    pub fn bind_player_identity(_client: &Client) {
        // Sketch only; see TODO above. Kept as a named touchpoint so the §7.7
        // checklist maps to a concrete location.
    }
}

impl Transport for SteamTransport {
    fn send(&mut self, outs: Vec<Outbound>) {
        for o in outs {
            let flags = send_flags_for(o.channel);
            match self.peers.get(&o.to) {
                Some(link) => {
                    let framed = frame(o.channel, &o.bytes);
                    // SendMessageToConnection(conn, bytes, flags). Errors here are
                    // connection-level (closed mid-send); the director's ack/digest
                    // loop recovers via reconnect/snapshot.
                    let _ = link.conn.send_message(&framed, flags);
                }
                None => {
                    // Target not connected yet (pre-handshake / mid-migration).
                    // Queue and retry on the next poll rather than dropping —
                    // CONTROL must not be lost.
                    self.pending.push(o);
                }
            }
        }
    }

    fn poll(&mut self) -> Vec<Inbound> {
        // 1. Host: accept any new peers first so their first messages decode.
        self.accept_pending_connections();
        // 2. Retry anything we couldn't send because its peer wasn't up yet.
        self.flush_pending();

        // 3. Drain messages. Host uses the poll group (all peers at once);
        //    client drains its single connection. Decode needs `&self` (the
        //    SteamID->PeerId map), so collect raw messages first, then decode.
        let raw: Vec<NetworkingMessage<ClientManager>> = match &mut self.role {
            Role::Host { poll_group, .. } => poll_group.receive_messages(MAX_RECV_PER_POLL),
            Role::Client { .. } => match self.peers.get_mut(&net::DIRECTOR) {
                Some(link) => link
                    .conn
                    .receive_messages(MAX_RECV_PER_POLL)
                    .unwrap_or_default(),
                None => Vec::new(),
            },
        };

        let mut out = Vec::with_capacity(raw.len());
        for m in &raw {
            if let Some(inb) = self.decode(m) {
                out.push(inb);
            }
        }
        out
    }
}

/// Errors the adapter can surface to the match driver. Connection-level faults
/// are recoverable (the protocol's ack/digest/snapshot loop heals them); these
/// are the setup-time failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamError {
    /// `SteamAPI_Init` failed (no Steam client / bad `steam_appid.txt`).
    Init,
    /// Could not open the host P2P listen socket.
    ListenSocket,
    /// Could not open a connection to the host SteamID.
    Connect,
    /// A role-specific call (e.g. migrate) was made on the wrong role.
    WrongRole,
}

/// Process-wide Steamworks bootstrap (`docs/07 §7.3` steps 1–5). Owns the
/// `steamworks::Client` + `SingleClient` callback pump. Construct ONCE at app
/// start; the [`SteamTransport`] borrows the `Client` to open sockets.
pub struct SteamBootstrap {
    pub client: Client,
    pub single: steamworks::SingleClient,
}

impl SteamBootstrap {
    /// `SteamAPI_Init`. Requires a running Steam client and a `steam_appid.txt`
    /// next to the executable carrying the registered App ID (see `steam_appid.txt`
    /// at the repo root / `godot/`). Returns the client + the single-threaded
    /// callback dispatcher.
    pub fn init() -> Result<SteamBootstrap, SteamError> {
        // `Client::init()` reads `steam_appid.txt`; `Client::init_app(app_id)`
        // forces the App ID for dev. The shipping build uses the registered free
        // App ID via the file.
        let (client, single) = Client::init().map_err(|_| SteamError::Init)?;

        // Initialize the relay network access early so the first connection
        // doesn't pay the SDR cert/route warmup latency.
        client.networking_utils().init_relay_network_access();

        Ok(SteamBootstrap { client, single })
    }

    /// Pump Steam callbacks. Call once per frame/driver iteration so connection
    /// status changes, auth responses, and lobby events are delivered.
    pub fn run_callbacks(&self) {
        self.single.run_callbacks();
    }

    /// Register the connection-status callback that detects host loss and feeds
    /// the migration path (`docs/07 §7.5`, `04 §4.4.6`).
    // TODO(steam-runtime): register
    // `client.register_callback::<NetConnectionStatusChanged>(...)`; on a
    // `ProblemDetectedLocally` / `ClosedByPeer` transition for the director
    // connection, surface a "host left" event so the UI can show graceful UX and
    // the client can call `SteamTransport::migrate_to_host(new_owner)`.
    pub fn on_connection_status_changed(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::transport::Channel;

    // These tests exercise the pure-logic parts that DON'T need a live SDK: the
    // channel<->lane round-trip, the framing, and the flag derivation from net's
    // source of truth. They run wherever the crate compiles (i.e. SDK present).
    #[test]
    fn lane_channel_roundtrip() {
        for c in [Channel::Control, Channel::Telemetry, Channel::Bulk] {
            assert_eq!(Lane::from_channel(c).to_channel(), c);
        }
    }

    #[test]
    fn frame_unframe_roundtrip() {
        for c in [Channel::Control, Channel::Telemetry, Channel::Bulk] {
            let payload = vec![9u8, 8, 7, 6];
            let framed = frame(c, &payload);
            let (chan, bytes) = unframe(&framed).unwrap();
            assert_eq!(chan, c);
            assert_eq!(bytes, payload);
        }
        assert!(unframe(&[]).is_none());
    }

    #[test]
    fn flags_match_net_source_of_truth() {
        // Control + Bulk reliable; Telemetry unreliable + no-nagle.
        assert!(send_flags_for(Channel::Control).contains(SendFlags::RELIABLE));
        assert!(send_flags_for(Channel::Bulk).contains(SendFlags::RELIABLE));
        let tele = send_flags_for(Channel::Telemetry);
        assert!(!tele.contains(SendFlags::RELIABLE));
        assert!(tele.contains(SendFlags::NO_NAGLE));
    }
}
