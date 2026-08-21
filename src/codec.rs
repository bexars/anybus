//! Shared bincode helpers for remote payloads and websocket frames.
//! Uses the same config as async-bincode 0.8: `standard()` with a u32 size limit.

fn config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<{ u32::MAX as usize }>()
}

pub(crate) fn encode<T: serde::Serialize>(
    value: &T,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::serde::encode_to_vec(value, config())
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, bincode::error::DecodeError> {
    bincode::serde::decode_from_slice(bytes, config()).map(|(value, _)| value)
}
