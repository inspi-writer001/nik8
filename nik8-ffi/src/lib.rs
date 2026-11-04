use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use nik8_core::{
    ConsumerConfig, ListenerConfig, ListenerEvent, Sender, SubmitPath, Talk, TalkEvent,
    ingress::parse_sha256_hex, parse_address, run_listener,
};
use nik8_shared::{VoicePacket, VoicePacketError, VoicePacketHeader};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

uniffi::setup_scaffolding!();

/// Crate version, used by the app to confirm the native library loaded.
#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum Nik8FfiError {
    #[error("voice packet has no Opus data")]
    MissingOpusPacket,
    #[error("unsupported voice packet version or codec")]
    UnsupportedFormat,
    #[error("Opus packet exceeds 1275 bytes")]
    OpusPacketTooLarge,
    #[error("invalid configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("native runtime failed: {reason}")]
    Runtime { reason: String },
    #[error("voice packet buffer too small")]
    BufferTooSmall,
    #[error("no session key configured")]
    NoSession,
    #[error("already talking")]
    AlreadyTalking,
    #[error("{reason}")]
    Network { reason: String },
}

impl From<VoicePacketError> for Nik8FfiError {
    fn from(error: VoicePacketError) -> Self {
        match error {
            VoicePacketError::MissingOpusPacket => Self::MissingOpusPacket,
            VoicePacketError::UnsupportedFormat => Self::UnsupportedFormat,
            VoicePacketError::OpusPacketTooLarge => Self::OpusPacketTooLarge,
            VoicePacketError::BufferTooSmall => Self::BufferTooSmall,
        }
    }
}

#[derive(Debug, uniffi::Record)]
pub struct VoicePacketInfo {
    pub version: u8,
    pub codec: u8,
    pub session_id: Vec<u8>,
    pub recipient: Vec<u8>,
    pub sequence: u32,
    pub capture_timestamp_ms: u64,
    pub opus_len: u32,
}

/// Parses a v1 voice packet payload (instruction data without the discriminator).
#[uniffi::export]
pub fn parse_voice_packet(data: Vec<u8>) -> Result<VoicePacketInfo, Nik8FfiError> {
    let packet = VoicePacket::parse(&data)?;
    let header = packet.header;

    Ok(VoicePacketInfo {
        version: header.version,
        codec: header.codec,
        session_id: header.session_id.to_vec(),
        recipient: header.recipient.to_vec(),
        sequence: header.sequence(),
        capture_timestamp_ms: header.capture_timestamp_ms(),
        opus_len: packet.opus_packet.len() as u32,
    })
}

#[derive(Debug, uniffi::Enum)]
pub enum ListenerState {
    Connecting,
    Online,
    Reconnecting {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
}

#[derive(Debug, uniffi::Record)]
pub struct ListenerStats {
    pub accepted: u64,
    pub duplicates: u64,
    pub other_recipients: u64,
    pub rejected: u64,
    pub unsupported: u64,
    pub last_sequence: Option<u32>,
}

/// Implemented by the platform layer. Called from native worker threads.
#[uniffi::export(with_foreign)]
pub trait ListenerCallback: Send + Sync {
    fn on_state(&self, state: ListenerState);
    fn on_stats(&self, stats: ListenerStats);
    /// The listener stopped on an error retrying cannot fix (bad endpoint, rejected token).
    fn on_failed(&self, reason: String);
    /// 120 ms of decoded 16 kHz mono PCM16 (little-endian bytes), in playback order.
    fn on_audio(&self, pcm: Vec<u8>);
}

/// Progress of a push-to-talk press. Called from native worker threads.
#[uniffi::export(with_foreign)]
pub trait TalkCallback: Send + Sync {
    fn on_sent(&self, packets: u32, signature: String);
    fn on_failed(&self, reason: String);
}

#[derive(Debug, uniffi::Record)]
pub struct SessionInfo {
    /// Session key address (base58); the `sender` on every voice packet.
    pub pubkey: String,
    /// `nik8_user` fee vault PDA (base58).
    pub sender_vault: String,
    pub balance_lamports: u64,
    pub registered: bool,
}

const CODEC_STACK_BYTES: usize = 8 * 1024 * 1024;

/// Opus TOC for a 20 ms SILK frame followed by silence. The program doesn't decode audio.
const TEST_OPUS_PACKET: [u8; 3] = [0x08, 0xFF, 0xFE];

/// Owns the Tokio runtime, the active listener and the session key. One per app.
#[derive(uniffi::Object)]
pub struct Nik8Client {
    runtime: Runtime,
    listening: Mutex<Option<CancellationToken>>,
    session: Mutex<Option<Arc<Sender>>>,
    talk: Mutex<Option<Talk>>,
    test_session_id: [u8; 16],
    test_sequence: AtomicU32,
}

#[uniffi::export]
impl Nik8Client {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, Nik8FfiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("nik8-rt")
            // Opus encode/decode runs on these threads; opus-rs needs ~3 MB of stack.
            .thread_stack_size(CODEC_STACK_BYTES)
            .enable_all()
            .build()
            .map_err(|error| Nik8FfiError::Runtime {
                reason: error.to_string(),
            })?;

