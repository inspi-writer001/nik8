#[derive(Debug, thiserror::Error)]
pub enum Nik8Error {
    #[error("Failed to Get leader Schedule: {0}")]
    FailedToGetLeaderSchedule(String),

    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Delivery timeout")]
    DeliveryTimeout,
    // ... more variants
}
