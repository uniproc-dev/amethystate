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

use crate::migration::StepResult;
use crate::migration::fields::Role;
use crate::observability::Disagreement;
use crate::reactive::FieldValue;
use crate::reactive::map::ReactiveMap;
use crate::store::{
    Check, OnDelete, OnUnreadable, OpenStruct, ReadRules, StoredAs, UnreadableEntries, WriteValue,
};
use crate::{
    Field, MigrationContext, ReactiveMapKey, ReactiveMapValue, SignalSubscription, Store, StoreExt,
};
use amethystate_core::path::StorePath;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use uuid::Uuid;

/// Asks the compiler what `T` is. See the [module docs](self).
///
/// `doc(hidden)`: the generated code reads it and nothing else has a use for
/// it. Public because an expansion lands in the caller's crate, not because a
/// caller is meant to write it.
#[doc(hidden)]
pub struct Probe<T: ?Sized>(PhantomData<T>);

/// What a type this crate knows nothing about is taken to be: one value, always
/// present.
///
/// Bring it into scope where a [`Probe`] is read - the fallback is a trait
/// const, so it resolves only when the trait is in scope, while the inherent
/// answers do not need it.
///
/// `doc(hidden)` for [`Probe`]'s reason, and open on purpose beneath that: the
/// fallback has to be implementable through, or a type from a crate this one
/// never heard of could not be a leaf.
#[doc(hidden)]
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

/// What a declared field's type is held by, and how it moves in and out of the
/// store - answered by the compiler rather than by how the type was spelled.
///
/// A macro runs before types exist, so it cannot ask whether a field is a map.
/// The generated code asks this instead, and the compiler picks the answer: a
/// `ReactiveMap<K, V>` is its own handle, and every other value is held by a
/// [`Field`]. An alias names the same type and gets the same answer.
///
/// The two implementations do not overlap because `ReactiveMap` is not a
/// [`FieldValue`] - it does not implement `Serialize`. Giving it one would make
/// this crate stop compiling, which is the guard.
///
/// `doc(hidden)` for [`Probe`]'s reason.
#[doc(hidden)]
pub trait Kind: Sized + 'static {
    /// What the struct holds for the field.
    type Handle: Clone;

    /// What the field is in a plain snapshot of the struct.
    type Data;

    /// What a declared `default` builds.
    type Seed;

    /// What the field shows when a struct is looked at.
    type Peek;

    fn build(
        store: &Store,
        at: StorePath,
        seed: Self::Seed,
        instance_id: Uuid,
        rules: KindRules<Self>,
    ) -> Result<Self::Handle, OpenStruct>;

    fn load_step(
        ctx: &mut MigrationContext<'_>,
        at: &[&str],
        stored_as: StoredAs<Self>,
        seed: impl FnOnce() -> Self::Seed,
    ) -> StepResult<Self::Data>;

    fn save_step(
        data: &Self::Data,
        ctx: &mut MigrationContext<'_>,
        at: &[&str],
        stored_as: StoredAs<Self>,
    ) -> StepResult<()>;

    fn load_plain(
        store: &Store,
        at: &StorePath,
        stored_as: StoredAs<Self>,
        check: Option<Check<Self>>,
        policy: OnUnreadable,
        seed: impl FnOnce() -> Self::Seed,
    ) -> Result<Self::Data, OpenStruct>;

    fn save_plain(
        data: &Self::Data,
        store: &Store,
        at: &StorePath,
        stored_as: StoredAs<Self>,
    ) -> Result<(), WriteValue>;

    fn snapshot(handle: &Self::Handle) -> Self::Data;

    fn watch(
        handle: &Self::Handle,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> SignalSubscription;

    fn refused(handle: &Self::Handle, why: &str);

    fn peek(handle: &Self::Handle) -> Self::Peek;

    fn disagreement(handle: &Self::Handle) -> Option<Disagreement>;
}

/// What a field declared about reading, handed to [`Kind::build`] whole.
///
/// Each kind takes the part that means something to it: a value takes the
/// check, how it is stored and what an unreadable value does; a map takes what
/// an unreadable entry does. What a removed key does means something to both.
#[doc(hidden)]
pub struct KindRules<T> {
    pub on_unreadable: OnUnreadable,
    pub on_delete: OnDelete,
    pub unreadable_entries: UnreadableEntries,
    pub check: Option<Check<T>>,
    pub stored_as: StoredAs<T>,
}

/// How many entries a map holds, shown as that rather than as the entries.
#[doc(hidden)]
pub struct EntryCount(pub usize);

impl fmt::Debug for EntryCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} entries", self.0)
    }
}

impl<T: FieldValue> Kind for T {
    type Handle = Field<T>;
    type Data = T;
    type Seed = T;
    type Peek = T;

    fn build(
        store: &Store,
        at: StorePath,
        seed: T,
        instance_id: Uuid,
        rules: KindRules<T>,
    ) -> Result<Field<T>, OpenStruct> {
        let read = ReadRules::new()
            .on_unreadable(rules.on_unreadable)
            .on_delete(rules.on_delete)
            .stored_as(rules.stored_as);

        let read = match rules.check {
            Some(check) => read.check(check),
            None => read,
        };

        crate::store::field_with_path_under(store, at, seed, instance_id, read)
    }

    fn load_step(
        ctx: &mut MigrationContext<'_>,
        at: &[&str],
        stored_as: StoredAs<T>,
        seed: impl FnOnce() -> T,
    ) -> StepResult<T> {
        Ok(ctx.get_as::<T>(at, stored_as)?.unwrap_or_else(seed))
    }

