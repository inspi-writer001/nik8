//! Push-to-talk transmit: PCM in, signed voice transactions out.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use nik8_shared::{VoicePacket, VoicePacketHeader};
use tokio::{runtime::Handle, sync::mpsc};

use crate::{
    audio::{PACKET_MS, VoiceEncoder},
    sender::Sender,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TalkEvent {
    Sent { packets: u32, signature: String },
    Failed { reason: String },
}

pub type TalkEventSink = Arc<dyn Fn(TalkEvent) + Send + Sync>;

enum TalkInput {
    Pcm(Vec<i16>),
    Finish,
}

/// One press of the Push button. Encoding runs on `runtime`, whose threads must
/// have large stacks (opus-rs needs ~3 MB); the caller's audio thread only queues PCM.
pub struct Talk {
    input: mpsc::UnboundedSender<TalkInput>,
}

impl Talk {
    pub fn start(
        sender: Arc<Sender>,
        recipient: [u8; 32],
        runtime: &Handle,
        on_event: TalkEventSink,
    ) -> Result<Self, String> {
        let mut session_id = [0_u8; 16];
        getrandom::fill(&mut session_id).map_err(|error| format!("random session id: {error}"))?;
        let (input, mut rx) = mpsc::unbounded_channel();

        runtime.spawn(async move {
            let mut transmitter = match Transmitter::new(sender, recipient, session_id, on_event.clone()) {
                Ok(transmitter) => transmitter,
                Err(reason) => return on_event(TalkEvent::Failed { reason }),
            };
            while let Some(message) = rx.recv().await {
                let result = match message {
                    TalkInput::Pcm(samples) => transmitter.push_pcm(&samples),
                    TalkInput::Finish => break,
                };
                if let Err(reason) = result {
                    on_event(TalkEvent::Failed { reason });
                }
            }
            // Released (or the handle was dropped): send what's left.
            if let Err(reason) = transmitter.finish() {
                on_event(TalkEvent::Failed { reason });
            }
        });

        Ok(Self { input })
    }

    /// Queues captured 16 kHz mono PCM. Never blocks.
    pub fn push_pcm(&self, samples: Vec<i16>) {
        let _ = self.input.send(TalkInput::Pcm(samples));
    }

    /// Flushes the remaining audio and ends the press.
    pub fn finish(self) {
        let _ = self.input.send(TalkInput::Finish);
    }
}

/// Fresh session ID and sequence numbers from 0 for each press.
struct Transmitter {
    sender: Arc<Sender>,
    recipient: [u8; 32],
    session_id: [u8; 16],
    started_ms: u64,
    encoder: VoiceEncoder,
    next_sequence: u32,
    /// Packets per transaction for the sender's submit path.
    batch: usize,
    pending: Vec<(u32, Vec<u8>)>,
    on_event: TalkEventSink,
}

impl Transmitter {
    fn new(
        sender: Arc<Sender>,
        recipient: [u8; 32],
        session_id: [u8; 16],
        on_event: TalkEventSink,
    ) -> Result<Self, String> {
        let batch = sender.packets_per_tx();
        Ok(Self {
            sender,
            recipient,
            session_id,
            started_ms: now_ms(),
            encoder: VoiceEncoder::new()?,
            next_sequence: 0,
            batch,
            pending: Vec::with_capacity(batch),
            on_event,
        })
    }

    fn push_pcm(&mut self, samples: &[i16]) -> Result<(), String> {
        for opus in self.encoder.push(samples)? {
            self.queue(opus);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        for opus in self.encoder.flush()? {
            self.queue(opus);
        }
        self.dispatch();
        Ok(())
    }

    fn queue(&mut self, opus: Vec<u8>) {
        self.pending.push((self.next_sequence, opus));
        self.next_sequence = self.next_sequence.wrapping_add(1);
        if self.pending.len() == self.batch {
            self.dispatch();
        }
    }

    /// Sends queued packets without waiting for the RPC. Transactions may land out
    /// of order; the receiver's jitter buffer reorders them.
    fn dispatch(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        let sender = Arc::clone(&self.sender);
        let on_event = Arc::clone(&self.on_event);
        let (recipient, session_id, started_ms) = (self.recipient, self.session_id, self.started_ms);

        tokio::spawn(async move {
            let packets: Vec<VoicePacket<'_>> = batch
                .iter()
                .map(|(sequence, opus)| VoicePacket {
                    header: VoicePacketHeader::new(
                        session_id,
                        recipient,
                        *sequence,
                        started_ms + u64::from(*sequence) * PACKET_MS,
                    ),
                    opus_packet: opus,
                })
                .collect();

            on_event(match sender.send_voice(&packets, true).await {
                Ok(signature) => TalkEvent::Sent {
                    packets: packets.len() as u32,
                    signature,
                },
                Err(reason) => TalkEvent::Failed { reason },
            });
        });
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
