use std::{
    collections::HashMap,
    error::Error,
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures::{SinkExt, StreamExt};
use solana_address::Address;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tonic::{Code, Status};
use yellowstone_grpc_client::{
    ClientTlsConfig, GeyserGrpcBuilderError, GeyserGrpcClient, GeyserGrpcClientError,
    GeyserStream, SubscribeRequestSink,
};
use yellowstone_grpc_proto::prelude::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions, SubscribeRequestPing,
    SubscribeUpdate, subscribe_update::UpdateOneof,
};

use crate::config::ConsumerConfig;

/// The deployed Nik8 program whose transaction instructions we consume.
pub const NIK8_PROGRAM_ID: &str = "NiK8BGgPDKystF2Azoj87yQM7VS6wcYzmm48ynjRHTU";

/// `NIK8_PROGRAM_ID` decoded from base58 once at build time.
pub const NIK8_PROGRAM_ID_BYTES: [u8; 32] = Address::from_str_const(NIK8_PROGRAM_ID).to_bytes();

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP2_KEEPALIVE: Duration = Duration::from_secs(15);
/// We ping the provider so a quiet chain still produces traffic (pongs).
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// No message at all (including pongs) for this long means the stream is dead.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const BACKOFF_INITIAL: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(20);
/// A stream that stayed up this long resets the backoff.
const HEALTHY_AFTER: Duration = Duration::from_secs(30);

/// Processed, successful, non-vote transactions that include `program_id`.
pub fn subscribe_request(program_id: &[u8; 32]) -> SubscribeRequest {
    SubscribeRequest {
        transactions: HashMap::from([(
            "nik8".to_owned(),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(false),
                account_include: vec![Address::new_from_array(*program_id).to_string()],
                ..Default::default()
            },
        )]),
        commitment: Some(CommitmentLevel::Processed as i32),
        ..Default::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionEvent {
    Connecting,
    Live,
    Reconnecting {
        attempt: u32,
        delay: Duration,
        reason: String,
    },
}

/// Errors that retrying cannot fix (bad endpoint, rejected credentials).
#[derive(Debug)]
pub enum SubscriptionError {
    InvalidConfig(String),
    Rejected(Status),
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => write!(formatter, "invalid gRPC config: {reason}"),
            Self::Rejected(status) => write!(
                formatter,
                "provider rejected subscription ({:?}): {}",
                status.code(),
                status.message()
            ),
        }
    }
}

impl Error for SubscriptionError {}

/// Keeps a Nik8 subscription alive until `cancel` fires or a fatal error occurs.
///
/// Reconnects with jittered exponential backoff, answers provider pings, sends its
/// own pings and treats a silent stream as dead. After a reconnect it resumes live;
/// missed updates are not replayed (stale voice is useless).
pub async fn run_subscription(
    config: &ConsumerConfig,
    cancel: &CancellationToken,
    mut on_event: impl FnMut(SubscriptionEvent),
    mut on_update: impl FnMut(&SubscribeUpdate),
) -> Result<(), SubscriptionError> {
    let mut backoff = Backoff::new(BACKOFF_INITIAL, BACKOFF_MAX);

    loop {
        on_event(SubscriptionEvent::Connecting);

        let reason = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            connected = connect_and_subscribe(config) => match connected {
                Ok((sink, stream)) => {
                    on_event(SubscriptionEvent::Live);
                    let live_since = Instant::now();
                    let reason = pump(sink, stream, cancel, &mut on_update).await;
                    if cancel.is_cancelled() {
                        return Ok(());
                    }
                    if live_since.elapsed() >= HEALTHY_AFTER {
                        backoff.reset();
                    }
                    reason
                }
                Err(AttemptError::Fatal(error)) => return Err(error),
                Err(AttemptError::Transient(reason)) => reason,
            },
        };

        let (attempt, delay) = backoff.next();
        on_event(SubscriptionEvent::Reconnecting {
            attempt,
            delay,
            reason,
        });

        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = sleep(delay) => {}
        }
    }
}

/// Reads one live stream until it ends, errors, stalls or is cancelled.
/// Returns the reason it stopped.
async fn pump(
    mut sink: SubscribeRequestSink,
    mut stream: GeyserStream,
    cancel: &CancellationToken,
    on_update: &mut impl FnMut(&SubscribeUpdate),
) -> String {
    let mut ping_timer = interval(PING_INTERVAL);
    ping_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return "cancelled".to_owned(),
            _ = ping_timer.tick() => {
                if let Err(error) = sink.send(ping_request()).await {
                    return format!("ping send failed: {error}");
                }
            }
            next = timeout(STALL_TIMEOUT, stream.next()) => match next {
                Err(_) => return format!("no data for {}s", STALL_TIMEOUT.as_secs()),
                Ok(None) => return "stream closed by provider".to_owned(),
                Ok(Some(Err(status))) => return format!("stream error: {status}"),
                Ok(Some(Ok(update))) => match update.update_oneof {
                    Some(UpdateOneof::Ping(_)) => {
                        if let Err(error) = sink.send(ping_request()).await {
                            return format!("pong send failed: {error}");
                        }
                    }
                    Some(UpdateOneof::Pong(_)) => {}
                    _ => on_update(&update),
                },
            },
        }
    }
}