    fn save_step(
        data: &T,
        ctx: &mut MigrationContext<'_>,
        at: &[&str],
        stored_as: StoredAs<T>,
    ) -> StepResult<()> {
        ctx.set_as(at, data, stored_as)
    }

    fn load_plain(
        store: &Store,
        at: &StorePath,
        stored_as: StoredAs<T>,
        check: Option<Check<T>>,
        policy: OnUnreadable,
        seed: impl FnOnce() -> T,
    ) -> Result<T, OpenStruct> {
        crate::store::load_declared(store, at, stored_as, check, policy, seed)
    }

    fn save_plain(
        data: &T,
        store: &Store,
        at: &StorePath,
        stored_as: StoredAs<T>,
    ) -> Result<(), WriteValue> {
        crate::store::save_declared(store, at, data, stored_as)
    }

    fn snapshot(handle: &Field<T>) -> T {
        handle.get()
    }

    fn watch(
        handle: &Field<T>,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> SignalSubscription {
        handle.subscribe(move |_| on_change())
    }

    fn refused(handle: &Field<T>, why: &str) {
        handle.__ame_refused(why);
    }

    fn peek(handle: &Field<T>) -> T {
        handle.get()
    }

    fn disagreement(handle: &Field<T>) -> Option<Disagreement> {
        handle.__ame_disagreement()
    }
}

impl<K, V> Kind for ReactiveMap<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue + Serialize + DeserializeOwned,
{
    type Handle = ReactiveMap<K, V>;
    type Data = IndexMap<K, V>;
    type Seed = HashMap<K, V>;
    type Peek = EntryCount;

    fn build(
        store: &Store,
        at: StorePath,
        seed: HashMap<K, V>,
        instance_id: Uuid,
        rules: KindRules<Self>,
    ) -> Result<Self, OpenStruct> {
        Ok(crate::store::reactive_map_where::<K, V>(
            store,
            at,
            seed,
            instance_id,
            rules.unreadable_entries,
            rules.on_delete,
        )?)
    }

    fn load_step(
        ctx: &mut MigrationContext<'_>,
        at: &[&str],
        _stored_as: StoredAs<Self>,
        _seed: impl FnOnce() -> HashMap<K, V>,
    ) -> StepResult<IndexMap<K, V>> {
        ctx.scan_map::<K, V>(at)
    }

    fn save_step(
        data: &IndexMap<K, V>,
        ctx: &mut MigrationContext<'_>,
        at: &[&str],
        _stored_as: StoredAs<Self>,
    ) -> StepResult<()> {
        ctx.delete_prefix(at)?;

        let mut entries = ctx.scoped(at);
        for (key, value) in data {
            entries.set(key.as_ref(), value)?;
        }
        Ok(())
    }

    fn load_plain(
        store: &Store,
        at: &StorePath,
        _stored_as: StoredAs<Self>,
        _check: Option<Check<Self>>,
        _policy: OnUnreadable,
        _seed: impl FnOnce() -> HashMap<K, V>,
    ) -> Result<IndexMap<K, V>, OpenStruct> {
        Ok(crate::store::load_map::<K, V>(store, at)?)
    }

    fn save_plain(
        data: &IndexMap<K, V>,
        store: &Store,
        at: &StorePath,
        _stored_as: StoredAs<Self>,
    ) -> Result<(), WriteValue> {
        let kept: std::collections::HashSet<&str> = data.keys().map(|key| key.as_ref()).collect();
        let stored = crate::store::StoreBackend::scan_keys(store, at)
            .map_err(|why| WriteValue::from_store(at, why))?;

        let gone: std::collections::BTreeSet<String> = stored
            .iter()
            .filter_map(|path| path.segments().nth(at.len()))
            .map(|name| name.as_str().to_string())
            .filter(|name| !kept.contains(name.as_str()))
            .collect();

        for name in gone {
            let entry = crate::store::entry_path(at, &name);
            crate::store::StoreBackend::delete_prefix(store, &entry)
                .map_err(|why| WriteValue::from_store(&entry, why))?;
        }

        for (key, value) in data {
            let entry = crate::store::entry_path(at, key.as_ref());
            <Store as StoreExt>::set(store, &entry, value)?;
        }
        Ok(())
    }

    fn snapshot(handle: &Self) -> IndexMap<K, V> {
        handle.entries().collect()
    }

    fn watch(handle: &Self, on_change: impl Fn() + Send + Sync + 'static) -> SignalSubscription {
        handle.subscribe_any(move |_| on_change())
    }

    fn refused(_handle: &Self, _why: &str) {}

    fn peek(handle: &Self) -> EntryCount {
        EntryCount(handle.len())
    }

    fn disagreement(_handle: &Self) -> Option<Disagreement> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handled_by<T: Kind>() -> PhantomData<T::Handle> {
        PhantomData
    }

    type Widths = ReactiveMap<String, u64>;

    #[test]
    fn a_value_is_held_by_a_field_and_a_map_by_itself() {
        let _: PhantomData<Field<u16>> = handled_by::<u16>();
        let _: PhantomData<Field<Option<String>>> = handled_by::<Option<String>>();
        let _: PhantomData<ReactiveMap<String, u64>> = handled_by::<ReactiveMap<String, u64>>();
    }

    #[test]
    fn an_alias_is_answered_for_the_type_it_names() {
        let _: PhantomData<ReactiveMap<String, u64>> = handled_by::<Widths>();
    }

    #[test]
    fn a_map_shows_how_many_entries_it_holds() {
        assert_eq!(format!("{:?}", EntryCount(3)), "3 entries");
    }
}
