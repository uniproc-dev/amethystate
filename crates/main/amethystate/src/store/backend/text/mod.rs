pub mod document;
pub mod error;
mod inspector;
#[cfg(feature = "json")]
pub mod json;
mod layout;
pub mod migration;
#[cfg(feature = "ron")]
pub mod ron;
pub mod store;
#[cfg(feature = "toml")]
pub mod toml;
pub mod tree;
mod watching;

pub use document::TextDocument;
pub use error::TextStoreError;
pub use store::TextStore;

#[cfg(feature = "json")]
pub use json::JsonStore;

#[cfg(feature = "toml")]
pub use toml::TomlStore;

#[cfg(feature = "ron")]
pub use ron::RonStore;
