use std::{error::Error, io, path::Path};

use solana_address::Address;
use solana_tpu_client_next::TransactionSender;
use wtransport::{Endpoint, Identity, RecvStream, ServerConfig, endpoint::IncomingSession};

use crate::validate::{program_id, validate_nik8_transaction};

/// Solana's maximum wire transaction size; also bounds per-stream memory.
const MAX_WIRE_TRANSACTION_BYTES: usize = 1_232;

type WebTransportResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub async fn serve_transaction_ingress(
    port: u16,
    certificate_pem: impl AsRef<Path>,
    private_key_pem: impl AsRef<Path>,
    transaction_sender: TransactionSender,
) -> WebTransportResult<()> {
    let identity = Identity::load_pemfiles(certificate_pem, private_key_pem).await?;
    let server_config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .build();
    let endpoint = Endpoint::server(server_config)?;
    let program_id = program_id()?;

    println!("Nik8 WebTransport ingress listening on UDP port {port} (program {program_id})");

    loop {
        let incoming_session = endpoint.accept().await;
        let transaction_sender = transaction_sender.clone();

        tokio::spawn(async move {
            if let Err(error) = handle_session(incoming_session, transaction_sender, program_id).await {
                eprintln!("WebTransport session closed: {error}");
            }
        });
    }
}

async fn handle_session(
    incoming_session: IncomingSession,
    transaction_sender: TransactionSender,
    program_id: Address,
) -> WebTransportResult<()> {
    let session_request = incoming_session.await?;
    let connection = session_request.accept().await?;
    println!("ingress session from {}", connection.remote_address());

    loop {
        let stream = connection.accept_uni().await?;
        let transaction_sender = transaction_sender.clone();

        tokio::spawn(async move {
            if let Err(error) = forward_stream(stream, transaction_sender, &program_id).await {
                eprintln!("WebTransport transaction rejected: {error}");
            }
        });
    }
}

async fn forward_stream(
    mut stream: RecvStream,
    transaction_sender: TransactionSender,
    program_id: &Address,
) -> WebTransportResult<()> {
    let mut transaction = Vec::with_capacity(MAX_WIRE_TRANSACTION_BYTES);
    let mut buffer = [0_u8; MAX_WIRE_TRANSACTION_BYTES];

    while let Some(bytes_read) = stream.read(&mut buffer).await? {
        if transaction.len() + bytes_read > MAX_WIRE_TRANSACTION_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "wire transaction exceeds Solana's packet limit",
            )
            .into());
        }

        transaction.extend_from_slice(&buffer[..bytes_read]);
    }

    validate_nik8_transaction(&transaction, program_id)?;
    transaction_sender.send_transaction(transaction).await?;
    Ok(())
}
