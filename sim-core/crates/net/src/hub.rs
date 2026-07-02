//! Deterministic in-process transport for tests. Routes [`Outbound`] →
//! [`Inbound`] with per-source one-way delay and stall injection, so we can
//! prove the independence thesis (`docs/06` M2 exit test) without Steam.
//!
//! Latency is measured in driver iterations: a message sent on step `S` over a
//! link with delay `d` is delivered when the hub reaches step `S + d + 1`. A
//! "stall" holds a source's messages until a given step.
//!
//! ## Chaos injection (M5)
//! On top of the baseline delay/stall the hub can model an *adverse* link per
//! source peer, all driven by ONE explicitly-seeded PRNG so every run is
//! byte-reproducible (no wall-clock, no `std` randomness):
//! - [`Hub::set_loss`] drops a `permille` fraction of a peer's outbound messages
//!   (modelling packet loss the protocol must recover from via digest→snapshot).
//! - [`Hub::set_jitter`] varies each message's one-way delay by `0..=max` steps.
//! - [`Hub::set_reorder`] adds a bounded random extra delay within a small
//!   window, so messages from a peer can arrive out of their send order (the
//!   delivery sort is stable on `(deliver_at, seq)`, so equal-step messages keep
//!   send order; the random per-message offset is what crosses them).
//!
//! The chaos RNG is consumed once per message in the order [`Hub::send`] is
//! called (the driver sends participants in a fixed order each step), so the
//! whole stream is a pure function of the seed and the traffic. A hub with no
//! chaos knobs set never touches the RNG, so existing delay/stall tests are
//! unaffected.

use crate::transport::{Inbound, Outbound, PeerId, Transport};
use determinism::Rng;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

struct InFlight {
    deliver_at: u32,
    seq: u64,
    to: PeerId,
    msg: Inbound,
}

#[derive(Default)]
pub struct Hub {
    step: u32,
    seq: u64,
    inflight: Vec<InFlight>,
    inboxes: BTreeMap<u32, Vec<Inbound>>,
    delay: BTreeMap<u32, u32>,
    stalled_until: BTreeMap<u32, u32>,
    /// Per-peer packet-loss rate in permille (0..=1000) for messages SENT BY peer.
    loss: BTreeMap<u32, u32>,
    /// Per-peer extra max one-way delay (jitter) for messages SENT BY peer; each
    /// message gets a random `0..=max` added to its delivery step.
    jitter: BTreeMap<u32, u32>,
    /// Per-peer reorder window for messages SENT BY peer; each message gets a
    /// random `0..=window` added to its delivery step, independently of jitter,
    /// so same-step messages can swap order.
    reorder: BTreeMap<u32, u32>,
    /// The single seeded chaos PRNG. Only consumed when a chaos knob is active
    /// for the sending peer, so a chaos-free hub stays byte-identical to before.
    chaos: Rng,
    /// Whether ANY chaos knob has been configured. Guards the chaos path so the
    /// RNG is never touched on a clean hub (keeps existing tests bit-stable).
    chaos_active: bool,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Build a hub with its chaos PRNG seeded explicitly. Use this (not [`new`])
    /// whenever loss/jitter/reorder is injected, so the whole run reproduces from
    /// `seed`. A hub built with [`new`] uses seed 0; harmless until a knob is set.
    pub fn with_chaos_seed(seed: u64) -> Hub {
        Hub {
            chaos: Rng::from_seed(seed),
            ..Hub::default()
        }
    }

    /// (Re)seed the chaos PRNG. Lets a test reset the chaos stream between runs.
    pub fn seed_chaos(&mut self, seed: u64) {
        self.chaos = Rng::from_seed(seed);
    }

    /// Current driver step.
    pub fn step(&self) -> u32 {
        self.step
    }

    /// Set the one-way delivery delay (in steps) for messages SENT BY `peer`.
    pub fn set_delay(&mut self, peer: PeerId, delay: u32) {
        self.delay.insert(peer.0, delay);
    }

    /// Hold all messages sent by `peer` until step `until` (a lag/stall window).
    pub fn stall(&mut self, peer: PeerId, until: u32) {
        self.stalled_until.insert(peer.0, until);
    }

