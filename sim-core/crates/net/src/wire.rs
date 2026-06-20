//! Wire protocol messages + codec (`docs/04 §4.4`, M2 subset). Zero-dep,
//! little-endian, length-prefixed. Owned centrally — it is the contract that
//! decouples director and client.

use sim::Input;

/// The encoded form of a player action (mirrors `sim::Input`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputCode {
    Noop,
    BuyOffer(u8),
    Reroll,
    Clear,
}

impl InputCode {
    pub fn from_input(i: Input) -> InputCode {
        match i {
            Input::Noop => InputCode::Noop,
            Input::BuyOffer { slot } => InputCode::BuyOffer(slot),
            Input::Reroll => InputCode::Reroll,
            Input::Clear => InputCode::Clear,
        }
    }
    pub fn to_input(self) -> Input {
        match self {
            InputCode::Noop => Input::Noop,
            InputCode::BuyOffer(slot) => Input::BuyOffer { slot },
            InputCode::Reroll => Input::Reroll,
            InputCode::Clear => Input::Clear,
        }
    }
}

/// M2 message set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    // ---- director → client ----
    /// Begin the match; build `ArenaState::new(master_seed, player_id)` at `start_tick`.
    MatchStart { start_tick: u32, master_seed: u64 },
    /// Authoritative clock beacon.
    TimeBeacon { server_tick: u32 },
    /// Acknowledge an input and pin its authoritative apply tick.
    InputAck { seq: u32, apply_tick: u32 },
    /// Authoritative arena snapshot (correction / reconnect).
    Snapshot { tick: u32, bytes: Vec<u8> },
    /// A player has been eliminated; `place` is their final placement.
    DeathConfirmed { player: u32, died_tick: u32, place: u32 },
    /// Final standings: `(player, place)` pairs.
    MatchResult { places: Vec<(u32, u32)> },

    // ---- client → director ----
    /// Join handshake; `content_hash` gates version/content match.
    Join { content_hash: u64 },
    /// A player action; the director assigns its `apply_tick`.
    Input { seq: u32, action: InputCode },
    /// Liveness + drift digest for the client's current tick.
    Digest { tick: u32, checksum: u64 },
}

#[derive(Debug, PartialEq, Eq)]
pub enum WireError {
    UnexpectedEof,
    BadTag(u8),
}

// ------------------------------- codec helpers -------------------------------

struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn input(&mut self, a: InputCode) {
        match a {
            InputCode::Noop => self.u8(0),
            InputCode::BuyOffer(s) => {
                self.u8(1);
                self.u8(s);
            }
            InputCode::Reroll => self.u8(2),
            InputCode::Clear => self.u8(3),
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.p.checked_add(n).ok_or(WireError::UnexpectedEof)?;
        let s = self.b.get(self.p..end).ok_or(WireError::UnexpectedEof)?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, WireError> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn input(&mut self) -> Result<InputCode, WireError> {
        Ok(match self.u8()? {
            0 => InputCode::Noop,
            1 => InputCode::BuyOffer(self.u8()?),
            2 => InputCode::Reroll,
            3 => InputCode::Clear,
            t => return Err(WireError::BadTag(t)),
        })
    }
}

/// Encode a message to bytes.
pub fn encode(m: &Msg) -> Vec<u8> {
    let mut w = W(Vec::new());
    match m {
        Msg::MatchStart { start_tick, master_seed } => {
            w.u8(0);
            w.u32(*start_tick);
            w.u64(*master_seed);
        }
        Msg::TimeBeacon { server_tick } => {
            w.u8(1);
            w.u32(*server_tick);
        }
        Msg::InputAck { seq, apply_tick } => {
            w.u8(2);
            w.u32(*seq);
            w.u32(*apply_tick);
        }
        Msg::Snapshot { tick, bytes } => {
            w.u8(3);
            w.u32(*tick);
            w.bytes(bytes);
        }
        Msg::DeathConfirmed { player, died_tick, place } => {
            w.u8(4);
            w.u32(*player);
            w.u32(*died_tick);
            w.u32(*place);
        }
        Msg::MatchResult { places } => {
            w.u8(5);
            w.u32(places.len() as u32);
            for (p, pl) in places {
                w.u32(*p);
                w.u32(*pl);
            }
        }
        Msg::Join { content_hash } => {
            w.u8(6);
            w.u64(*content_hash);
        }
        Msg::Input { seq, action } => {
            w.u8(7);
            w.u32(*seq);
            w.input(*action);
        }
        Msg::Digest { tick, checksum } => {
            w.u8(8);
            w.u32(*tick);
            w.u64(*checksum);
        }
    }
    w.0
}

/// Decode a message from bytes.
pub fn decode(bytes: &[u8]) -> Result<Msg, WireError> {
    let mut r = R { b: bytes, p: 0 };
    let m = match r.u8()? {
        0 => Msg::MatchStart {
            start_tick: r.u32()?,
            master_seed: r.u64()?,
        },
        1 => Msg::TimeBeacon { server_tick: r.u32()? },
        2 => Msg::InputAck {
            seq: r.u32()?,
            apply_tick: r.u32()?,
        },
        3 => Msg::Snapshot {
            tick: r.u32()?,
            bytes: r.bytes()?,
        },
        4 => Msg::DeathConfirmed {
            player: r.u32()?,
            died_tick: r.u32()?,
            place: r.u32()?,
        },
        5 => {
            let n = r.u32()? as usize;
            let mut places = Vec::with_capacity(n);
            for _ in 0..n {
                places.push((r.u32()?, r.u32()?));
            }
            Msg::MatchResult { places }
        }
        6 => Msg::Join { content_hash: r.u64()? },
        7 => Msg::Input {
            seq: r.u32()?,
            action: r.input()?,
        },
        8 => Msg::Digest {
            tick: r.u32()?,
            checksum: r.u64()?,
        },
        t => return Err(WireError::BadTag(t)),
    };
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_variants() {
        let msgs = vec![
            Msg::MatchStart { start_tick: 5, master_seed: 0xDEAD_BEEF_1234 },
            Msg::TimeBeacon { server_tick: 99 },
            Msg::InputAck { seq: 7, apply_tick: 42 },
            Msg::Snapshot { tick: 10, bytes: vec![1, 2, 3, 4] },
            Msg::DeathConfirmed { player: 2, died_tick: 800, place: 3 },
            Msg::MatchResult { places: vec![(1, 1), (2, 2)] },
            Msg::Join { content_hash: 0xABCD },
            Msg::Input { seq: 3, action: InputCode::BuyOffer(2) },
            Msg::Input { seq: 4, action: InputCode::Clear },
            Msg::Digest { tick: 30, checksum: 0x1122_3344_5566_7788 },
        ];
        for m in msgs {
            assert_eq!(decode(&encode(&m)).unwrap(), m, "roundtrip {m:?}");
        }
    }

    #[test]
    fn bad_tag_is_rejected() {
        assert_eq!(decode(&[200]), Err(WireError::BadTag(200)));
    }

    #[test]
    fn truncated_is_rejected() {
        assert_eq!(decode(&[0, 1, 2]), Err(WireError::UnexpectedEof));
    }
}