enum AttemptError {
    Fatal(SubscriptionError),
    Transient(String),
}

async fn connect(config: &ConsumerConfig) -> Result<GeyserGrpcClient, AttemptError> {
    let invalid = |error: GeyserGrpcBuilderError| {
        AttemptError::Fatal(SubscriptionError::InvalidConfig(error.to_string()))
    };

    GeyserGrpcClient::build_from_shared(config.grpc_endpoint.clone())
        .map_err(invalid)?
        .x_token(config.x_token.clone())
        .map_err(invalid)?
        .tls_config(tls_config())
        .map_err(invalid)?
        .connect_timeout(CONNECT_TIMEOUT)
        .http2_keep_alive_interval(HTTP2_KEEPALIVE)
        .keep_alive_while_idle(true)
        .connect()
        .await
        .map_err(|error| AttemptError::Transient(format!("connect failed: {}", chain(&error))))
}

async fn connect_and_subscribe(
    config: &ConsumerConfig,
) -> Result<(SubscribeRequestSink, GeyserStream), AttemptError> {
    connect(config)
        .await?
        .subscribe_with_request(Some(subscribe_request(&config.program_id)))
        .await
        .map_err(classify)
}

/// A `confirmed` recent blockhash from the provider. `processed` hashes can belong
/// to a fork that is later dropped, which would fail the transaction.
pub async fn latest_blockhash(config: &ConsumerConfig) -> Result<[u8; 32], String> {
    let mut client = connect(config).await.map_err(|error| match error {
        AttemptError::Fatal(error) => error.to_string(),
        AttemptError::Transient(reason) => reason,
    })?;
    let response = client
        .get_latest_blockhash(Some(CommitmentLevel::Confirmed))
        .await
        .map_err(|error| format!("get_latest_blockhash failed: {}", chain(&error)))?;

    response
        .blockhash
        .parse::<Address>()
        .map(|hash| hash.to_bytes())
        .map_err(|_| format!("provider returned an invalid blockhash: {}", response.blockhash))
}

fn classify(error: GeyserGrpcClientError) -> AttemptError {
    match error {
        GeyserGrpcClientError::TonicStatus(status) if is_fatal(status.code()) => {
            AttemptError::Fatal(SubscriptionError::Rejected(status))
        }
        error => AttemptError::Transient(format!("subscribe failed: {}", chain(&error))),
    }
}

/// `error: cause: cause`, deduplicating tonic's repeated "transport error" layers.
fn chain(error: &dyn Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if parts.last() != Some(&text) {
            parts.push(text);
        }
        source = cause.source();
    }
    parts.join(": ")
}

fn is_fatal(code: Code) -> bool {
    matches!(
        code,
        Code::Unauthenticated | Code::PermissionDenied | Code::InvalidArgument
    )
}

/// Android has no system store rustls can read, so mobile builds ship webpki roots.
fn tls_config() -> ClientTlsConfig {
    if cfg!(target_os = "android") {
        ClientTlsConfig::new().with_webpki_roots()
    } else {
        ClientTlsConfig::new().with_native_roots()
    }
}

fn ping_request() -> SubscribeRequest {
    SubscribeRequest {
        ping: Some(SubscribeRequestPing { id: 1 }),
        ..Default::default()
    }
}

struct Backoff {
    initial: Duration,
    max: Duration,
    attempt: u32,
}

impl Backoff {
    fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            attempt: 0,
        }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Returns the attempt number and a delay jittered into `[base / 2, base]`.
    fn next(&mut self) -> (u32, Duration) {
        let base = self
            .initial
            .saturating_mul(1 << self.attempt.min(16))
            .min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        (self.attempt, base / 2 + base.mul_f64(jitter_fraction() / 2.0))
    }
}

/// Cheap `[0, 1)` jitter; spreading reconnects doesn't need a real RNG.
fn jitter_fraction() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    f64::from(nanos % 1_000) / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_caps_and_resets() {
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(1));

        let delays: Vec<Duration> = (0..6).map(|_| backoff.next().1).collect();
        assert!(delays[0] >= Duration::from_millis(50) && delays[0] <= Duration::from_millis(100));
        assert!(delays[2] >= Duration::from_millis(200) && delays[2] <= Duration::from_millis(400));
        assert!(delays[5] <= Duration::from_secs(1));

        backoff.reset();
        let (attempt, delay) = backoff.next();
        assert_eq!(attempt, 1);
        assert!(delay <= Duration::from_millis(100));
    }

    #[test]
    fn only_credential_and_request_errors_are_fatal() {
        assert!(is_fatal(Code::Unauthenticated));
        assert!(is_fatal(Code::PermissionDenied));
        assert!(!is_fatal(Code::Unavailable));
        assert!(!is_fatal(Code::DeadlineExceeded));
        assert!(!is_fatal(Code::Internal));
    }

    #[test]
    fn subscribes_to_the_configured_program() {
        let request = subscribe_request(&NIK8_PROGRAM_ID_BYTES);
        let filter = &request.transactions["nik8"];
        assert_eq!(filter.account_include, [NIK8_PROGRAM_ID]);
        assert_eq!(filter.vote, Some(false));
        assert_eq!(filter.failed, Some(false));
        assert_eq!(request.commitment, Some(CommitmentLevel::Processed as i32));
    }
}
