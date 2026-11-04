use core::f32::consts::PI;
use std::{env, net::UdpSocket};

use hound;
use nik8_sender::tracker::{ScheduleLeaderUpdater, track_slots_update};
use nik8_sender::{refresh_leader_schedule, webtransport::serve_transaction_ingress, ws_url};
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::{EncodableKey, Signer};
use solana_tpu_client_next::{
    Client, ClientBuilder, TransactionSender, leader_updater::LeaderUpdater,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = dotenvy::dotenv();
    refresh_leader_schedule().await;
    tokio::spawn(track_slots_update(ws_url()));

    let mut updater_preview = ScheduleLeaderUpdater;
    println!("next TPU leaders: {:?}", updater_preview.next_leaders(3));

    let (sender, _tpu_client) = build_tpu_relay()?;
    let port = env::var("NIK8_WEBTRANSPORT_PORT")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(4433);
    let certificate_pem = env::var("NIK8_WEBTRANSPORT_CERT_PEM")?;
    let private_key_pem = env::var("NIK8_WEBTRANSPORT_KEY_PEM")?;

    serve_transaction_ingress(port, certificate_pem, private_key_pem, sender).await
}

fn build_tpu_relay() -> Result<(TransactionSender, Client), Box<dyn std::error::Error + Send + Sync>>
{
    let socket = UdpSocket::bind("0.0.0.0:0")?;

    ClientBuilder::new(Box::new(ScheduleLeaderUpdater))
        .bind_socket(socket.into())
        .leader_send_fanout(2)
        .build()
        .map_err(Into::into)
}

pub async fn do_hound() {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create("sine.wav", spec).unwrap();
    for t in (0..(44100 * 2)).map(|x| x as f32 / 44100.0) {
        let sample = (t * 440.0 * 2.0 * PI).sin();
        let amplitude = i16::MAX as f32;
        writer.write_sample((sample * amplitude) as i16).unwrap();
    }
}

// Demo code - not sent
// pub async fn send_without_rpc(
//     recent_blockhash: solana_hash::Hash,
// ) -> Result<(TransactionSender, Client), Box<dyn std::error::Error + Send + Sync>> {
//     use solana_system_interface::instruction;
//     use solana_transaction::Transaction;

//     let (sender, client) = build_tpu_relay()?;

//     let user_keypair =
//         Keypair::read_from_file("./accounts/NK8sp7K77FesYKacsCu4dxuMGrPyVjUqdQdDQCHNLtU.json")
//             .expect("why would this fail");

//     let recipient = Address::from_str_const("nK8dDDndjpNWAJGp8yoJVyPjRLMvB284G6Z2ZMU2B1e");
//     let transfer = instruction::transfer(&user_keypair.pubkey(), &recipient, 1_000_000);
//     let transaction = Transaction::new_signed_with_payer(
//         &[transfer],
//         Some(&user_keypair.pubkey()),
//         &[&user_keypair],
//         recent_blockhash,
//     );

//     println!("transaction signature is: {}", transaction.signatures[0]);
//     sender
//         .send_transaction(bincode::serialize(&transaction)?)
//         .await?;

//     Ok((sender, client))
// }
