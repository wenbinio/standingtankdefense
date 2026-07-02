//! Replay capture + re-sim verification (`docs/07 §7.6`/§7.7): the leaderboard
//! anti-cheat primitive.
//!
//! A match winner submits a compact `(master_seed, player_id, content_hash,
//! input_log, claimed_result)`. An independent verifier re-builds
//! `ArenaState::new(master_seed, player_id)`, replays the inputs at their
//! authoritative `apply_tick`, and confirms the claimed outcome — the death tick
//! and a checksum digest of the arena at the decisive moment — was legally
//! produced by those inputs. No entities are shipped, only inputs + seeds
//! (`docs/03`), so the record is tiny and the re-sim is the source of truth.
//!
//! ## Why this matches the live shadow bit-for-bit
//! The verifier drives the arena through the EXACT canonical apply path used by
//! [`crate::Director`] and [`crate::Client`]: it loads each tick's action into a
//! [`Schedule`] keyed by `apply_tick` and, when stepping arena tick `T`, applies
//! `schedule.take(T)` then `sim::step(&mut arena, action)`. The inputs recorded
//! in a [`Replay`] are the POST-FILTER actions actually fed to `sim::step` on the
//! director's shadow (any `Challenge` filtering is already baked into the logged
//! action), so the verifier needs no challenge state and still reproduces the
//! shadow's trajectory exactly. Determinism is inherited from the sim core:
//! integer/fixed-point math, seeded PRNG streams, stable iteration — no floats,
//! no wall-clock, no platform RNG (`docs/05 §5.6`).

use crate::wire::{InputCode, WireError};
use crate::Schedule;
use sim::ArenaState;

/// How many ticks past the claimed death the verifier keeps stepping to confirm
/// the death is real and stable (the sim freezes after death, so a genuine death
/// stays dead; a forged-early `death_tick` shows the tank still alive here).
const VERIFY_MARGIN_TICKS: u32 = 4;

/// The verifiable outcome a submitter claims for a run. The competitively
/// load-bearing fact is "who died when" (placement), captured as `death_tick`,
/// plus a `result_digest` pinning the full arena state at the decisive moment so
/// a forged digest (right death tick, wrong state) is still caught.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClaimedResult {
    /// The arena tick at which the player's tank died (`ArenaState::death_tick`).
    pub death_tick: u32,
    /// `sim::checksum` of the arena state captured immediately AFTER the step
    /// that resolved the death (i.e. the first tick at which `dead` is true).
    pub result_digest: u64,
}

/// A compact, engine-independent replay record. Serializable with the same
/// zero-dep little-endian style as [`crate::wire`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Replay {
    pub master_seed: u64,
    pub player_id: u32,
    /// Content version gate — the verifier's content must match (`docs/05`).
    pub content_hash: u64,
    /// Ordered `(apply_tick, action)` input log. The action is the post-filter
    /// `InputCode` actually applied to the shadow at `apply_tick`. Only
    /// non-`Noop` actions need be recorded; the schedule yields `Noop` for any
    /// unrecorded tick, matching the live apply path.
    pub inputs: Vec<(u32, InputCode)>,
    pub claimed: ClaimedResult,
}

impl Replay {
    /// Build the [`Schedule`] the verifier drives — exactly how the director
    /// loads acked inputs (`schedule.set(apply_tick, action)`).
    fn schedule(&self) -> Schedule {
        let mut s = Schedule::new();
        for (apply_tick, action) in &self.inputs {
            s.set(*apply_tick, action.to_input());
        }
        s
    }
}

/// Why a re-sim verification failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerifyFail {
    /// The verifier's content does not match the replay's `content_hash`.
    ContentMismatch { expected: u64, got: u64 },
    /// The tank never died within the claimed death tick + margin window — the
    /// claimed `death_tick` is unreachable from these inputs.
    NeverDied,
    /// The tank died, but at a different tick than claimed (e.g. a tampered
    /// `death_tick`, or an altered input that shifts the death).
    WrongDeathTick { claimed: u32, actual: u32 },
    /// Death tick matched, but the arena-state digest at the decisive moment did
    /// not — a forged `result_digest` or a tampered input that changes state
    /// without moving the death tick.
    DigestMismatch { claimed: u64, actual: u64 },
}

/// The result of re-simming a [`Replay`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerifyOutcome {
    /// The inputs legally reproduce the claimed death tick AND digest.
    Pass,
    /// The claim is not reproducible; carries the specific reason.
    Fail(VerifyFail),
}

impl VerifyOutcome {
    pub fn is_pass(self) -> bool {
        matches!(self, VerifyOutcome::Pass)
    }
}