        Ok(Arc::new(Self {
            runtime,
            listening: Mutex::new(None),
            session: Mutex::new(None),
            talk: Mutex::new(None),
            // Unique enough per app launch to keep test sequences from colliding.
            test_session_id: now_ms().to_le_bytes(),
            test_sequence: AtomicU32::new(1),
        }))
    }

    /// Loads the session key from its 32-byte seed. The platform layer generates and
    /// stores the seed (encrypted); it never leaves the device. Returns the address.
    ///
    /// With `ingress_url` and `ingress_cert_sha256` (hex), voice goes through the
    /// `sender` ingress; otherwise, and as a fallback, through `rpc_url`.
    pub fn configure_session(
        &self,
        seed: Vec<u8>,
        rpc_url: String,
        grpc_endpoint: String,
        grpc_token: Option<String>,
        ingress_url: Option<String>,
        ingress_cert_sha256: Option<String>,
    ) -> Result<String, Nik8FfiError> {
        let invalid = |reason: String| Nik8FfiError::InvalidConfig { reason };

        let seed: [u8; 32] = seed
            .try_into()
            .map_err(|_| invalid("session seed must be 32 bytes".into()))?;
        let chain = ConsumerConfig::new(grpc_endpoint, grpc_token)
            .map_err(|error| invalid(error.to_string()))?;
        let mut sender = Sender::new(&seed, chain, &rpc_url).map_err(invalid)?;
        if let Some(url) = ingress_url.filter(|url| !url.trim().is_empty()) {
            let hash = ingress_cert_sha256
                .as_deref()
                .and_then(parse_sha256_hex)
                .ok_or_else(|| invalid("ingress certificate SHA-256 missing or invalid".into()))?;
            sender = sender.with_ingress(&url, hash).map_err(invalid)?;
        }
        let pubkey = nik8_core::rpc::base58(&sender.pubkey());

        *lock(&self.session) = Some(Arc::new(sender));
        Ok(pubkey)
    }

    /// "ingress" or "rpc": how voice transactions are submitted.
    pub fn submit_path(&self) -> Result<String, Nik8FfiError> {
        Ok(match self.sender()?.submit_path() {
            SubmitPath::Ingress => "ingress",
            SubmitPath::Rpc => "rpc",
        }
        .to_owned())
    }

    /// Balance and registration of the session key. Blocks on the network.
    pub fn session_status(&self) -> Result<SessionInfo, Nik8FfiError> {
        let sender = self.sender()?;
        let status = self
            .runtime
            .block_on(sender.status())
            .map_err(|reason| Nik8FfiError::Network { reason })?;

        Ok(SessionInfo {
            pubkey: status.pubkey_base58(),
            sender_vault: status.sender_vault_base58(),
            balance_lamports: status.balance_lamports,
            registered: status.registered,
        })
    }

    /// Runs `Cre8Account` for the session key and waits for confirmation.
    /// Returns the signature, or `None` if it was already registered.
    pub fn register_sender(&self) -> Result<Option<String>, Nik8FfiError> {
        let sender = self.sender()?;
        self.runtime
            .block_on(sender.register())
            .map_err(|reason| Nik8FfiError::Network { reason })
    }

