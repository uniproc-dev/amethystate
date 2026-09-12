//! What an ordinary program needs, in one import.
//!
//! ```
//! use amethystate::prelude::*;
//! ```
//!
//! Declaring state, opening a store, reading and writing through it, and the
//! two rules a declaration can carry. Everything here is re-exported from
//! where it lives, so naming a path directly stays as good as it was.
//!
//! Two of these are here because leaving them out is the mistake people make:
//! [`StoreExt`] and [`StoreBackend`] are traits, and without them in scope a
//! store has no `get`, no `set` and no `save_now` - which reads as the method
//! being missing rather than the trait.
//!
//! What is not here is what a program reaches for once and by name: the shape
//! of a subscription, the schema a migration compares, the layout of the files
//! on disk. A prelude that carried those would be a second crate root.

/// Turning a struct into stored state, and moving what an older build wrote.
pub use crate::{AmeData, amethystate, migrate};

/// Opening a store, and reaching the one this process installed.
pub use crate::store::builder::{Backend, StoreBuilder};
pub use crate::store::default::Store;
pub use crate::{IntoGlobalStore, global_store, init_global};

/// Reading and writing through it. Both are traits, and both are why a method
/// looks missing when they are not in scope.
pub use crate::store::{StoreBackend, StoreExt};

/// What a declared field holds, and what it does when the store disagrees.
pub use crate::reactive::{Change, Field, MapChange, ReactiveMap, ReactiveScope};
pub use crate::store::{OnDelete, OnUnreadable};

/// A struct that can be loaded whole, and where anything is stored.
pub use crate::store::AmeStateSlice;
pub use amethystate_core::path::StorePath;

/// What comes back when something goes wrong, and what carries it.
pub use crate::errors::Report;
pub use crate::store::{StorageError, StorageResult};

/// What a `#[migrate]` step returns, and what one built by hand is handed.
pub use crate::MigrationResult;
pub use crate::migration::MigrationContext;
