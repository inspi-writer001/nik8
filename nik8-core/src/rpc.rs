//! The few Solana JSON-RPC calls the app needs: submit, confirm, balance, account existence.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use solana_address::Address;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONFIRM_POLL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct RpcClient {
    http: reqwest::Client,
    url: String,
}

impl RpcClient {
    pub fn new(url: impl Into<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| format!("could not build HTTP client: {error}"))?;
        Ok(Self {
            http,
            url: url.into(),
        })
    }

    /// Submits a signed wire transaction and returns its signature (base58).
    pub async fn send_transaction(
        &self,
        transaction: &[u8],
        skip_preflight: bool,
    ) -> Result<String, String> {
        let result = self
            .call(
                "sendTransaction",
                json!([
                    STANDARD.encode(transaction),
                    {
                        "encoding": "base64",
                        "skipPreflight": skip_preflight,
                        "preflightCommitment": "confirmed",
                    }
                ]),
            )
            .await?;
        result
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("unexpected sendTransaction result: {result}"))
    }

    /// Waits until `signature` is `confirmed`, failing on a transaction error or timeout.
    pub async fn confirm(&self, signature: &str, timeout: Duration) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let result = self
                .call("getSignatureStatuses", json!([[signature]]))
                .await?;
            let status = &result["value"][0];
            if !status.is_null() {
                if !status["err"].is_null() {
                    return Err(format!("transaction failed: {}", status["err"]));
                }
                if matches!(
                    status["confirmationStatus"].as_str(),
                    Some("confirmed" | "finalized")
                ) {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("transaction {signature} not confirmed in time"));
            }
            tokio::time::sleep(CONFIRM_POLL).await;
        }
    }

    pub async fn balance(&self, address: &[u8; 32]) -> Result<u64, String> {
        let result = self
            .call(
                "getBalance",
                json!([base58(address), { "commitment": "confirmed" }]),
            )
            .await?;
        result["value"]
            .as_u64()
            .ok_or_else(|| format!("unexpected getBalance result: {result}"))
    }

    pub async fn account_exists(&self, address: &[u8; 32]) -> Result<bool, String> {
        let result = self
            .call(
                "getAccountInfo",
                json!([base58(address), { "commitment": "confirmed", "encoding": "base64" }]),
            )
            .await?;
        Ok(!result["value"].is_null())
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let response: Value = self
            .http
            .post(&self.url)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .map_err(|error| format!("{method} request failed: {error}"))?
            .json()
            .await
            .map_err(|error| format!("{method} returned invalid JSON: {error}"))?;

        if let Some(error) = response.get("error") {
            return Err(format!("{method} failed: {}", rpc_error_message(error)));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("{method} returned no result"))
    }
}

pub fn base58(address: &[u8; 32]) -> String {
    Address::new_from_array(*address).to_string()
}

/// Prefers the simulation logs' last line, which usually names the program error.
fn rpc_error_message(error: &Value) -> String {
    let message = error["message"].as_str().unwrap_or("unknown error");
    let last_log = error["data"]["logs"]
        .as_array()
        .and_then(|logs| logs.iter().rev().find_map(Value::as_str));
    match last_log {
        Some(log) => format!("{message} ({log})"),
        None => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_program_logs_in_errors() {
        let error = json!({
            "code": -32002,
            "message": "Transaction simulation failed",
            "data": { "logs": ["Program NiK8 invoke [1]", "Program NiK8 failed: custom program error: 0x1771"] }
        });
        assert_eq!(
            rpc_error_message(&error),
            "Transaction simulation failed (Program NiK8 failed: custom program error: 0x1771)"
        );
        assert_eq!(rpc_error_message(&json!({ "message": "boom" })), "boom");
    }
}
