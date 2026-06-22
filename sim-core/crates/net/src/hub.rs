//! Deterministic in-process transport for tests. Routes [`Outbound`] →
//! [`Inbound`] with per-source one-way delay and stall injection, so we can
//! prove the independence thesis (`docs/06` M2 exit test) without Steam.
//!
//! Latency is measured in driver iterations: a message sent on step `S` over a
//! link with delay `d` is delivered when the hub reaches step `S + d + 1`. A
//! "stall" holds a source's messages until a given step.

use crate::transport::{Inbound, Outbound, PeerId, Transport};
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
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
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

    fn delay_of(&self, peer: PeerId) -> u32 {
        *self.delay.get(&peer.0).unwrap_or(&0)
    }

    /// Enqueue everything `from` is sending this step.
    pub fn send(&mut self, from: PeerId, outs: Vec<Outbound>) {
        // A message released on step R with one-way delay d is delivered when the
        // hub reaches step R + d + 1. A stall holds the release step to `until`.
        let stall = *self.stalled_until.get(&from.0).unwrap_or(&0);
        let release = self.step.max(stall);
        let deliver_at = release + self.delay_of(from) + 1;
        for o in outs {
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