    /// Sends one silent voice packet to `recipient` (base58). Returns the signature.
    pub fn send_test_packet(&self, recipient: String) -> Result<String, Nik8FfiError> {
        let sender = self.sender()?;
        let recipient = parse_address(&recipient).ok_or_else(|| Nik8FfiError::InvalidConfig {
            reason: "recipient is not a valid address".into(),
        })?;
        let sequence = self.test_sequence.fetch_add(1, Ordering::Relaxed);
        let packet = VoicePacket {
            header: VoicePacketHeader::new(
                self.test_session_id,
                recipient,
                sequence,
                now_ms() as u64,
            ),
            opus_packet: &TEST_OPUS_PACKET,
        };

        self.runtime
            .block_on(sender.send_voice(&[packet], false))
            .map_err(|reason| Nik8FfiError::Network { reason })
    }

    /// Fetches a blockhash ahead of time so the first push doesn't wait for one.
    pub fn prepare_to_talk(&self) {
        if let Ok(sender) = self.sender() {
            self.runtime.spawn(async move {
                let _ = sender.prepare().await;
            });
        }
    }

    /// Starts a push-to-talk press addressed to `recipient` (base58). Feed audio with
    /// `push_pcm` and end with `stop_talking`.
    pub fn start_talking(
        &self,
        recipient: String,
        callback: Arc<dyn TalkCallback>,
    ) -> Result<(), Nik8FfiError> {
        let sender = self.sender()?;
        let recipient = parse_address(&recipient).ok_or_else(|| Nik8FfiError::InvalidConfig {
            reason: "recipient is not a valid address".into(),
        })?;

        let mut talk = lock(&self.talk);
        if talk.is_some() {
            return Err(Nik8FfiError::AlreadyTalking);
        }
        *talk = Some(
            Talk::start(
                sender,
                recipient,
                self.runtime.handle(),
                Arc::new(move |event| match event {
                    TalkEvent::Sent { packets, signature } => callback.on_sent(packets, signature),
                    TalkEvent::Failed { reason } => callback.on_failed(reason),
                }),
            )
            .map_err(|reason| Nik8FfiError::Runtime { reason })?,
        );
        Ok(())
    }

    /// Queues captured 16 kHz mono PCM16 (little-endian bytes, as `AudioRecord`
    /// produces) for the current press. Ignored when not talking.
    pub fn push_pcm(&self, pcm: Vec<u8>) {
        if let Some(talk) = lock(&self.talk).as_ref() {
            talk.push_pcm(
                pcm.chunks_exact(2)
                    .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
                    .collect(),
            );
        }
    }

    /// Ends the current press; remaining audio is flushed and sent.
    pub fn stop_talking(&self) {
        if let Some(talk) = lock(&self.talk).take() {
            talk.finish();
        }
    }

    /// Starts (or restarts) listening for packets addressed to `recipient` (base58).
    pub fn start_listening(
        &self,
        grpc_endpoint: String,
        grpc_token: Option<String>,
        recipient: String,
        callback: Arc<dyn ListenerCallback>,
    ) -> Result<(), Nik8FfiError> {
        let invalid = |reason: String| Nik8FfiError::InvalidConfig { reason };

        let recipient =
            parse_address(&recipient).ok_or_else(|| invalid("recipient is not a valid address".into()))?;
        let consumer = ConsumerConfig::new(grpc_endpoint, grpc_token)
            .map_err(|error| invalid(error.to_string()))?;
        let config = ListenerConfig {
            consumer,
            recipient,
        };

        self.stop_listening();
        let cancel = CancellationToken::new();
        *self.lock_listening() = Some(cancel.clone());

        self.runtime.spawn(async move {
            let result = run_listener(&config, &cancel, |event| match event {
                ListenerEvent::State(state) => callback.on_state(state.into()),
                ListenerEvent::Stats(stats) => callback.on_stats(stats.into()),
                ListenerEvent::Audio(pcm) => callback.on_audio(pcm16_le_bytes(&pcm)),
            })
            .await;

            if let Err(error) = result {
                callback.on_failed(error.to_string());
            }
        });

        Ok(())
    }

    pub fn stop_listening(&self) {
        if let Some(cancel) = self.lock_listening().take() {
            cancel.cancel();
        }
    }
}

impl Nik8Client {
    fn lock_listening(&self) -> std::sync::MutexGuard<'_, Option<CancellationToken>> {
        lock(&self.listening)
    }

    fn sender(&self) -> Result<Arc<Sender>, Nik8FfiError> {
        lock(&self.session).clone().ok_or(Nik8FfiError::NoSession)
    }
}

