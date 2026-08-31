//! Persistent reactive state for Rust GUI apps.

#![allow(clippy::complexity)]
#![deny(rustdoc::broken_intra_doc_links)]
mod codec;
mod global;
mod macros;

pub mod migration;
pub mod observability;
pub mod reactive;
pub mod shape;
pub mod store;

pub type AmeData<T> = <T as AmeState>::Data;
pub type MigrationResult<T> = StorageResult<T>;

pub use error_stack;
pub use inventory;
pub use serde;
pub use uuid;

pub use reactive::{
    AmeState, AmeStateNode, Change, Field, InterceptDisposer, MapChange, ReactiveCell, ReactiveMap,
    ReactiveMapKey, ReactiveMapValue, ReactiveScope, SignalSubscription,
};
pub use store::StoreSubscription;

pub mod errors {
    pub use crate::codec::CodecError;
    pub use crate::reactive::error::{FieldError, ReactiveMapError, WriteError, WriteResult};
    pub use crate::store::StorageError;
    pub use amethystate_core::facts;
    pub use error_stack::Report;
}
pub mod stores {
    pub use crate::store::default::*;
}

pub use store::{
    AmeStateSlice, StateScope, StorageResult, StoreEvent, StoreOp, SubscriptionKind,
    builder::StoreBuilder, config::StoreConfig, default::Store,
};

pub use migration::{MigrationContext, MigrationError, MigrationPlan, MigrationReport};

pub use amethystate_macros::{AmeType, amethystate, migrate};
pub use global::*;

#[cfg(any(feature = "tauri", feature = "json"))]
pub use serde_json;
pub use store::StoreBackend;
pub use store::StoreExt;

#[cfg(any(feature = "test-utils", test))]
pub mod test_utils;

#[cfg(feature = "tauri")]
pub mod tauri {
    pub use amethystate_core::scheme::*;
    pub use amethystate_tauri::*;
}

pub mod core {
    pub use amethystate_core::*;
}

#[cfg(any(feature = "async", feature = "tauri"))]
pub mod client {
    pub use amethystate_core::AmeBackendAsync;
    pub use amethystate_core::AmeStateSliceAsync;
    pub use amethystate_core::async_impl::*;

    use amethystate_core::async_impl::Field as CoreField;
    use amethystate_core::async_impl::ReactiveMap as CoreReactiveMap;
    #[cfg(feature = "tauri")]
    pub type ReactiveMap<K, V, B = crate::tauri::TauriBackend> = CoreReactiveMap<K, V, B>;

    #[cfg(all(feature = "async", not(feature = "tauri")))]
    pub type ReactiveMap<K, V, B> = CoreReactiveMap<K, V, B>;

    #[cfg(feature = "tauri")]
    pub type Field<V, B = crate::tauri::TauriBackend> = CoreField<V, B>;

    #[cfg(all(feature = "async", not(feature = "tauri")))]
    pub type Field<V, B> = CoreField<V, B>;
}
