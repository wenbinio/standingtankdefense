//! Steam-ready match results (`docs/07 §7.6`). Maps the director's authoritative
//! placement to the per-player rows a Steam build writes via `ISteamUserStats`
//! (leaderboards/achievements). Pure data — the actual Steam calls live behind
//! the `steam` adapter (`steam.rs`), not built in this sandbox.

use crate::replay::{verify, Replay, VerifyOutcome};
use crate::transport::PeerId;

/// One player's end-of-match record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerStats {
    pub player: PeerId,
    /// Final placement (1 = last alive / best).
    pub place: u32,
    /// Finished in the surviving (top) half of the lobby (`docs/02 §2.7`).
    pub win: bool,
    /// Sole survivor — earns a "Last Stand".
    pub last_stand: bool,
}

/// The full set of rows to upload to Steam stats/leaderboards.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MatchStats {
    pub rows: Vec<PlayerStats>,
}

impl MatchStats {
    /// Build from the director's final placements `(player, place)`. Win = top
    /// half; Last Stand = place 1 (`docs/02 §2.7`).
    pub fn from_placements(placements: &[(PeerId, u32)]) -> MatchStats {
        let n = placements.len() as u32;
        let win_cutoff = n.div_ceil(2); // top half (ceil for odd lobbies)
        let rows = placements
            .iter()
            .map(|&(player, place)| PlayerStats {
                player,
                place,
                win: place <= win_cutoff,
                last_stand: place == 1,
            })
            .collect();
        MatchStats { rows }
    }
}

/// A leaderboard-submission row: a player's placement plus the captured replay
/// backing it. The replay is the anti-cheat evidence — re-simming it must
/// reproduce the claimed death (`docs/07 §7.6`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SubmittedResult<'a> {
    pub player: PeerId,
    pub place: u32,
    pub replay: &'a Replay,
}

/// Verify a batch of submitted replays against a local content hash before they
/// reach the leaderboard. Returns one `(player, VerifyOutcome)` per submission,
/// in input order — `VerifyOutcome::Pass` rows are safe to upload; any `Fail`
/// is a rejected (forged/tampered/wrong-content) claim. Pure re-sim, no Steam.
pub fn verify_submissions(
    submissions: &[SubmittedResult<'_>],
    local_content_hash: u64,
) -> Vec<(PeerId, VerifyOutcome)> {
    submissions
        .iter()
        .map(|s| (s.player, verify(s.replay, local_content_hash)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u32) -> PeerId {
        PeerId(n)
    }

    #[test]
    fn eight_player_top_half_wins_and_sole_survivor_last_stands() {
        let placements: Vec<(PeerId, u32)> = (1..=8).map(|p| (pid(p), p)).collect();
        let stats = MatchStats::from_placements(&placements);
        // places 1..=4 win, 5..=8 lose; place 1 is the Last Stand.
        for r in &stats.rows {
            assert_eq!(
                r.win,
                r.place <= 4,
                "win cutoff wrong for place {}",
                r.place
            );
            assert_eq!(r.last_stand, r.place == 1);
        }
    }

    #[test]
    fn odd_lobby_uses_ceil_for_the_winning_half() {
        let placements: Vec<(PeerId, u32)> = (1..=7).map(|p| (pid(p), p)).collect();
        let stats = MatchStats::from_placements(&placements);
        // ceil(7/2) = 4 winners.
        assert_eq!(stats.rows.iter().filter(|r| r.win).count(), 4);
    }

    /// Drive a director to a player's death and pull the captured replay, so the
    /// submission test runs against a real live-capture record (not a synthetic
    /// one).
    fn captured_replay(content_hash: u64) -> Replay {
        use crate::director::Director;
        use crate::START_LEAD;
        let mut d = Director::with_content_hash(&[pid(1)], 0x5151, content_hash);
        for _ in 0..START_LEAD {
            d.tick(vec![]);
        }
        for _ in 0..500_000u32 {
            d.tick(vec![]);
            if d.result().is_some() {
                break;
            }
        }
        d.replay(pid(1)).cloned().expect("captured replay")
    }

    #[test]
    fn verify_submissions_passes_clean_and_rejects_tampered() {
        const CONTENT: u64 = 0xC0FFEE;
        let clean = captured_replay(CONTENT);
        let mut forged = clean.clone();
        forged.claimed.death_tick = forged.claimed.death_tick.saturating_sub(1);

        let subs = vec![
            SubmittedResult {
                player: pid(1),
                place: 1,
                replay: &clean,
            },
            SubmittedResult {
                player: pid(2),
                place: 2,
                replay: &forged,
            },
        ];
        let outcomes = verify_submissions(&subs, CONTENT);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].0, pid(1));
        assert!(outcomes[0].1.is_pass(), "clean replay must pass");
        assert_eq!(outcomes[1].0, pid(2));
        assert!(!outcomes[1].1.is_pass(), "tampered replay must be rejected");

        // Wrong content hash rejects everything.
        let wrong = verify_submissions(&subs, CONTENT ^ 0x1);
        assert!(wrong.iter().all(|(_, o)| !o.is_pass()));
    }
}