/// Re-sim a replay and confirm its claimed result. Bit-identical to the live
/// shadow because it reuses the canonical `schedule.take(tick)` → `sim::step`
/// apply path (see module docs).
///
/// `local_content_hash` is the verifier's own content hash; the replay's
/// `content_hash` must equal it (a version/content mismatch is an automatic
/// fail, since determinism only holds within one content set).
pub fn verify(replay: &Replay, local_content_hash: u64) -> VerifyOutcome {
    if replay.content_hash != local_content_hash {
        return VerifyOutcome::Fail(VerifyFail::ContentMismatch {
            expected: local_content_hash,
            got: replay.content_hash,
        });
    }

    let mut arena = ArenaState::new(replay.master_seed, replay.player_id);
    let mut schedule = replay.schedule();

    // Run to (and a little past) the claimed death tick using the canonical
    // apply path. Capture the digest at the FIRST tick the tank is dead.
    let claimed = replay.claimed;
    let limit = claimed.death_tick.saturating_add(VERIFY_MARGIN_TICKS);
    let mut actual_death: Option<(u32, u64)> = None;

    // `sim::step` consumes the action scheduled for the pre-step tick. Iterate
    // arena ticks 0..=limit so we step PAST `death_tick` by the margin. The sim
    // freezes after death, so continuing to step is a harmless no-op that only
    // confirms the death is stable.
    for _ in 0..=limit {
        let pre = arena.tick;
        let action = schedule.take(pre);
        sim::step(&mut arena, action);
        if actual_death.is_none() && arena.dead {
            // First tick the tank is dead: record the authoritative death tick
            // and the digest of the arena right after the resolving step.
            let dt = arena.death_tick.unwrap_or(pre);
            actual_death = Some((dt, sim::checksum(&arena)));
        }
    }

    let (actual_tick, actual_digest) = match actual_death {
        Some(d) => d,
        None => return VerifyOutcome::Fail(VerifyFail::NeverDied),
    };

    if actual_tick != claimed.death_tick {
        return VerifyOutcome::Fail(VerifyFail::WrongDeathTick {
            claimed: claimed.death_tick,
            actual: actual_tick,
        });
    }
    if actual_digest != claimed.result_digest {
        return VerifyOutcome::Fail(VerifyFail::DigestMismatch {
            claimed: claimed.result_digest,
            actual: actual_digest,
        });
    }
    VerifyOutcome::Pass
}

// ------------------------------- capture -------------------------------------

/// Accumulates the actual applied inputs over a driven run, then mints a
/// [`Replay`] from the observed death. Use this to record a run for submission:
/// log each NON-`Noop` action at the arena tick it was applied, then call
/// [`Capture::finish`] with the dead arena.
///
/// This mirrors the live apply path: the caller drives an `ArenaState` with the
/// same `schedule.take(tick)` → `sim::step` loop, recording the action it fed to
/// `sim::step` whenever it is not `Noop`.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    master_seed: u64,
    player_id: u32,
    content_hash: u64,
    inputs: Vec<(u32, InputCode)>,
}

impl Capture {
    pub fn new(master_seed: u64, player_id: u32, content_hash: u64) -> Capture {
        Capture {
            master_seed,
            player_id,
            content_hash,
            inputs: Vec::new(),
        }
    }

    /// Record that `action` was applied at arena tick `apply_tick`. `Noop`
    /// actions are dropped (the schedule yields `Noop` by default on re-sim).
    pub fn record(&mut self, apply_tick: u32, action: InputCode) {
        if action != InputCode::Noop {
            self.inputs.push((apply_tick, action));
        }
    }

    /// Mint a [`Replay`] from a dead arena. Returns `None` if the arena is not
    /// dead (there is no claimable death tick to verify).
    ///
    /// `result_digest` MUST be the checksum captured at the first tick the arena
    /// became dead (the same moment [`verify`] samples). The caller passes it in
    /// because only it observed that exact instant; `finish` validates the
    /// arena is genuinely dead.
    pub fn finish(self, dead_arena: &ArenaState, result_digest: u64) -> Option<Replay> {
        if !dead_arena.dead {
            return None;
        }
        let death_tick = dead_arena.death_tick?;
        Some(Replay {
            master_seed: self.master_seed,
            player_id: self.player_id,
            content_hash: self.content_hash,
            inputs: self.inputs,
            claimed: ClaimedResult {
                death_tick,
                result_digest,
            },
        })
    }
}

// ------------------------------- codec ---------------------------------------

