//! Host-side **lobby / session lifecycle** state machine (`docs/07 §7.3`).
//!
//! This is the transport-agnostic *model* behind the Steam matchmaking flow:
//! the host opens a lobby, players join, the lobby data carries the host id,
//! ruleset, `content_hash`, and per-member ready flags, and on start the host
//! produces a [`MatchPlan`] that the director turns into the `[04]` protocol's
//! [`crate::wire::Msg::MatchStart`]. The actual `ISteamMatchmaking`
//! create/join/set-lobby-data calls live in the Steam adapter / UI layer
//! (`docs/07 §7.2`); they *feed* this machine, they are not implemented here.
//!
//! ## Determinism
//! Pure integer state. No wall-clock, no RNG. The `master_seed` threaded into
//! [`Lobby::start`] comes IN from the host (the only authority that mints
//! seeds, `docs/05 §5.6` / `docs/03`); this module never generates it. Members
//! are stored and iterated in stable [`PeerId`] order so the [`MatchPlan`]
//! player list and the encoded payload are reproducible.
//!
//! ## Host-loss (`docs/07 §7.5`)
//! [`Lobby::promote_host`] reassigns lobby ownership when Steam transfers the
//! lobby owner (host disconnect → next member becomes owner). It is pure state:
//! the new host promotes its standby director and peers reconnect — none of
//! which is this module's concern; it only tracks *who* the owner is.

use crate::transport::PeerId;
use crate::wire::WireError;

/// Maximum party size, including the host. Matches the demo's N=8
/// (`docs/07 §7.3`, eight-player arena).
pub const MAX_PARTY: usize = 8;

/// Settings the lobby data carries (`docs/07 §7.3`: "map/ruleset, game speed,
/// challenge"). Small, serializable, deterministic. Shipped as Steam lobby data
/// so every member sees the same configuration before [`Lobby::start`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ruleset {
    /// Map / arena identifier.
    pub map_id: u16,
    /// Game-speed code (e.g. 0 = normal, 1 = fast). Purely a meta setting; it
    /// never feeds `state_checksum`, so it cannot perturb determinism.
    pub game_speed: u8,
    /// Optional challenge / mutator code (0 = none). Mirrors `sim::bot::Challenge`
    /// selection at the meta layer; the director applies the real challenge.
    pub challenge: u16,
}

impl Ruleset {
    /// A plain default match: map 0, normal speed, no challenge.
    pub fn standard() -> Ruleset {
        Ruleset { map_id: 0, game_speed: 0, challenge: 0 }
    }
}

/// One lobby member, in stable [`PeerId`] order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Member {
    /// The member's lobby slot / peer id.
    pub peer: PeerId,
    /// Whether this member has flagged ready.
    pub ready: bool,
    /// The `content_hash` the member presented on join; must equal the lobby's.
    pub content_hash: u64,
}

/// Lobby lifecycle phase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Open and accepting joins; not everyone is ready yet.
    Filling,
    /// Every member is ready — [`Lobby::start`] is now legal.
    Ready,
    /// The match has begun; the lobby is closed to new joins and re-starts.
    Started,
}

/// Why a [`Lobby::join`] was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JoinReject {
    /// The member's `content_hash` does not match the lobby's (`docs/07 §7.3`:
    /// incompatible build/content gated out before the match).
    ContentMismatch { lobby: u64, peer: u64 },
    /// The lobby already holds [`MAX_PARTY`] members.
    Full,
    /// The match has already started; no late joins.
    Closed,
    /// This peer is already a member (idempotent: existing state untouched).
    Duplicate,
}

/// Why a [`Lobby::start`] was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartReject {
    /// Not every member has flagged ready.
    NotAllReady,
    /// Fewer than two members (need at least one non-host opponent).
    NotEnoughPlayers,
    /// The match has already started.
    AlreadyStarted,
}

/// The output of a successful [`Lobby::start`]: everything the host/director
/// needs to construct the match and broadcast
/// [`crate::wire::Msg::MatchStart`]. The `players` are the **non-host** members
/// in slot order (the arenas the director shadows); the host runs the director,
/// not a player arena.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchPlan {
    /// Authoritative match seed, threaded in from the host.
    pub master_seed: u64,
    /// Non-host participants, in stable slot order.
    pub players: Vec<PeerId>,
    /// The agreed content hash (every player matched it on join).
    pub content_hash: u64,
    /// The ruleset the match runs under.
    pub ruleset: Ruleset,
}

