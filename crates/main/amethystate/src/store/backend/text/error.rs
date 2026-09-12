use crate::codec::CodecError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TextStoreError {
    #[error("Text store IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Codec(#[from] CodecError),

    #[error("Text root must be an object/mapping")]
    RootMustBeObject,

    #[error("Text path segment '{0}' not found")]
    PathSegmentMissing(String),

    #[error("File watcher error: {0}")]
    Watch(String),
}
