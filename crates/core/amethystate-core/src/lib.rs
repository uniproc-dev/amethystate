#![allow(clippy::complexity)]
#![deny(rustdoc::broken_intra_doc_links)]
pub mod backend;
pub mod change;
pub mod facts;
pub mod failure;

#[cfg(feature = "async")]
pub mod async_impl;
pub mod path;
pub mod primitives;
pub mod scheme;
mod state;

#[cfg(feature = "test-utils")]
pub mod test_utils;

#[cfg(any(test, feature = "proptest-strategies"))]
pub mod strategies;

#[cfg(feature = "async")]
pub use async_impl::*;

#[cfg(feature = "async")]
pub use primitives::field_ops_async::*;

#[cfg(feature = "async")]
pub use primitives::map_ops_async::*;

pub use backend::*;
pub use primitives::*;
pub use scheme::*;
#[cfg(feature = "async")]
pub use state::*;

pub use change::{Change, MapChange, Source};
pub use primitives::field_core::FieldCore;
pub use primitives::field_ops::*;
pub use primitives::intercept::{InterceptDisposer, InterceptGuard};
pub use primitives::map_core::{Entries, ReactiveMapCore, Walk};
pub use primitives::map_ops::*;
pub use primitives::signal::{ReactiveScope, Signal, SignalSubscription};
