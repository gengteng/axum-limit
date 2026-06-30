use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};

/// Errors that occur while encoding or decoding policy state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The serialized payload was invalid.
    InvalidPayload(String),
}

impl Display for CodecError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            CodecError::InvalidPayload(message) => write!(f, "invalid policy payload: {message}"),
        }
    }
}

impl Error for CodecError {}

pub(crate) fn encode_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec(value).map_err(|error| CodecError::InvalidPayload(error.to_string()))
}

pub(crate) fn decode_json<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, CodecError> {
    serde_json::from_slice(bytes).map_err(|error| CodecError::InvalidPayload(error.to_string()))
}