/// Serialize a [`Replay`] to bytes (zero-dep, little-endian, matching `wire`).
pub fn encode(r: &Replay) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&r.master_seed.to_le_bytes());
    out.extend_from_slice(&r.player_id.to_le_bytes());
    out.extend_from_slice(&r.content_hash.to_le_bytes());
    out.extend_from_slice(&(r.inputs.len() as u32).to_le_bytes());
    for (apply_tick, action) in &r.inputs {
        out.extend_from_slice(&apply_tick.to_le_bytes());
        encode_action(&mut out, *action);
    }
    out.extend_from_slice(&r.claimed.death_tick.to_le_bytes());
    out.extend_from_slice(&r.claimed.result_digest.to_le_bytes());
    out
}

/// Deserialize a [`Replay`] from bytes.
pub fn decode(bytes: &[u8]) -> Result<Replay, WireError> {
    let mut r = Reader { b: bytes, p: 0 };
    let master_seed = r.u64()?;
    let player_id = r.u32()?;
    let content_hash = r.u64()?;
    let n = r.u32()? as usize;
    let mut inputs = Vec::with_capacity(n);
    for _ in 0..n {
        let apply_tick = r.u32()?;
        let action = decode_action(&mut r)?;
        inputs.push((apply_tick, action));
    }
    let death_tick = r.u32()?;
    let result_digest = r.u64()?;
    Ok(Replay {
        master_seed,
        player_id,
        content_hash,
        inputs,
        claimed: ClaimedResult {
            death_tick,
            result_digest,
        },
    })
}