/// Host-side lobby state machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lobby {
    host: PeerId,
    content_hash: u64,
    ruleset: Ruleset,
    phase: Phase,
    /// Members in stable [`PeerId`] order (sorted on every mutation).
    members: Vec<Member>,
}

impl Lobby {
    /// Open a lobby. The `host` is the owner and is seated as the first member
    /// (already "ready" — the host gates start by readying everyone else). The
    /// host's own slot carries the lobby `content_hash` by construction.
    pub fn new(host: PeerId, content_hash: u64, ruleset: Ruleset) -> Lobby {
        Lobby {
            host,
            content_hash,
            ruleset,
            phase: Phase::Filling,
            members: vec![Member { peer: host, ready: true, content_hash }],
        }
    }

    // ----------------------------- accessors -----------------------------

    /// The current lobby owner (host).
    pub fn host(&self) -> PeerId {
        self.host
    }

    /// The agreed content hash that joins are gated against.
    pub fn content_hash(&self) -> u64 {
        self.content_hash
    }

    /// The ruleset carried in the lobby data.
    pub fn ruleset(&self) -> Ruleset {
        self.ruleset
    }

    /// The current lifecycle phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// All members, host first, in stable slot order.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Whether `peer` is currently a member.
    pub fn contains(&self, peer: PeerId) -> bool {
        self.members.iter().any(|m| m.peer == peer)
    }

