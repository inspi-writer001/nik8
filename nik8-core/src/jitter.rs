//! Per-talk-session reordering. Packets can arrive out of order (different
//! transactions land in different orders) or not at all; playback never waits
//! long for a missing one.

use std::collections::BTreeMap;

/// How many packets past a gap we hold before declaring the gap lost.
/// Two 120 ms packets ride in each transaction, so this is about one transaction.
const MAX_HELD: usize = 2;

#[derive(Debug, Eq, PartialEq)]
pub enum Playout {
    Packet(Vec<u8>),
    /// A missing packet; play concealment in its place.
    Lost,
}

/// Orders packets of one talk session by sequence number.
#[derive(Default)]
pub struct JitterBuffer {
    next: Option<u32>,
    held: BTreeMap<u32, Vec<u8>>,
}

impl JitterBuffer {
    /// Adds a packet and returns whatever is now ready to play, in order.
    pub fn push(&mut self, sequence: u32, opus: Vec<u8>) -> Vec<Playout> {
        let next = *self.next.get_or_insert(sequence);
        if sequence < next {
            return Vec::new(); // too late; already played or concealed
        }
        self.held.insert(sequence, opus);
        self.drain()
    }

    fn drain(&mut self) -> Vec<Playout> {
        let mut ready = Vec::new();
        loop {
            let next = self.next.expect("set on first push");
            if let Some(opus) = self.held.remove(&next) {
                ready.push(Playout::Packet(opus));
                self.next = Some(next.wrapping_add(1));
            } else if self.held.len() > MAX_HELD {
                ready.push(Playout::Lost);
                self.next = Some(next.wrapping_add(1));
            } else {
                return ready;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(byte: u8) -> Vec<u8> {
        vec![byte]
    }

    fn played(ready: Vec<Playout>) -> Vec<Option<u8>> {
        ready
            .into_iter()
            .map(|playout| match playout {
                Playout::Packet(opus) => Some(opus[0]),
                Playout::Lost => None,
            })
            .collect()
    }

    #[test]
    fn plays_in_order_packets_immediately() {
        let mut buffer = JitterBuffer::default();
        assert_eq!(played(buffer.push(5, packet(5))), [Some(5)]);
        assert_eq!(played(buffer.push(6, packet(6))), [Some(6)]);
    }

    #[test]
    fn reorders_within_the_window() {
        let mut buffer = JitterBuffer::default();
        buffer.push(0, packet(0));
        assert!(buffer.push(2, packet(2)).is_empty());
        assert_eq!(played(buffer.push(1, packet(1))), [Some(1), Some(2)]);
    }

    #[test]
    fn conceals_a_gap_once_enough_later_packets_arrive() {
        let mut buffer = JitterBuffer::default();
        buffer.push(0, packet(0));
        assert!(buffer.push(2, packet(2)).is_empty());
        assert!(buffer.push(3, packet(3)).is_empty());
        assert_eq!(
            played(buffer.push(4, packet(4))),
            [None, Some(2), Some(3), Some(4)]
        );
        // The lost packet showing up late is ignored.
        assert!(buffer.push(1, packet(1)).is_empty());
    }

    #[test]
    fn drops_duplicates_of_played_packets() {
        let mut buffer = JitterBuffer::default();
        buffer.push(0, packet(0));
        assert!(buffer.push(0, packet(0)).is_empty());
    }
}