/// These locks only guard an `Option`; a panic while holding one leaves nothing
/// half-updated, so a poisoned lock is safe to reuse.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// f32 PCM in [-1, 1] to little-endian PCM16 bytes, as `AudioTrack` takes.
fn pcm16_le_bytes(pcm: &[f32]) -> Vec<u8> {
    pcm.iter()
        .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes())
        .collect()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

impl Drop for Nik8Client {
    fn drop(&mut self) {
        self.stop_talking();
        self.stop_listening();
    }
}

impl From<nik8_core::ListenerState> for ListenerState {
    fn from(state: nik8_core::ListenerState) -> Self {
        match state {
            nik8_core::ListenerState::Connecting => Self::Connecting,
            nik8_core::ListenerState::Online => Self::Online,
            nik8_core::ListenerState::Reconnecting {
                attempt,
                delay,
                reason,
            } => Self::Reconnecting {
                attempt,
                delay_ms: delay.as_millis() as u64,
                reason,
            },
        }
    }
}

impl From<nik8_core::ListenerStats> for ListenerStats {
    fn from(stats: nik8_core::ListenerStats) -> Self {
        Self {
            accepted: stats.accepted,
            duplicates: stats.duplicates,
            other_recipients: stats.other_recipients,
            rejected: stats.rejected,
            unsupported: stats.unsupported,
            last_sequence: stats.last_sequence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn parses_through_ffi_surface() {
        let mut data = vec![VoicePacket::VERSION, VoicePacket::CODEC_OPUS];
        data.extend_from_slice(&[7; 16]);
        data.extend_from_slice(&[8; 32]);
        data.extend_from_slice(&42_u32.to_le_bytes());
        data.extend_from_slice(&1_700_000_000_000_u64.to_le_bytes());
        data.extend_from_slice(&[0xF8, 0xFF, 0xFE]);

        let info = parse_voice_packet(data).unwrap();
        assert_eq!(info.sequence, 42);
        assert_eq!(info.recipient, vec![8; 32]);
        assert_eq!(info.opus_len, 3);
    }

    #[test]
    fn maps_parse_errors() {
        assert!(matches!(
            parse_voice_packet(vec![1; 62]),
            Err(Nik8FfiError::MissingOpusPacket)
        ));
    }

    struct Recorder(Mutex<mpsc::Sender<String>>);

    impl ListenerCallback for Recorder {
        fn on_state(&self, state: ListenerState) {
            let _ = self.0.lock().unwrap().send(format!("{state:?}"));
        }
        fn on_stats(&self, _stats: ListenerStats) {}
        fn on_failed(&self, reason: String) {
            let _ = self.0.lock().unwrap().send(format!("failed: {reason}"));
        }
        fn on_audio(&self, _pcm: Vec<u8>) {}
    }

    #[test]
    fn converts_pcm_to_le_bytes() {
        assert_eq!(pcm16_le_bytes(&[0.0, 1.0, -1.0, 2.0]), [0, 0, 0xFF, 0x7F, 0x01, 0x80, 0xFF, 0x7F]);
    }

    #[test]
    fn rejects_bad_recipient_before_starting() {
        let client = Nik8Client::new().unwrap();
        let (tx, _rx) = mpsc::channel();
        let result = client.start_listening(
            "https://example.com".into(),
            None,
            "nope".into(),
            Arc::new(Recorder(Mutex::new(tx))),
        );
        assert!(matches!(result, Err(Nik8FfiError::InvalidConfig { .. })));
    }

    #[test]
    fn reports_reconnects_and_stops_on_request() {
        let client = Nik8Client::new().unwrap();
        let (tx, rx) = mpsc::channel();
        client
            .start_listening(
                "http://127.0.0.1:1".into(),
                None,
                "NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU".into(),
                Arc::new(Recorder(Mutex::new(tx))),
            )
            .unwrap();

        let timeout = std::time::Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(timeout).unwrap(), "Connecting");
        assert!(rx.recv_timeout(timeout).unwrap().starts_with("Reconnecting"));

        client.stop_listening();
        // Drain anything already in flight, then expect silence.
        std::thread::sleep(std::time::Duration::from_millis(300));
        while rx.try_recv().is_ok() {}
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(rx.try_recv().is_err());
    }
}