    /// Drop a `permille`/1000 fraction of messages SENT BY `peer` (seeded). Models
    /// packet loss; the protocol must recover via digest→snapshot. `0` = lossless,
    /// `1000` = a fully black-holed link.
    pub fn set_loss(&mut self, peer: PeerId, permille: u32) {
        self.loss.insert(peer.0, permille.min(1000));
        self.chaos_active = true;
    }

    /// Vary the one-way delay of messages SENT BY `peer` by a seeded `0..=max`
    /// extra steps. `0` disables jitter for the peer.
    pub fn set_jitter(&mut self, peer: PeerId, max: u32) {
        self.jitter.insert(peer.0, max);
        self.chaos_active = true;
    }

    /// Reorder messages SENT BY `peer` within a small `window`: each message gets
    /// a seeded `0..=window` extra delay, so same-step messages can swap order.
    /// `window == 0` disables reorder for the peer.
    pub fn set_reorder(&mut self, peer: PeerId, window: u32) {
        self.reorder.insert(peer.0, window);
        self.chaos_active = true;
    }

    /// Drop ALL chaos knobs (loss/jitter/reorder) on every peer, healing every
    /// link back to a clean (delay/stall-only) state. The chaos RNG is NOT
    /// reseeded, so a post-heal run continues the same deterministic stream.
    /// Lets a test model a transient outage that then recovers — the recovery
    /// (digest→snapshot reconvergence) is exactly what M5 asserts survives.
    pub fn clear_chaos(&mut self) {
        self.loss.clear();
        self.jitter.clear();
        self.reorder.clear();
        self.chaos_active = false;
    }

    fn delay_of(&self, peer: PeerId) -> u32 {
        *self.delay.get(&peer.0).unwrap_or(&0)
    }

    /// Enqueue everything `from` is sending this step.
    pub fn send(&mut self, from: PeerId, outs: Vec<Outbound>) {
        // A message released on step R with one-way delay d is delivered when the
        // hub reaches step R + d + 1. A stall holds the release step to `until`.
        let stall = *self.stalled_until.get(&from.0).unwrap_or(&0);
        let release = self.step.max(stall);
        let base_deliver_at = release + self.delay_of(from) + 1;

        // Chaos knobs for this source (all 0/absent ⇒ the clean path below).
        let loss = self.loss.get(&from.0).copied().unwrap_or(0);
        let jitter = self.jitter.get(&from.0).copied().unwrap_or(0);
        let reorder = self.reorder.get(&from.0).copied().unwrap_or(0);
        let chaotic = self.chaos_active && (loss > 0 || jitter > 0 || reorder > 0);

        for o in outs {
            let mut deliver_at = base_deliver_at;
            if chaotic {
                // Roll the seeded chaos RNG ONCE per message, in this fixed order
                // (loss, then jitter, then reorder), so the stream is a pure
                // function of (seed, traffic). Loss is decided first; jitter and
                // reorder rolls are still consumed on a dropped message so the
                // per-message RNG advance is independent of the drop outcome.
                let dropped = loss > 0 && self.chaos.below(1000) < loss;
                if jitter > 0 {
                    deliver_at += self.chaos.below(jitter + 1);
                }
                if reorder > 0 {
                    deliver_at += self.chaos.below(reorder + 1);
                }
                if dropped {
                    // Reliable channels would retransmit in production; here the
                    // drop is permanent and recovery is the digest→snapshot path.
                    self.seq += 1; // keep seq monotonic so ordering stays stable
                    continue;
                }
            }
            self.inflight.push(InFlight {
                deliver_at,
                seq: self.seq,
                to: o.to,
                msg: Inbound {
                    from,
                    channel: o.channel,
                    bytes: o.bytes,
                },
            });
            self.seq += 1;
        }
    }

    /// Advance one step, delivering all matured messages into inboxes in a
    /// deterministic order (`deliver_at`, then send `seq`).
    pub fn advance(&mut self) {
        self.step += 1;
        let now = self.step;
        // Stable partition: matured vs still-in-flight.
        let mut matured: Vec<InFlight> = Vec::new();
        let mut rest: Vec<InFlight> = Vec::new();
        for f in self.inflight.drain(..) {
            if f.deliver_at <= now {
                matured.push(f);
            } else {
                rest.push(f);
            }
        }
        self.inflight = rest;
        matured.sort_by_key(|f| (f.deliver_at, f.seq));
        for f in matured {
            self.inboxes.entry(f.to.0).or_default().push(f.msg);
        }
    }

