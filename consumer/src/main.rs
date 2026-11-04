use nik8_consumer::{ConsumerConfig, SubscriptionEvent, extract_voice_packets, run_subscription};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ConsumerConfig::from_env()?;

    let cancel = CancellationToken::new();
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        }
    });

    let on_event = |event| match event {
        SubscriptionEvent::Connecting => println!("connecting to Yellowstone"),
        SubscriptionEvent::Live => println!("Nik8 Yellowstone subscription active"),
        SubscriptionEvent::Reconnecting {
            attempt,
            delay,
            reason,
        } => eprintln!("reconnecting (attempt {attempt}, in {delay:?}): {reason}"),
    };

    let on_update = |update: &_| {
        let extracted = extract_voice_packets(update, &config.program_id);

        for packet in &extracted.packets {
            println!(
                "voice packet: recipient={:02x?}, session={:02x?}, sequence={}, timestamp_ms={}, opus_bytes={}",
                packet.header.recipient,
                packet.header.session_id,
                packet.header.sequence(),
                packet.header.capture_timestamp_ms(),
                packet.opus_packet.len(),
            );
        }
        for rejection in &extracted.rejected {
            eprintln!("skipped instruction: {rejection}");
        }
    };

    run_subscription(&config, &cancel, on_event, on_update).await?;
    Ok(())
}
