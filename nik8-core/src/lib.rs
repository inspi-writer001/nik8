//! Platform-independent Nik8 client logic. No FFI here; `nik8-ffi` wraps it for the app.

pub mod audio;
pub mod ingress;
pub mod jitter;
pub mod listener;
pub mod rpc;
pub mod sender;
pub mod talk;
pub mod tx;

pub use listener::*;
pub use nik8_consumer::{ConsumerConfig, SubscriptionError};
pub use sender::{Sender, SessionStatus, SubmitPath};
pub use talk::{Talk, TalkEvent, TalkEventSink};

use std::str::FromStr;

use solana_address::Address;

/// Decodes a base58 Solana address.
pub fn parse_address(address: &str) -> Option<[u8; 32]> {
    Address::from_str(address.trim())
        .ok()
        .map(|address| address.to_bytes())
}
