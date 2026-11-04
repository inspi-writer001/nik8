use std::{env, error::Error, fmt};

use crate::yellowstone::NIK8_PROGRAM_ID_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumerConfig {
    pub grpc_endpoint: String,
    pub x_token: Option<String>,
    /// Program whose `OnewayMessage` instructions are consumed.
    pub program_id: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsumerConfigError {
    MissingGrpcEndpoint,
    EmptyGrpcEndpoint,
}

impl fmt::Display for ConsumerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingGrpcEndpoint => formatter.write_str("NIK8_GRPC_ENDPOINT must be set"),
            Self::EmptyGrpcEndpoint => formatter.write_str("NIK8_GRPC_ENDPOINT cannot be empty"),
        }
    }
}

impl Error for ConsumerConfigError {}

impl ConsumerConfig {
    /// Explicit configuration for embedders (mobile). Blank tokens are treated as absent.
    pub fn new(
        grpc_endpoint: impl Into<String>,
        x_token: Option<String>,
    ) -> Result<Self, ConsumerConfigError> {
        let grpc_endpoint = grpc_endpoint.into();
        if grpc_endpoint.trim().is_empty() {
            return Err(ConsumerConfigError::EmptyGrpcEndpoint);
        }

        Ok(Self {
            grpc_endpoint,
            x_token: x_token.filter(|token| !token.trim().is_empty()),
            program_id: NIK8_PROGRAM_ID_BYTES,
        })
    }

    pub fn with_program_id(mut self, program_id: [u8; 32]) -> Self {
        self.program_id = program_id;
        self
    }

    /// Desktop CLI configuration from `NIK8_GRPC_ENDPOINT` / `NIK8_GRPC_X_TOKEN` (and `.env`).
    pub fn from_env() -> Result<Self, ConsumerConfigError> {
        let _ = dotenvy::dotenv();

        let grpc_endpoint =
            env::var("NIK8_GRPC_ENDPOINT").map_err(|_| ConsumerConfigError::MissingGrpcEndpoint)?;

        Self::new(grpc_endpoint, env::var("NIK8_GRPC_X_TOKEN").ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_endpoint_and_drops_blank_token() {
        assert_eq!(
            ConsumerConfig::new("  ", None),
            Err(ConsumerConfigError::EmptyGrpcEndpoint)
        );

        let config = ConsumerConfig::new("https://example.com", Some(" ".into())).unwrap();
        assert_eq!(config.x_token, None);
        assert_eq!(config.program_id, NIK8_PROGRAM_ID_BYTES);
    }
}
