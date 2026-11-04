use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
    time::Duration,
};

use nik8_consumer::{
    ConsumerConfig, SubscriptionError, SubscriptionEvent, extract_voice_packets, run_subscription,
};
use nik8_shared::VoicePacket;
use tokio_util::sync::CancellationToken;

use crate::{
    audio::VoiceDecoder,
    jitter::{JitterBuffer, Playout},
};

/// How many recent `(session, sequence)` pairs are remembered for deduplication.
/// At 120 ms per packet this covers about a minute of one talker.
const DEDUPE_WINDOW: usize = 512;
/// Concurrent talk sessions decoded at once; the oldest is dropped beyond this.
const MAX_PLAYBACK_SESSIONS: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListenerConfig {
    pub consumer: ConsumerConfig,
    /// Only packets addressed to this account are accepted.
    pub recipient: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListenerState {
    Connecting,
    Online,
    Reconnecting {
        attempt: u32,
        delay: Duration,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ListenerStats {
    /// Packets for this recipient, after deduplication.
    pub accepted: u64,
    pub duplicates: u64,
    /// Valid packets addressed to someone else.
    pub other_recipients: u64,
    /// Malformed instructions or updates.
    pub rejected: u64,
    /// Well-formed packets in a version/codec this build doesn't support.
    pub unsupported: u64,
    pub last_session_id: Option<[u8; 16]>,
    pub last_sequence: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListenerEvent {
    State(ListenerState),
    Stats(ListenerStats),
    /// 120 ms of decoded 16 kHz mono PCM, in playback order.
    Audio(Vec<f32>),
}

/// Listens for voice packets addressed to `config.recipient` until cancelled or a
/// fatal subscription error. Emits state changes, updated stats and decoded audio.
pub async fn run_listener(
    config: &ListenerConfig,
    cancel: &CancellationToken,
    on_event: impl FnMut(ListenerEvent),
) -> Result<(), SubscriptionError> {
    // Both subscription callbacks emit events; a Mutex (not RefCell) keeps the future Send.
    let on_event = Mutex::new(on_event);
    let emit = |event| (on_event.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))(event);
    let mut receiver = Receiver::new(config.recipient);
    let mut playback = Playback::default();
    let mut accepted = Vec::new();

    run_subscription(
        &config.consumer,
        cancel,
        |event| {
            let state = match event {
                SubscriptionEvent::Connecting => ListenerState::Connecting,
                SubscriptionEvent::Live => ListenerState::Online,
                SubscriptionEvent::Reconnecting {
                    attempt,
                    delay,
                    reason,
                } => ListenerState::Reconnecting {
                    attempt,
                    delay,
                    reason,
                },
            };
            emit(ListenerEvent::State(state));
        },
        |update| {
            let extracted = extract_voice_packets(update, &config.consumer.program_id);
            accepted.clear();
            if receiver.ingest(&extracted.packets, &extracted.rejected, &mut accepted) {
                emit(ListenerEvent::Stats(receiver.stats));
            }
            for packet in accepted.drain(..) {
                for pcm in playback.play(packet) {
                    emit(ListenerEvent::Audio(pcm));
                }
            }
        },
    )
    .await
}

/// A packet that passed recipient filtering and deduplication.
struct AcceptedPacket {
    session_id: [u8; 16],
    sequence: u32,
    opus: Vec<u8>,
}

/// Reordering and decoding, one jitter buffer and decoder per talk session.
#[derive(Default)]
struct Playback {
    sessions: HashMap<[u8; 16], (JitterBuffer, VoiceDecoder)>,
    order: VecDeque<[u8; 16]>,
}

impl Playback {
    fn play(&mut self, packet: AcceptedPacket) -> Vec<Vec<f32>> {
        if !self.sessions.contains_key(&packet.session_id) {
            let Ok(decoder) = VoiceDecoder::new() else {
                return Vec::new();
            };
            if self.order.len() == MAX_PLAYBACK_SESSIONS
                && let Some(oldest) = self.order.pop_front()
            {
                self.sessions.remove(&oldest);
            }
            self.order.push_back(packet.session_id);
            self.sessions
                .insert(packet.session_id, (JitterBuffer::default(), decoder));
        }
        let (buffer, decoder) = self
            .sessions
            .get_mut(&packet.session_id)
            .expect("inserted above");

        buffer
            .push(packet.sequence, packet.opus)
            .into_iter()
            .map(|playout| match playout {
                Playout::Packet(opus) => decoder.decode(&opus).unwrap_or_else(|_| decoder.conceal()),
                Playout::Lost => decoder.conceal(),
            })
            .collect()
    }
}

/// Recipient filtering, deduplication and counters. Pure; no I/O.
struct Receiver {
    recipient: [u8; 32],
    seen: HashSet<([u8; 16], u32)>,
    seen_order: VecDeque<([u8; 16], u32)>,
    stats: ListenerStats,
}

impl Receiver {
    fn new(recipient: [u8; 32]) -> Self {
        Self {
            recipient,
            seen: HashSet::with_capacity(DEDUPE_WINDOW),
            seen_order: VecDeque::with_capacity(DEDUPE_WINDOW),
            stats: ListenerStats::default(),
        }
    }

    /// Appends packets for this recipient to `accepted`; returns whether the stats changed.
    fn ingest(
        &mut self,
        packets: &[VoicePacket<'_>],
        rejected: &[nik8_consumer::Nik8InstructionError],
        accepted: &mut Vec<AcceptedPacket>,
    ) -> bool {
        let before = self.stats;

        for error in rejected {
            if error.is_unsupported_format() {
                self.stats.unsupported += 1;
            } else {
                self.stats.rejected += 1;
            }
        }

        for packet in packets {
            let header = &packet.header;
            if header.recipient != self.recipient {
                self.stats.other_recipients += 1;
                continue;
            }

            let key = (header.session_id, header.sequence());
            if !self.remember(key) {
                self.stats.duplicates += 1;
                continue;
            }

            self.stats.accepted += 1;
            self.stats.last_session_id = Some(header.session_id);
            self.stats.last_sequence = Some(header.sequence());
            accepted.push(AcceptedPacket {
                session_id: header.session_id,
                sequence: header.sequence(),
                opus: packet.opus_packet.to_vec(),
            });
        }

        self.stats != before
    }

    /// Returns false if `key` was already seen.
    fn remember(&mut self, key: ([u8; 16], u32)) -> bool {
        if !self.seen.insert(key) {
            return false;
        }
        self.seen_order.push_back(key);
        if self.seen_order.len() > DEDUPE_WINDOW
            && let Some(oldest) = self.seen_order.pop_front()
        {
            self.seen.remove(&oldest);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nik8_consumer::Nik8InstructionError;
    use nik8_shared::{VoicePacketError, VoicePacketHeader};

    const ME: [u8; 32] = [8; 32];
    const SOMEONE_ELSE: [u8; 32] = [5; 32];

    fn packet(recipient: [u8; 32], session: u8, sequence: u32) -> VoicePacket<'static> {
        VoicePacket {
            header: VoicePacketHeader {
                version: 1,
                codec: 1,
                session_id: [session; 16],
                recipient,
                sequence_le: sequence.to_le_bytes(),
                capture_timestamp_ms_le: [0; 8],
            },
            opus_packet: &[0xF8],
        }
    }

    #[test]
    fn accepts_only_packets_for_this_recipient() {
        let mut receiver = Receiver::new(ME);
        assert!(receiver.ingest(&[packet(ME, 1, 1), packet(SOMEONE_ELSE, 1, 2)], &[], &mut Vec::new()));

        assert_eq!(receiver.stats.accepted, 1);
        assert_eq!(receiver.stats.other_recipients, 1);
        assert_eq!(receiver.stats.last_sequence, Some(1));
    }

    #[test]
    fn drops_duplicates_per_session() {
        let mut receiver = Receiver::new(ME);
        receiver.ingest(&[packet(ME, 1, 1), packet(ME, 1, 1), packet(ME, 2, 1)], &[], &mut Vec::new());

        assert_eq!(receiver.stats.accepted, 2);
        assert_eq!(receiver.stats.duplicates, 1);
    }

    #[test]
    fn dedupe_window_is_bounded() {
        let mut receiver = Receiver::new(ME);
        for sequence in 0..(DEDUPE_WINDOW as u32 + 10) {
            receiver.ingest(&[packet(ME, 1, sequence)], &[], &mut Vec::new());
        }

        assert_eq!(receiver.seen.len(), DEDUPE_WINDOW);
        assert_eq!(receiver.seen_order.len(), DEDUPE_WINDOW);
    }

    #[test]
    fn counts_unsupported_separately_from_malformed() {
        let mut receiver = Receiver::new(ME);
        receiver.ingest(
            &[],
            &[
                Nik8InstructionError::InvalidVoicePacket(VoicePacketError::UnsupportedFormat),
                Nik8InstructionError::MissingMessage,
            ],
            &mut Vec::new(),
        );

        assert_eq!(receiver.stats.unsupported, 1);
        assert_eq!(receiver.stats.rejected, 1);
    }

    #[test]
    fn reports_no_change_for_empty_updates() {
        let mut receiver = Receiver::new(ME);
        assert!(!receiver.ingest(&[], &[], &mut Vec::new()));
    }

    #[test]
    fn parses_base58_addresses() {
        assert_eq!(
            crate::parse_address("NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU"),
            Some(nik8_consumer::NIK8_PROGRAM_ID_BYTES)
        );
        assert_eq!(crate::parse_address("not an address"), None);
    }

    #[test]
    fn playback_decodes_per_session_in_order() {
        let mut encoder = crate::audio::VoiceEncoder::new().unwrap();
        let opus = encoder.push(&[0; crate::audio::PACKET_SAMPLES * 2]).unwrap();
        let mut playback = Playback::default();

        let accepted = |sequence: u32, opus: &Vec<u8>| AcceptedPacket {
            session_id: [1; 16],
            sequence,
            opus: opus.clone(),
        };
        assert_eq!(playback.play(accepted(0, &opus[0])).len(), 1);
        assert_eq!(playback.play(accepted(1, &opus[1])).len(), 1);
        assert!(playback.play(accepted(1, &opus[1])).is_empty());
    }
}
