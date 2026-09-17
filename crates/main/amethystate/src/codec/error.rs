use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodecError {
    #[cfg(any(feature = "json", test))]
    #[error("JSON codec error: {0}")]
    Json(#[from] serde_json::Error),

    #[cfg(feature = "sqlite")]
    #[error("JSON codec error: {0}")]
    SonicJson(#[from] sonic_rs::Error),

    #[cfg(feature = "toml")]
    #[error("TOML error: {0}")]
    Toml(String),

    #[cfg(feature = "ron")]
    #[error("RON codec error: {0}")]
    Ron(#[from] ron::error::Error),

    #[cfg(any(feature = "redb", feature = "memory"))]
    #[error("MessagePack encode error: {0}")]
    MessagePackEncode(#[from] rmp_serde::encode::Error),

    #[cfg(any(feature = "redb", feature = "memory"))]
    #[error("MessagePack decode error: {0}")]
    MessagePackDecode(#[from] rmp_serde::decode::Error),

    #[error("Erased codec error: {0}")]
    Erased(#[from] erased_serde::Error),

    #[error("Codec error: {0}")]
    Custom(String),
}