    fn member_mut(&mut self, peer: PeerId) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.peer == peer)
    }

    // ----------------------------- membership ----------------------------

    /// A player joins. Rejected on content mismatch, a full or closed lobby, or
    /// a duplicate join. On success the member is seated `ready = false` and the
    /// phase is recomputed.
    pub fn join(&mut self, peer: PeerId, member_content_hash: u64) -> Result<(), JoinReject> {
        if self.phase == Phase::Started {
            return Err(JoinReject::Closed);
        }
        if self.contains(peer) {
            return Err(JoinReject::Duplicate);
        }
        if member_content_hash != self.content_hash {
            return Err(JoinReject::ContentMismatch {
                lobby: self.content_hash,
                peer: member_content_hash,
            });
        }
        if self.members.len() >= MAX_PARTY {
            return Err(JoinReject::Full);
        }
        self.members.push(Member { peer, ready: false, content_hash: member_content_hash });
        self.members.sort_by_key(|m| m.peer);
        self.recompute_phase();
        Ok(())
    }

    /// A member leaves. No-op if absent or if the host leaves via this path
    /// (host loss is [`Lobby::promote_host`], not a plain leave). Returns whether
    /// a member was removed.
    pub fn leave(&mut self, peer: PeerId) -> bool {
        if peer == self.host {
            return false;
        }
        let before = self.members.len();
        self.members.retain(|m| m.peer != peer);
        let removed = self.members.len() != before;
        if removed {
            self.recompute_phase();
        }
        removed
    }

    // ----------------------------- ready gating --------------------------

    /// Set a member's ready flag. No-op (returns `false`) if the peer is absent
    /// or the match already started. Recomputes the phase.
    pub fn set_ready(&mut self, peer: PeerId, ready: bool) -> bool {
        if self.phase == Phase::Started {
            return false;
        }
        let found = match self.member_mut(peer) {
            Some(m) => {
                m.ready = ready;
                true
            }
            None => false,
        };
        if found {
            self.recompute_phase();
        }
        found
    }

    /// Whether every current member is ready.
    pub fn all_ready(&self) -> bool {
        self.members.iter().all(|m| m.ready)
    }

    /// Non-host members in stable slot order — the player arenas the director
    /// will shadow.
    pub fn players(&self) -> Vec<PeerId> {
        self.members.iter().filter(|m| m.peer != self.host).map(|m| m.peer).collect()
    }

    fn recompute_phase(&mut self) {
        if self.phase == Phase::Started {
            return;
        }
        self.phase = if self.all_ready() && !self.players().is_empty() {
            Phase::Ready
        } else {
            Phase::Filling
        };
    }

    // ----------------------------- lifecycle -----------------------------

    /// Begin the match. Legal only when every member is ready and at least one
    /// non-host player is present. Threads `master_seed` (minted by the host)
    /// into a [`MatchPlan`] and transitions to [`Phase::Started`]. The plan's
    /// `players` are the non-host members in slot order; the director uses it to
    /// build shadows and broadcast [`crate::wire::Msg::MatchStart`].
    pub fn start(&mut self, master_seed: u64) -> Result<MatchPlan, StartReject> {
        if self.phase == Phase::Started {
            return Err(StartReject::AlreadyStarted);
        }
        let players = self.players();
        if players.is_empty() {
            return Err(StartReject::NotEnoughPlayers);
        }
        if !self.all_ready() {
            return Err(StartReject::NotAllReady);
        }
        self.phase = Phase::Started;
        Ok(MatchPlan {
            master_seed,
            players,
            content_hash: self.content_hash,
            ruleset: self.ruleset,
        })
    }

    /// Reassign lobby ownership (`docs/07 §7.5`). Fed by the Steam lobby
    /// owner-change callback on host disconnect: the next member becomes owner,
    /// promotes its standby director, and peers reconnect. Pure state here — we
    /// only move the `host` marker. The new host is ensured to be ready (an
    /// owner cannot block its own lobby). If `new_host` is not yet a member it is
    /// seated (subject to capacity); otherwise it is simply marked host. Returns
    /// `false` only if the new host had to be seated but the lobby was full.
    pub fn promote_host(&mut self, new_host: PeerId) -> bool {
        if !self.contains(new_host) {
            if self.members.len() >= MAX_PARTY {
                return false;
            }
            self.members.push(Member {
                peer: new_host,
                ready: true,
                content_hash: self.content_hash,
            });
            self.members.sort_by_key(|m| m.peer);
        }
        self.host = new_host;
        if let Some(m) = self.member_mut(new_host) {
            m.ready = true;
        }
        self.recompute_phase();
        true
    }

    // ----------------------------- codec ---------------------------------

    /// Deterministic little-endian encode of the lobby data payload (the
    /// "lobby metadata" of `docs/07 §7.3`), in the `wire` style so the Steam
    /// adapter can ship it as lobby data / a control message.
    ///
    /// Layout: `host:u32 | content_hash:u64 | map_id:u16 | game_speed:u8 |
    /// challenge:u16 | phase:u8 | n:u32 | n×(peer:u32, flags:u8, hash:u64)`.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = LobbyW(Vec::new());
        w.u32(self.host.0);
        w.u64(self.content_hash);
        w.u16(self.ruleset.map_id);
        w.u8(self.ruleset.game_speed);
        w.u16(self.ruleset.challenge);
        w.u8(phase_tag(self.phase));
        w.u32(self.members.len() as u32);
        for m in &self.members {
            w.u32(m.peer.0);
            w.u8(if m.ready { 1 } else { 0 });
            w.u64(m.content_hash);
        }
        w.0
    }

    /// Decode a lobby data payload produced by [`Lobby::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Lobby, WireError> {
        let mut r = LobbyR { b: bytes, p: 0 };
        let host = PeerId(r.u32()?);
        let content_hash = r.u64()?;
        let ruleset = Ruleset {
            map_id: r.u16()?,
            game_speed: r.u8()?,
            challenge: r.u16()?,
        };
        let phase = phase_from_tag(r.u8()?)?;
        let n = r.u32()? as usize;
        let mut members = Vec::with_capacity(n);
        for _ in 0..n {
            let peer = PeerId(r.u32()?);
            let ready = r.u8()? != 0;
            let hash = r.u64()?;
            members.push(Member { peer, ready, content_hash: hash });
        }
        Ok(Lobby { host, content_hash, ruleset, phase, members })
    }
}

fn phase_tag(p: Phase) -> u8 {
    match p {
        Phase::Filling => 0,
        Phase::Ready => 1,
        Phase::Started => 2,
    }
}