fn encode_action(out: &mut Vec<u8>, a: InputCode) {
    match a {
        InputCode::Noop => out.push(0),
        InputCode::BuyOffer(s) => {
            out.push(1);
            out.push(s);
        }
        InputCode::Reroll => out.push(2),
        InputCode::Clear => out.push(3),
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
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
}

fn decode_action(r: &mut Reader) -> Result<InputCode, WireError> {
    Ok(match r.u8()? {
        0 => InputCode::Noop,
        1 => InputCode::BuyOffer(r.u8()?),
        2 => InputCode::Reroll,
        3 => InputCode::Clear,
        t => return Err(WireError::BadTag(t)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::Input;

    /// Drive an arena to death using the canonical apply path, scheduling the
    /// given `(apply_tick, action)` inputs, and capture a [`Replay`]. Returns the
    /// minted replay. This mirrors exactly how the director steps a shadow, so
    /// the captured replay re-sims bit-identically.
    fn capture_run(
        seed: u64,
        player_id: u32,
        content_hash: u64,
        inputs: &[(u32, InputCode)],
        max_ticks: u32,
    ) -> Replay {
        let mut arena = ArenaState::new(seed, player_id);
        let mut schedule = Schedule::new();
        for (t, a) in inputs {
            schedule.set(*t, a.to_input());
        }
        let mut cap = Capture::new(seed, player_id, content_hash);
        let mut digest_at_death = None;
        for _ in 0..max_ticks {
            let pre = arena.tick;
            let action = schedule.take(pre);
            cap.record(pre, InputCode::from_input(action));
            sim::step(&mut arena, action);
            if arena.dead {
                digest_at_death = Some(sim::checksum(&arena));
                break;
            }
        }
        let digest = digest_at_death.expect("arena must die within max_ticks");
        cap.finish(&arena, digest)
            .expect("dead arena yields a replay")
    }

    const SEED: u64 = 0x5151;
    const CONTENT: u64 = 0xC0FFEE;

    #[test]
    fn legit_replay_passes() {
        // A real driven run (no inputs — the lone tank dies to contact damage).
        let replay = capture_run(SEED, 1, CONTENT, &[], 500_000);
        assert_eq!(verify(&replay, CONTENT), VerifyOutcome::Pass);
        // The claimed death tick must be the real one.
        assert!(replay.claimed.death_tick > 0);
    }

    #[test]
    fn legit_replay_with_inputs_passes() {
        // A run with real actions applied at their apply ticks still re-sims to
        // the same death + digest.
        let inputs = vec![
            (8u32, InputCode::Reroll),
            (40, InputCode::BuyOffer(0)),
            (90, InputCode::Clear),
        ];
        let replay = capture_run(SEED, 2, CONTENT, &inputs, 500_000);
        assert_eq!(verify(&replay, CONTENT), VerifyOutcome::Pass);
        // The recorded inputs are exactly the non-Noop applied actions.
        assert_eq!(replay.inputs, inputs);
    }

    #[test]
    fn tampered_death_tick_fails() {
        let mut replay = capture_run(SEED, 1, CONTENT, &[], 500_000);
        let real = replay.claimed.death_tick;
        // Claim the tank died one tick earlier than it actually did.
        replay.claimed.death_tick = real - 1;
        match verify(&replay, CONTENT) {
            VerifyOutcome::Fail(VerifyFail::WrongDeathTick { claimed, actual }) => {
                assert_eq!(claimed, real - 1);
                assert_eq!(actual, real);
            }
            other => panic!("expected WrongDeathTick, got {other:?}"),
        }
    }

    #[test]
    fn forged_digest_fails() {
        let mut replay = capture_run(SEED, 1, CONTENT, &[], 500_000);
        let real = replay.claimed.result_digest;
        replay.claimed.result_digest ^= 0xDEAD_BEEF;
        match verify(&replay, CONTENT) {
            VerifyOutcome::Fail(VerifyFail::DigestMismatch { claimed, actual }) => {
                assert_ne!(claimed, actual);
                assert_eq!(actual, real, "verifier recomputes the true digest");
            }
            other => panic!("expected DigestMismatch, got {other:?}"),
        }
    }

    #[test]
    fn tampered_input_fails() {
        // Capture a clean run with a known input, then alter the input log.
        // Changing an always-effective input (Reroll advances the shop/reroll RNG
        // streams, which feed the checksum) makes the re-sim diverge from the
        // claimed death/digest, so verification fails. The verifier re-sims the
        // ALTERED inputs and finds they don't reproduce the claim.
        let inputs = vec![(8u32, InputCode::Reroll), (40, InputCode::Reroll)];
        let mut replay = capture_run(SEED, 3, CONTENT, &inputs, 500_000);
        // Tamper: the player never actually rerolled at tick 40 (drop it). The
        // claim was minted from a run that DID, so the re-sim now diverges.
        replay.inputs.retain(|(t, _)| *t != 40);
        let outcome = verify(&replay, CONTENT);
        assert!(
            matches!(
                outcome,
                VerifyOutcome::Fail(VerifyFail::WrongDeathTick { .. })
                    | VerifyOutcome::Fail(VerifyFail::DigestMismatch { .. })
            ),
            "tampered input must fail verification, got {outcome:?}"
        );
    }

    #[test]
    fn content_mismatch_fails() {
        let replay = capture_run(SEED, 1, CONTENT, &[], 500_000);
        match verify(&replay, CONTENT ^ 0x1) {
            VerifyOutcome::Fail(VerifyFail::ContentMismatch { expected, got }) => {
                assert_eq!(expected, CONTENT ^ 0x1);
                assert_eq!(got, CONTENT);
            }
            other => panic!("expected ContentMismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_is_deterministic() {
        // Verifying the same replay twice yields identical outcomes.
        let replay = capture_run(SEED, 1, CONTENT, &[], 500_000);
        let a = verify(&replay, CONTENT);
        let b = verify(&replay, CONTENT);
        assert_eq!(a, b);
        assert!(a.is_pass());
    }

    #[test]
    fn codec_roundtrips() {
        let inputs = vec![
            (8u32, InputCode::Reroll),
            (40, InputCode::BuyOffer(2)),
            (90, InputCode::Clear),
        ];
        let replay = capture_run(SEED, 2, CONTENT, &inputs, 500_000);
        let bytes = encode(&replay);
        let back = decode(&bytes).expect("decode");
        assert_eq!(back, replay);
        // A decoded replay still verifies.
        assert_eq!(verify(&back, CONTENT), VerifyOutcome::Pass);
    }

    #[test]
    fn decode_rejects_truncated_and_bad_tag() {
        assert_eq!(decode(&[1, 2, 3]), Err(WireError::UnexpectedEof));
        // Build a valid header with one input whose action tag is invalid.
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(&0u64.to_le_bytes()); // master_seed
        b.extend_from_slice(&0u32.to_le_bytes()); // player_id
        b.extend_from_slice(&0u64.to_le_bytes()); // content_hash
        b.extend_from_slice(&1u32.to_le_bytes()); // inputs len = 1
        b.extend_from_slice(&0u32.to_le_bytes()); // apply_tick
        b.push(200); // invalid action tag
        assert_eq!(decode(&b), Err(WireError::BadTag(200)));
    }

    #[test]
    fn noop_inputs_are_not_recorded() {
        let mut cap = Capture::new(SEED, 1, CONTENT);
        cap.record(0, InputCode::from_input(Input::Noop));
        cap.record(5, InputCode::Reroll);
        assert_eq!(cap.inputs, vec![(5, InputCode::Reroll)]);
    }
}
