//! Reactive primitives: values that persist and notify.
//!
//! # Subscribers run ahead of the disk
//!
//! Every primitive here shares one ordering. A write lands in the store's
//! buffer, subscribers are notified, and only later does the debouncer commit
//! it to disk. So a callback always sees the new value while it is still
//! buffered, and a crash before the commit takes back something a subscriber
//! already acted on - a request it sent, a file it wrote, a screen it drew.
//!
//! This is deliberate: a UI that repainted only after `fsync` would be
//! unusable. But it means a subscription is not a durability signal. When an
//! effect must not outlive the data that caused it, write through the
//! `durable()` form and act after it returns, rather than from the callback.
//!
//! The same holds for interceptors, which run earlier still - before the value
//! reaches the buffer at all.
//!
//! The book works through the whole picture in
//! [Durability](https://uniproc-dev.github.io/amethystate/concepts/durability).

pub mod cell;
pub mod entry_cell;
pub mod error;
pub mod field;
pub mod map;
pub mod watch;

pub use crate::migration::node::*;
pub use amethystate_core::change::*;
pub use amethystate_core::primitives::intercept::*;
pub use amethystate_core::primitives::map_core::{
    Entries, InterceptorAny, InterceptorKey, SubscriberAny, SubscriberKey, Walk,
};
pub use amethystate_core::primitives::signal::{
    ReactiveScope, SignalSubscription, SubscriptionMeta,
};
pub use cell::*;
pub use field::*;
pub use map::*;
pub use watch::*;
