//! Session-key operations against the deployed program: register, send voice packets.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use nik8_consumer::{ConsumerConfig, latest_blockhash};
use nik8_shared::VoicePacket;

use crate::{
    ingress::Ingress,
    rpc::{RpcClient, base58},
    tx::{
        SessionKey, build_create_sender_tx, build_voice_tx, sender_vault_address,
        transaction_signature,
    },
};

const REGISTER_CONFIRM_TIMEOUT: Duration = Duration::from_secs(45);
/// Blockhashes are valid for ~60 s; refreshing every 20 s leaves plenty of margin.
const BLOCKHASH_MAX_AGE: Duration = Duration::from_secs(20);
/// Public RPC limits `sendTransaction` to ~4/s, so RPC sends batch two 120 ms packets.
const RPC_PACKETS_PER_TX: usize = 2;
/// The ingress has no such limit; one packet per transaction saves 120 ms of waiting.
const INGRESS_PACKETS_PER_TX: usize = 1;

/// How voice transactions reach the leaders.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitPath {
    /// WebTransport to our `sender`, which forwards to the leaders' TPUs.
    Ingress,
    /// JSON-RPC `sendTransaction`; the RPC node forwards.
    Rpc,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStatus {
    pub pubkey: [u8; 32],
    pub sender_vault: [u8; 32],
    pub balance_lamports: u64,
    /// Whether the `nik8_user` fee vault exists, i.e. `Cre8Account` has run.
    pub registered: bool,
}

pub struct Sender {
    key: SessionKey,
    /// Yellowstone connection (blockhashes) and the program ID.
    chain: ConsumerConfig,
    rpc: RpcClient,
    ingress: Option<Ingress>,
    blockhash: Mutex<Option<([u8; 32], Instant)>>,
}

impl Sender {
    pub fn new(seed: &[u8; 32], chain: ConsumerConfig, rpc_url: &str) -> Result<Self, String> {
        Ok(Self {
            key: SessionKey::from_seed(seed),
            chain,
            rpc: RpcClient::new(rpc_url)?,
            ingress: None,
            blockhash: Mutex::new(None),
        })
    }

    /// Sends voice through the `sender` ingress at `url`, whose certificate must hash
    /// to `cert_sha256`. RPC remains the fallback.
    pub fn with_ingress(mut self, url: &str, cert_sha256: [u8; 32]) -> Result<Self, String> {
        self.ingress = Some(Ingress::new(url, cert_sha256)?);
        Ok(self)
    }

    pub fn submit_path(&self) -> SubmitPath {
        if self.ingress.is_some() {
            SubmitPath::Ingress
        } else {
            SubmitPath::Rpc
        }
    }

    /// 120 ms voice packets per transaction for the current submit path.
    pub fn packets_per_tx(&self) -> usize {
        match self.submit_path() {
            SubmitPath::Ingress => INGRESS_PACKETS_PER_TX,
            SubmitPath::Rpc => RPC_PACKETS_PER_TX,
        }
    }

    pub fn pubkey(&self) -> [u8; 32] {
        self.key.pubkey()
    }

    pub fn sender_vault(&self) -> [u8; 32] {
        sender_vault_address(&self.chain.program_id, &self.pubkey())
    }

    pub async fn status(&self) -> Result<SessionStatus, String> {
        let pubkey = self.pubkey();
        let sender_vault = self.sender_vault();
        let (balance_lamports, registered) = tokio::try_join!(
            self.rpc.balance(&pubkey),
            self.rpc.account_exists(&sender_vault)
        )?;

        Ok(SessionStatus {
            pubkey,
            sender_vault,
            balance_lamports,
            registered,
        })
    }

    /// Runs `Cre8Account` for the session key if it isn't registered yet and waits
    /// for confirmation. Returns the signature, or `None` if already registered.
    pub async fn register(&self) -> Result<Option<String>, String> {
        if self.rpc.account_exists(&self.sender_vault()).await? {
            return Ok(None);
        }

        let blockhash = self.recent_blockhash().await?;
        let transaction = build_create_sender_tx(&self.key, &self.chain.program_id, &blockhash)
            .map_err(|error| error.to_string())?;
        let signature = self.rpc.send_transaction(&transaction, false).await?;
        self.rpc
            .confirm(&signature, REGISTER_CONFIRM_TIMEOUT)
            .await?;
        Ok(Some(signature))
    }

    /// Signs and submits voice packets in one transaction. Returns the signature (base58).
    ///
    /// Uses the ingress when configured and falls back to RPC if it fails.
    /// `skip_preflight` (RPC only) trades error detail for latency; live audio uses it.
    pub async fn send_voice(
        &self,
        packets: &[VoicePacket<'_>],
        skip_preflight: bool,
    ) -> Result<String, String> {
        let blockhash = self.recent_blockhash().await?;
        let transaction = build_voice_tx(&self.key, &self.chain.program_id, packets, &blockhash)
            .map_err(|error| error.to_string())?;

        if let Some(ingress) = &self.ingress {
            match ingress.send(&transaction).await {
                Ok(()) => return Ok(signature_base58(&transaction)),
                Err(ingress_error) => {
                    return self
                        .rpc
                        .send_transaction(&transaction, skip_preflight)
                        .await
                        .map_err(|rpc_error| format!("{ingress_error}; RPC fallback: {rpc_error}"));
                }
            }
        }
        self.rpc
            .send_transaction(&transaction, skip_preflight)
            .await
    }

    /// Fetches a blockhash and opens the ingress connection now, so the first
    /// packet of a talk doesn't wait for either.
    pub async fn prepare(&self) -> Result<(), String> {
        self.recent_blockhash().await?;
        if let Some(ingress) = &self.ingress {
            ingress.warm_up().await?;
        }
        Ok(())
    }

    async fn recent_blockhash(&self) -> Result<[u8; 32], String> {
        if let Some((hash, fetched)) = *self.lock_blockhash()
            && fetched.elapsed() < BLOCKHASH_MAX_AGE
        {
            return Ok(hash);
        }
        let hash = latest_blockhash(&self.chain).await?;
        *self.lock_blockhash() = Some((hash, Instant::now()));
        Ok(hash)
    }

    fn lock_blockhash(&self) -> std::sync::MutexGuard<'_, Option<([u8; 32], Instant)>> {
        // Guards a plain value; a poisoned lock holds nothing half-written.
        self.blockhash
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn signature_base58(transaction: &[u8]) -> String {
    transaction_signature(transaction)
        .map(|signature| bs58::encode(signature).into_string())
        .unwrap_or_default()
}

impl SessionStatus {
    pub fn pubkey_base58(&self) -> String {
        base58(&self.pubkey)
    }

    pub fn sender_vault_base58(&self) -> String {
        base58(&self.sender_vault)
    }
}