    /// Drain `peer`'s inbox.
    pub fn take(&mut self, peer: PeerId) -> Vec<Inbound> {
        self.inboxes.remove(&peer.0).unwrap_or_default()
    }

    /// True if no messages are queued or in flight anywhere.
    pub fn idle(&self) -> bool {
        self.inflight.is_empty() && self.inboxes.values().all(|v| v.is_empty())
    }
}

/// A single participant's [`Transport`] view onto a shared [`Hub`]. Several
/// endpoints share one `Rc<RefCell<Hub>>`; the driver still calls
/// [`Hub::advance`] centrally to move the global clock. This is the test-side
/// implementation of the same trait the Steam adapter implements in production,
/// so a driver written against `Transport` runs unchanged on either bus.
pub struct HubEndpoint {
    hub: Rc<RefCell<Hub>>,
    me: PeerId,
}

impl HubEndpoint {
    pub fn new(hub: Rc<RefCell<Hub>>, me: PeerId) -> HubEndpoint {
        HubEndpoint { hub, me }
    }
}

impl Transport for HubEndpoint {
    fn send(&mut self, outs: Vec<Outbound>) {
        self.hub.borrow_mut().send(self.me, outs);
    }
    fn poll(&mut self) -> Vec<Inbound> {
        self.hub.borrow_mut().take(self.me)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Channel, Outbound};

    fn out(to: u32) -> Outbound {
        Outbound {
            to: PeerId(to),
            channel: Channel::Control,
            bytes: vec![1],
        }
    }

    #[test]
    fn zero_delay_delivers_next_step() {
        let mut h = Hub::new();
        h.send(PeerId(0), vec![out(1)]);
        assert!(h.take(PeerId(1)).is_empty()); // not yet
        h.advance();
        assert_eq!(h.take(PeerId(1)).len(), 1);
    }

    #[test]
    fn delay_postpones_delivery() {
        let mut h = Hub::new();
        h.set_delay(PeerId(0), 3);
        h.send(PeerId(0), vec![out(1)]);
        for _ in 0..3 {
            h.advance();
            assert!(h.take(PeerId(1)).is_empty());
        }
        h.advance();
        assert_eq!(h.take(PeerId(1)).len(), 1);
    }

    #[test]
    fn stall_holds_then_releases() {
        let mut h = Hub::new();
        h.stall(PeerId(2), 5);
        h.send(PeerId(2), vec![out(0)]);
        for _ in 0..5 {
            h.advance();
            assert!(h.take(PeerId(0)).is_empty());
        }
        h.advance(); // step 6 > stall 5
        assert_eq!(h.take(PeerId(0)).len(), 1);
    }

    /// Loss eventually drops some — but not all — of a stream of messages, and
    /// the count is a deterministic function of the seed (same seed ⇒ same drops).
    #[test]
    fn loss_drops_a_seeded_fraction() {
        fn delivered(seed: u64) -> usize {
            let mut h = Hub::with_chaos_seed(seed);
            h.set_loss(PeerId(0), 400); // ~40%
            let n = 1000;
            for _ in 0..n {
                h.send(PeerId(0), vec![out(1)]);
            }
            // Flush everything in flight.
            h.advance();
            h.take(PeerId(1)).len()
        }
        let a = delivered(0xABC);
        let b = delivered(0xABC);
        assert_eq!(a, b, "loss must be reproducible for a fixed seed");
        assert!(
            a > 0 && a < 1000,
            "≈40% loss should drop some, keep some: {a}"
        );
        // Roughly in the expected band (wide tolerance — this is a spot check).
        assert!(
            (400..=800).contains(&a),
            "delivered {a} not near ~60% of 1000"
        );
        // A different seed yields a different (but still partial) drop pattern.
        assert!(delivered(0x999) > 0);
    }

    /// Full loss (1000‰) black-holes the link entirely.
    #[test]
    fn full_loss_delivers_nothing() {
        let mut h = Hub::with_chaos_seed(1);
        h.set_loss(PeerId(0), 1000);
        for _ in 0..50 {
            h.send(PeerId(0), vec![out(1)]);
        }
        h.advance();
        assert!(
            h.take(PeerId(1)).is_empty(),
            "a 1000‰ link delivers nothing"
        );
    }

