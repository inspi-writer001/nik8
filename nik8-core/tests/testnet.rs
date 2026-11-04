//! Live round trips against the deployed program on the cluster in `RPC_URL`. Spends a few thousand
//! lamports. Run with:
//!   cargo test -p nik8_core --test testnet -- --ignored --nocapture
//!
//! Needs `PAYER_KEYPAIR`, `RPC_URL`, `NIK8_GRPC_ENDPOINT` (and
//! optionally `NIK8_GRPC_X_TOKEN`) in the workspace `.env`.

use std::{
    env, fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nik8_core::{ConsumerConfig, Sender, rpc::RpcClient};
use nik8_shared::{VoicePacket, VoicePacketHeader};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends real transactions"]
async fn registers_and_sends_a_voice_packet_to_self() {
    let dotenv = dotenvy::dotenv().expect("workspace .env");
    let root = dotenv.parent().unwrap();

    let keypair_path = root.join(env::var("PAYER_KEYPAIR").unwrap());
    let seed = read_seed(&keypair_path);
    let rpc_url = env::var("RPC_URL").unwrap();
    let chain = ConsumerConfig::from_env().unwrap();

    let sender = Sender::new(&seed, chain, &rpc_url).unwrap();
    let status = sender.status().await.unwrap();
    println!(
        "sender {} vault {} balance {} registered {}",
        status.pubkey_base58(),
        status.sender_vault_base58(),
        status.balance_lamports,
        status.registered
    );

    match sender.register().await.unwrap() {
        Some(signature) => println!("registered: {signature}"),
        None => println!("already registered"),
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let session_id = (now_ms as u128).to_le_bytes();
    // Opus TOC for a 20 ms SILK frame followed by silence; the program doesn't decode audio.
    let opus = [0x08, 0xFF, 0xFE];

    let packet = VoicePacket {
        header: VoicePacketHeader::new(session_id, status.pubkey, 1, now_ms),
        opus_packet: &opus,
    };
    let signature = sender.send_voice(&[packet], false).await.unwrap();
    println!("voice packet: {signature}");

    RpcClient::new(&rpc_url)
        .unwrap()
        .confirm(&signature, Duration::from_secs(45))
        .await
        .unwrap();
    println!("confirmed");
}

/// First 32 bytes of a Solana CLI keypair JSON array are the ed25519 seed.
fn read_seed(path: &Path) -> [u8; 32] {
    let bytes: Vec<u8> = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    bytes[..32].try_into().unwrap()
}

/// Full voice path: WAV → Opus → transactions → program → Yellowstone → decode → WAV.
/// Writes what was heard to `target/roundtrip-received.wav`. Spends devnet/testnet lamports.
#[test]
#[ignore = "sends real transactions"]
fn round_trips_audio_through_the_chain() {
    use std::sync::{Arc, Mutex};

    use nik8_core::{
        ListenerConfig, ListenerEvent, ListenerState, Talk, TalkEvent, audio::SAMPLE_RATE,
        run_listener,
    };
    use tokio_util::sync::CancellationToken;

    let dotenv = dotenvy::dotenv().expect("workspace .env");
    let root = dotenv.parent().unwrap().to_owned();
    let seed = read_seed(&root.join(env::var("PAYER_KEYPAIR").unwrap()));
    let rpc_url = env::var("RPC_URL").unwrap();
    let chain = ConsumerConfig::from_env().unwrap();

    // opus-rs needs ~3 MB of stack in release, far more unoptimized.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();

    let sender = Arc::new(Sender::new(&seed, chain.clone(), &rpc_url).unwrap());
    let me = sender.pubkey();
    runtime.block_on(sender.prepare()).unwrap();

    let online = Arc::new(Mutex::new(false));
    let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
    let cancel = CancellationToken::new();
    runtime.spawn({
        let (online, heard, cancel) = (online.clone(), heard.clone(), cancel.clone());
        let config = ListenerConfig {
            consumer: chain,
            recipient: me,
        };
        async move {
            run_listener(&config, &cancel, |event| match event {
                ListenerEvent::State(ListenerState::Online) => *online.lock().unwrap() = true,
                ListenerEvent::Audio(pcm) => heard.lock().unwrap().extend(pcm),
                _ => {}
            })
            .await
            .unwrap();
        }
    });
    while !*online.lock().unwrap() {
        std::thread::sleep(Duration::from_millis(100));
    }

    let sent = Arc::new(Mutex::new((0_u32, Vec::<String>::new())));
    let talk = Talk::start(sender, me, runtime.handle(), {
        let sent = sent.clone();
        Arc::new(move |event| {
            let mut sent = sent.lock().unwrap();
            match event {
                TalkEvent::Sent { packets, .. } => sent.0 += packets,
                TalkEvent::Failed { reason } => sent.1.push(reason),
            }
        })
    })
    .unwrap();

    let mut wav = hound::WavReader::open(root.join("voice-hello-there.wav")).unwrap();
    assert_eq!(wav.spec().sample_rate, SAMPLE_RATE as u32);
    let pcm: Vec<i16> = wav.samples::<i16>().map(Result::unwrap).collect();
    let started = std::time::Instant::now();
    for chunk in pcm.chunks(960) {
        talk.push_pcm(chunk.to_vec());
        std::thread::sleep(Duration::from_millis(60)); // real-time, like a mic
    }
    talk.finish();
    let expected = pcm.len().div_ceil(nik8_core::audio::PACKET_SAMPLES) * nik8_core::audio::PACKET_SAMPLES;

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while heard.lock().unwrap().len() < expected && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let first_heard = started.elapsed();
    cancel.cancel();

    let heard = heard.lock().unwrap().clone();
    let (sent_packets, failures) = sent.lock().unwrap().clone();
    let out = root.join("target/roundtrip-received.wav");
    let mut writer = hound::WavWriter::create(
        &out,
        hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in &heard {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();

    println!(
        "sent {sent_packets} packets ({} failures), heard {:.2}s of {:.2}s, done after {:.1}s → {}",
        failures.len(),
        heard.len() as f32 / SAMPLE_RATE as f32,
        expected as f32 / SAMPLE_RATE as f32,
        first_heard.as_secs_f32(),
        out.display()
    );
    for failure in failures.iter().take(3) {
        println!("  failure: {failure}");
    }
    assert!(heard.len() * 10 >= expected * 9, "lost more than 10% of the audio");
}

/// Per-packet latency: from when a packet's first sample was captured to when it
/// arrived on Yellowstone. Includes batching, signing, submission and inclusion.
/// Sender and receiver share this machine's clock, so the numbers are exact.
#[test]
#[ignore = "sends real transactions"]
fn measures_packet_latency() {
    use std::sync::{Arc, Mutex};

    use nik8_consumer::{SubscriptionEvent, extract_voice_packets, run_subscription};
    use nik8_core::{Talk, TalkEvent, audio::PACKET_SAMPLES};
    use tokio_util::sync::CancellationToken;

    let dotenv = dotenvy::dotenv().expect("workspace .env");
    let root = dotenv.parent().unwrap().to_owned();
    let seed = read_seed(&root.join(env::var("PAYER_KEYPAIR").unwrap()));
    let rpc_url = env::var("RPC_URL").unwrap();
    let chain = ConsumerConfig::from_env().unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    // Ingress when configured, unless NIK8_SUBMIT=rpc forces the RPC path.
    let mut sender = Sender::new(&seed, chain.clone(), &rpc_url).unwrap();
    let configured = |name| env::var(name).ok().filter(|value: &String| !value.trim().is_empty());
    if let (Some(url), Some(hash), false) = (
        configured("NIK8_INGRESS_URL"),
        configured("NIK8_INGRESS_CERT_SHA256"),
        env::var("NIK8_SUBMIT").as_deref() == Ok("rpc"),
    ) {
        let hash = nik8_core::ingress::parse_sha256_hex(&hash).expect("NIK8_INGRESS_CERT_SHA256");
        sender = sender.with_ingress(&url, hash).unwrap();
    }
    let (path, packets_per_tx) = (sender.submit_path(), sender.packets_per_tx());
    let sender = Arc::new(sender);
    let me = sender.pubkey();
    runtime.block_on(sender.prepare()).unwrap();

    let started_ms = now_ms();
    let online = Arc::new(Mutex::new(false));
    let arrivals = Arc::new(Mutex::new(Vec::<(u32, u64)>::new()));
    let cancel = CancellationToken::new();
    runtime.spawn({
        let (online, arrivals, cancel, chain) = (online.clone(), arrivals.clone(), cancel.clone(), chain.clone());
        async move {
            let program = chain.program_id;
            run_subscription(
                &chain,
                &cancel,
                |event| {
                    if event == SubscriptionEvent::Live {
                        *online.lock().unwrap() = true;
                    }
                },
                |update| {
                    let arrived = now_ms();
                    for packet in extract_voice_packets(update, &program).packets {
                        let header = packet.header;
                        if header.recipient == me && header.capture_timestamp_ms() >= started_ms {
                            let latency = arrived.saturating_sub(header.capture_timestamp_ms());
                            arrivals.lock().unwrap().push((header.sequence(), latency));
                        }
                    }
                },
            )
            .await
            .unwrap();
        }
    });
    while !*online.lock().unwrap() {
        std::thread::sleep(Duration::from_millis(100));
    }

    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let talk = Talk::start(sender, me, runtime.handle(), {
        let failures = failures.clone();
        Arc::new(move |event| {
            if let TalkEvent::Failed { reason } = event {
                failures.lock().unwrap().push(reason);
            }
        })
    })
    .unwrap();

    let mut wav = hound::WavReader::open(root.join("voice-hello-there.wav")).unwrap();
    let pcm: Vec<i16> = wav.samples::<i16>().map(Result::unwrap).collect();
    for chunk in pcm.chunks(960) {
        talk.push_pcm(chunk.to_vec());
        std::thread::sleep(Duration::from_millis(60));
    }
    talk.finish();

    let expected = pcm.len().div_ceil(PACKET_SAMPLES);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while arrivals.lock().unwrap().len() < expected && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    cancel.cancel();

    let mut latencies: Vec<u64> = arrivals.lock().unwrap().iter().map(|(_, ms)| *ms).collect();
    latencies.sort_unstable();
    let percentile = |p: f64| latencies[((latencies.len() - 1) as f64 * p).round() as usize];
    let failures = failures.lock().unwrap();
    println!(
        "path={path:?} packets_per_tx={packets_per_tx} received {}/{expected} | capture→arrival p50 {} ms, p95 {} ms, max {} ms | {} send failures",
        latencies.len(),
        percentile(0.5),
        percentile(0.95),
        latencies[latencies.len() - 1],
        failures.len()
    );
    for failure in failures.iter().take(3) {
        println!("  failure: {failure}");
    }
    assert!(latencies.len() * 10 >= expected * 9, "lost more than 10% of packets");
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
