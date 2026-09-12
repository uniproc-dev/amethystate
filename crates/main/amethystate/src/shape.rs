//! What a declared field's type is, answered by the compiler.
//!
//! A declared path contributes its role and whether it may hold nothing to the
//! store's record of the shape. Both are facts about a type, so the type system
//! is where they are read from: an alias and a renamed import answer the same
//! as the type they name.
//!
//! [`Probe`] answers for **every** type, including types from crates this one
//! has never heard of. The types this crate provides get an inherent impl; an
//! inherent associated const shadows a trait's, so every other type falls
//! through to [`AnyShape`] and is described as one opaque value. Nothing is
//! asked of the type itself, so a leaf may be foreign.

use crate::migration::fields::Role;
use crate::reactive::map::ReactiveMap;
use std::marker::PhantomData;

/// Asks the compiler what `T` is. See the [module docs](self).
pub struct Probe<T: ?Sized>(PhantomData<T>);

/// What a type this crate knows nothing about is taken to be: one value, always
/// present.
///
/// Bring it into scope where a [`Probe`] is read - the fallback is a trait
/// const, so it resolves only when the trait is in scope, while the inherent
/// answers do not need it.
pub trait AnyShape {
    /// What the store does with the path this type is declared at.
    const ROLE: Role = Role::Field;

    /// Whether the path may hold nothing while still being a path.
    const OPTIONAL: bool = false;
}

impl<T: ?Sized> AnyShape for Probe<T> {}

impl<K, V> Probe<ReactiveMap<K, V>> {
    pub const ROLE: Role = Role::Map;
    pub const OPTIONAL: bool = false;
}

impl<T> Probe<Option<T>> {
    pub const ROLE: Role = Role::Field;
    pub const OPTIONAL: bool = true;
}