fn phase_from_tag(t: u8) -> Result<Phase, WireError> {
    Ok(match t {
        0 => Phase::Filling,
        1 => Phase::Ready,
        2 => Phase::Started,
        t => return Err(WireError::BadTag(t)),
    })
}

// --- local codec helpers (wire-style; u16 added for the compact ruleset) ---

struct LobbyW(Vec<u8>);
impl LobbyW {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
}

struct LobbyR<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> LobbyR<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.p.checked_add(n).ok_or(WireError::UnexpectedEof)?;
        let s = self.b.get(self.p..end).ok_or(WireError::UnexpectedEof)?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::DIRECTOR;

    const HASH: u64 = 0x00C0_FFEE_1234;
    const HOST: PeerId = DIRECTOR; // host = director slot (PeerId(0))

    fn p(n: u32) -> PeerId {
        PeerId(n)
    }

    fn host_lobby() -> Lobby {
        Lobby::new(HOST, HASH, Ruleset::standard())
    }

    #[test]
    fn opens_with_host_seated_and_filling() {
        let l = host_lobby();
        assert_eq!(l.host(), HOST);
        assert_eq!(l.phase(), Phase::Filling);
        assert_eq!(l.members().len(), 1);
        assert!(l.members()[0].ready, "host seats itself ready");
        assert!(l.players().is_empty(), "host is not a player arena");
    }

    #[test]
    fn join_and_leave() {
        let mut l = host_lobby();
        assert!(l.join(p(1), HASH).is_ok());
        assert!(l.join(p(2), HASH).is_ok());
        assert_eq!(l.players(), vec![p(1), p(2)]);
        assert!(l.leave(p(1)));
        assert_eq!(l.players(), vec![p(2)]);
        assert!(!l.leave(p(9)), "leaving as a non-member is a no-op");
        assert!(!l.leave(HOST), "host does not leave via leave()");
        assert!(l.contains(HOST));
    }

    #[test]
    fn members_stay_in_slot_order() {
        let mut l = host_lobby();
        // join out of order; members() must still be sorted by PeerId.
        l.join(p(5), HASH).unwrap();
        l.join(p(2), HASH).unwrap();
        l.join(p(8), HASH).unwrap();
        l.join(p(3), HASH).unwrap();
        let order: Vec<u32> = l.members().iter().map(|m| m.peer.0).collect();
        assert_eq!(order, vec![0, 2, 3, 5, 8]);
        assert_eq!(l.players(), vec![p(2), p(3), p(5), p(8)]);
    }

    #[test]
    fn content_mismatch_is_rejected() {
        let mut l = host_lobby();
        assert_eq!(
            l.join(p(1), 0xBAD),
            Err(JoinReject::ContentMismatch { lobby: HASH, peer: 0xBAD })
        );
        assert!(!l.contains(p(1)), "rejected join seats nobody");
    }

    #[test]
    fn duplicate_join_is_rejected_and_idempotent() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        l.set_ready(p(1), true);
        assert_eq!(l.join(p(1), HASH), Err(JoinReject::Duplicate));
        // existing state untouched
        assert!(l.members().iter().find(|m| m.peer == p(1)).unwrap().ready);
        assert_eq!(l.players().len(), 1);
    }

    #[test]
    fn max_size_is_enforced() {
        let mut l = host_lobby();
        // host + 7 = 8 = MAX_PARTY
        for n in 1..=(MAX_PARTY as u32 - 1) {
            assert!(l.join(p(n), HASH).is_ok(), "join {n} within capacity");
        }
        assert_eq!(l.members().len(), MAX_PARTY);
        assert_eq!(l.join(p(99), HASH), Err(JoinReject::Full));
    }

    #[test]
    fn ready_gating_drives_phase_and_start() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        l.join(p(2), HASH).unwrap();
        assert_eq!(l.phase(), Phase::Filling);
        assert_eq!(l.start(0xBAD), Err(StartReject::NotAllReady));

        l.set_ready(p(1), true);
        assert_eq!(l.phase(), Phase::Filling, "not all ready yet");
        l.set_ready(p(2), true);
        assert_eq!(l.phase(), Phase::Ready, "all ready -> Ready");
        assert!(l.all_ready());

        // un-ready falls back to Filling and re-blocks start.
        l.set_ready(p(2), false);
        assert_eq!(l.phase(), Phase::Filling);
        assert!(l.start(1).is_err());
        l.set_ready(p(2), true);
        assert_eq!(l.phase(), Phase::Ready);
    }

    #[test]
    fn start_needs_a_non_host_player() {
        let mut l = host_lobby();
        // only the host present, who is ready -> still not enough players.
        assert!(l.all_ready());
        assert_eq!(l.phase(), Phase::Filling, "lone host never reaches Ready");
        assert_eq!(l.start(7), Err(StartReject::NotEnoughPlayers));
    }

    #[test]
    fn start_produces_correct_plan_and_threads_seed() {
        let mut l = host_lobby();
        l.join(p(3), HASH).unwrap();
        l.join(p(1), HASH).unwrap();
        l.set_ready(p(1), true);
        l.set_ready(p(3), true);
        assert_eq!(l.phase(), Phase::Ready);

        const SEED: u64 = 0xDEAD_BEEF_CAFE_F00D;
        let plan = l.start(SEED).expect("ready lobby starts");
        assert_eq!(plan.master_seed, SEED, "seed threaded through unchanged");
        assert_eq!(plan.players, vec![p(1), p(3)], "non-host members in slot order");
        assert_eq!(plan.content_hash, HASH);
        assert_eq!(plan.ruleset, Ruleset::standard());
        assert_eq!(l.phase(), Phase::Started);
    }

    #[test]
    fn start_twice_fails_and_lobby_closes() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        l.set_ready(p(1), true);
        assert!(l.start(1).is_ok());
        assert_eq!(l.start(2), Err(StartReject::AlreadyStarted));
        // closed to new joins / ready changes.
        assert_eq!(l.join(p(2), HASH), Err(JoinReject::Closed));
        assert!(!l.set_ready(p(1), false));
        assert_eq!(l.phase(), Phase::Started);
    }

    #[test]
    fn promote_host_reassigns_ownership() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        l.join(p(2), HASH).unwrap();
        assert_eq!(l.host(), HOST);

        // Steam transfers ownership to the next member on host loss.
        assert!(l.promote_host(p(1)));
        assert_eq!(l.host(), p(1));
        // The new host is a member and forced ready; old host still seated.
        assert!(l.members().iter().find(|m| m.peer == p(1)).unwrap().ready);
        assert!(l.contains(HOST));
        // players() now excludes the NEW host, includes the old one.
        assert_eq!(l.players(), vec![HOST, p(2)]);
    }

    #[test]
    fn promote_host_seats_an_external_owner() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        assert!(l.promote_host(p(4)));
        assert_eq!(l.host(), p(4));
        assert!(l.contains(p(4)));
        assert!(l.members().iter().find(|m| m.peer == p(4)).unwrap().ready);
    }

    #[test]
    fn lobby_data_round_trips() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        l.join(p(5), HASH).unwrap();
        l.set_ready(p(1), true);

        let ruleset = Ruleset { map_id: 3, game_speed: 1, challenge: 42 };
        let mut l2 = Lobby::new(HOST, HASH, ruleset);
        l2.join(p(1), HASH).unwrap();
        l2.set_ready(p(1), true);

        for orig in [l, l2] {
            let bytes = orig.encode();
            let back = Lobby::decode(&bytes).expect("decode");
            assert_eq!(back, orig, "decode reproduces the lobby");
            // re-encode is byte-identical (deterministic codec)
            assert_eq!(back.encode(), bytes);
        }
    }

    #[test]
    fn decode_rejects_bad_phase_tag_and_truncation() {
        let mut l = host_lobby();
        l.join(p(1), HASH).unwrap();
        let mut bytes = l.encode();
        // phase byte sits after host(4)+hash(8)+map(2)+speed(1)+chal(2) = 17
        bytes[17] = 9;
        assert_eq!(Lobby::decode(&bytes), Err(WireError::BadTag(9)));
        assert_eq!(Lobby::decode(&[0, 1, 2]), Err(WireError::UnexpectedEof));
    }
}