    /// Jitter keeps every message (no loss) but spreads deliveries across a few
    /// steps; total delivered count is conserved.
    #[test]
    fn jitter_preserves_all_messages() {
        let mut h = Hub::with_chaos_seed(7);
        h.set_jitter(PeerId(0), 4);
        let n = 200u32;
        for _ in 0..n {
            h.send(PeerId(0), vec![out(1)]);
        }
        // Drain over enough steps to clear base delay (1) + max jitter (4).
        let mut total = 0;
        for _ in 0..10 {
            h.advance();
            total += h.take(PeerId(1)).len();
        }
        assert_eq!(
            total as u32, n,
            "jitter must not drop or duplicate messages"
        );
    }

    /// Reorder makes same-step messages arrive out of send order for at least one
    /// message, deterministically. We tag each message by its payload byte and
    /// check the delivered sequence is a permutation that is NOT sorted.
    #[test]
    fn reorder_permutes_within_window() {
        fn tagged(to: u32, tag: u8) -> Outbound {
            Outbound {
                to: PeerId(to),
                channel: Channel::Control,
                bytes: vec![tag],
            }
        }
        let mut h = Hub::with_chaos_seed(0xD15);
        h.set_reorder(PeerId(0), 3);
        // Send a burst of tagged messages on ONE step.
        let burst: Vec<Outbound> = (0..16u8).map(|i| tagged(1, i)).collect();
        h.send(PeerId(0), burst);
        // Drain across the reorder window.
        let mut got: Vec<u8> = Vec::new();
        for _ in 0..6 {
            h.advance();
            for inb in h.take(PeerId(1)) {
                got.push(inb.bytes[0]);
            }
        }
        assert_eq!(got.len(), 16, "no message lost under reorder");
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            (0..16u8).collect::<Vec<_>>(),
            "every tag delivered once"
        );
        assert_ne!(got, sorted, "reorder must actually permute the send order");
    }

    /// `clear_chaos` heals a black-holed link: messages flow again afterwards.
    #[test]
    fn clear_chaos_heals_the_link() {
        let mut h = Hub::with_chaos_seed(5);
        h.set_loss(PeerId(0), 1000);
        h.send(PeerId(0), vec![out(1)]);
        h.advance();
        assert!(h.take(PeerId(1)).is_empty(), "fully lossy before heal");
        h.clear_chaos();
        h.send(PeerId(0), vec![out(1)]);
        h.advance();
        assert_eq!(
            h.take(PeerId(1)).len(),
            1,
            "link delivers again after clear_chaos"
        );
    }

    /// A hub with NO chaos knobs set behaves exactly like the legacy hub: the
    /// chaos RNG is never consumed, so plain delay/stall semantics are intact.
    #[test]
    fn no_chaos_is_a_clean_link() {
        let mut h = Hub::with_chaos_seed(123);
        h.set_delay(PeerId(0), 2);
        for _ in 0..5 {
            h.send(PeerId(0), vec![out(1)]);
        }
        // delay 2 ⇒ delivered at step 3; nothing before.
        h.advance();
        h.advance();
        assert!(h.take(PeerId(1)).is_empty());
        h.advance();
        assert_eq!(
            h.take(PeerId(1)).len(),
            5,
            "clean link delivers all, on time"
        );
    }

    // Drive two participants purely through the `Transport` trait (no direct
    // Hub calls), proving HubEndpoint is a faithful stand-in for the Steam
    // adapter: A.send -> central advance -> B.poll.
    #[test]
    fn endpoints_route_over_transport_trait() {
        use crate::transport::Transport;
        let hub = Rc::new(RefCell::new(Hub::new()));
        let mut a = HubEndpoint::new(hub.clone(), PeerId(0));
        let mut b = HubEndpoint::new(hub.clone(), PeerId(1));
        a.send(vec![out(1)]);
        assert!(b.poll().is_empty()); // not delivered until the clock advances
        hub.borrow_mut().advance();
        let got = b.poll();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from, PeerId(0));
        assert!(a.poll().is_empty()); // nothing addressed back to A
    }
}
