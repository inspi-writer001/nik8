use std::env;

use solana_epoch_info::EpochInfo;
use solana_rpc_client_types::response::RpcContactInfo;

use crate::Nik8Error;

/// Cluster RPC for epoch info, cluster nodes and the leader schedule, from `RPC_URL`.
/// `main` loads `.env` before anything calls this.
pub fn rpc_url() -> String {
    env::var("RPC_URL").expect("RPC_URL must be set (cluster RPC endpoint)")
}

/// WebSocket for slot updates: `WS_URL`, or derived from `RPC_URL`
/// (`http` → `ws`, and a validator's RPC port 8899 → its PubSub port 8900).
pub fn ws_url() -> String {
    env::var("WS_URL").unwrap_or_else(|_| derive_ws_url(&rpc_url()))
}

fn derive_ws_url(rpc_url: &str) -> String {
    let ws = if let Some(rest) = rpc_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = rpc_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        rpc_url.to_owned()
    };
    ws.replacen(":8899", ":8900", 1)
}

/// Fetch the latest blockhash from the same validator whose TPU is used below.
///
/// This is a read-only RPC request. It does not simulate or submit a transaction.
pub async fn get_current_blockhash_from_validator()
-> Result<solana_hash::Hash, Box<dyn std::error::Error>> {
    use solana_rpc_client::nonblocking::rpc_client::RpcClient;

    let rpc_client = RpcClient::new(rpc_url());

    Ok(rpc_client.get_latest_blockhash().await?)
}

pub async fn get_epoch_info() -> Result<EpochInfo, Box<dyn std::error::Error>> {
    use solana_rpc_client::nonblocking::rpc_client::RpcClient;

    let rpc_client = RpcClient::new(rpc_url());

    Ok(rpc_client.get_epoch_info().await?)
}

pub async fn get_cluster_nodes() -> Result<Vec<RpcContactInfo>, Box<dyn std::error::Error>> {
    use solana_rpc_client::nonblocking::rpc_client::RpcClient;

    let rpc_client = RpcClient::new(rpc_url());

    Ok(rpc_client.get_cluster_nodes().await?)
}

pub async fn get_current_slot() -> Result<u64, Box<dyn std::error::Error>> {
    use solana_rpc_client::nonblocking::rpc_client::RpcClient;
    let rpc_client = RpcClient::new(rpc_url());
    let current_slot = rpc_client.get_slot().await?;
    Ok(current_slot)
}

pub async fn get_leader_schedule(
    slot: u64,
) -> Result<
    std::collections::HashMap<std::string::String, std::vec::Vec<usize>>,
    Box<dyn std::error::Error>,
> {
    use solana_rpc_client::nonblocking::rpc_client::RpcClient;
    let rpc_client = RpcClient::new(rpc_url());

    let leaders_schedule = rpc_client.get_leader_schedule(slot.into()).await;

    match leaders_schedule {
        Ok(optional_leader_schedule) => match optional_leader_schedule {
            Some(leader_schedule) => Ok(leader_schedule),
            None => Err(Nik8Error::FailedToGetLeaderSchedule("none".to_string()).into()),
        },
        Err(error) => Err(Nik8Error::FailedToGetLeaderSchedule(error.to_string()).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::derive_ws_url;

    #[test]
    fn derives_websocket_urls() {
        assert_eq!(
            derive_ws_url("https://devnet.helius-rpc.com/?api-key=x"),
            "wss://devnet.helius-rpc.com/?api-key=x"
        );
        assert_eq!(
            derive_ws_url("https://api.devnet.solana.com"),
            "wss://api.devnet.solana.com"
        );
        assert_eq!(
            derive_ws_url("http://74.50.83.238:8899"),
            "ws://74.50.83.238:8900"
        );
    }
}
