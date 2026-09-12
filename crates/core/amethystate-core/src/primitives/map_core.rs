use crate::SignalSubscription;
use crate::change::MapChange;
use crate::path::StorePath;
use crate::primitives::intercept::{InterceptDisposer, InterceptGuard};
use crate::primitives::signal::{SubscriptionMeta, forget, held, label};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use rpds::RedBlackTreeMapSync;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, SmolStrBuilder};
use std::any;
use std::borrow::Borrow;
use std::fmt::{self, Debug, Display, Write as _};
use std::hash::Hash;
use std::panic::Location;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Where a map's entries live, relative to the map itself.
pub trait MapEntryPath {
    /// The path of the entry `name` sits at.
    fn entry(&self, name: &str) -> StorePath;
}

impl MapEntryPath for StorePath {
    fn entry(&self, name: &str) -> StorePath {
        self.push(name)
    }
}

pub type InterceptorAny<K, V> =
    Arc<dyn Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static>;
pub type InterceptorKey<K, V> =
    Arc<dyn Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static>;
pub type SubscriberAny<K, V> = Arc<dyn Fn(&MapChange<K, V>) + Send + Sync + 'static>;
pub type SubscriberKey<K, V> = Arc<dyn Fn(&MapChange<K, V>) + Send + Sync + 'static>;

/// What a map may be keyed by: a key that already *is* the name its entry sits
/// at.
///
/// An entry lives at one level under the map, and that level is named by the
/// key. A key that borrows its own name needs no spelling and no parsing: the
/// store is addressed with what the caller already holds, a listing hands back
/// what is already there, and neither can disagree with the other, because
/// there is only one of them.
///
/// A key that is not a string is spelled by [`Id`], which renders it once and
/// holds the rendering.
pub trait ReactiveMapKey: AsRef<str> + Clone + Hash + Eq + Send + Sync + 'static {
    /// The key a stored name stands for, or `None` where that name is not one.
    ///
    /// The only direction that can fail, and the only one that allocates. It is
    /// walked once per entry when a map is built from the store, and never on a
    /// read.
    fn read(name: &str) -> Option<Self>;
}

impl ReactiveMapKey for String {
    fn read(name: &str) -> Option<Self> {
        Some(name.to_string())
    }
}

impl ReactiveMapKey for SmolStr {
    fn read(name: &str) -> Option<Self> {
        Some(SmolStr::new(name))
    }
}

impl<T> ReactiveMapKey for Id<T>
where
    T: Display + FromStr + Clone + Send + Sync + 'static,
{
    fn read(name: &str) -> Option<Self> {
        T::from_str(name).ok().map(Id::new)
    }
}

/// A key that is not a string, spelled once and kept that way.
///
/// `Id::new(9u16)` names its entry `9`, `Id::new(some_uuid)` names it by the
/// uuid's own spelling. The rendering is done when the `Id` is built rather
/// than at every lookup, which is what a `Uuid` wants most: 36 characters do
/// not fit beside the value, so spelling one costs the heap, and this pays it
/// once for the life of the key instead of once per read.
///
/// Entries are listed in the order of the *spelling*, so `Id<u16>` lists `10`
/// before `9`. A number's decimal spelling does not order like the number, and
/// nothing here pretends otherwise - padding it would make the name in the file
/// something nobody typed.
#[derive(Clone)]
pub struct Id<T> {
    spelled: SmolStr,
    value: T,
}

impl<T: Display> Id<T> {
    pub fn new(value: T) -> Self {
        let mut out = SmolStrBuilder::new();
        let _ = write!(out, "{value}");

        Self {
            spelled: out.finish(),
            value,
        }
    }
}

impl<T> Id<T> {
    /// What was spelled, for a caller that wants the number back.
    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T> AsRef<str> for Id<T> {
    fn as_ref(&self) -> &str {
        &self.spelled
    }
}

/// So a map keyed by an id can be looked up by the spelling alone, without
/// building one. Sound because `Eq` and `Hash` below answer from the same
/// spelling, which is what `Borrow` asks.
impl<T> Borrow<str> for Id<T> {
    fn borrow(&self) -> &str {
        &self.spelled
    }
}

/// By the spelling, which is what the store is addressed by and what a listing
/// is ordered by. Two ids that spell alike are one entry whatever they hold.
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.spelled == other.spelled
    }
}

impl<T> Eq for Id<T> {}

impl<T> Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.spelled.hash(state);
    }
}

impl<T> Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.spelled)
    }
}

impl<T: Debug> Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Id").field(&self.value).finish()
    }
}

/// As the spelling, so a snapshot holding one for a key is a document with
/// string keys - which is what every format this library writes needs a map's
/// keys to be.
impl<T> Serialize for Id<T> {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&self.spelled)
    }
}

impl<'de, T: Display + FromStr> Deserialize<'de> for Id<T> {
    fn deserialize<D: serde::Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        let spelled = <std::borrow::Cow<'de, str>>::deserialize(from)?;

        T::from_str(&spelled).map(Id::new).map_err(|_| {
            serde::de::Error::custom(format!("{spelled:?} is not a {}", any::type_name::<T>()))
        })
    }
}

pub trait ReactiveMapValue:
    Serialize + DeserializeOwned + Clone + Send + Sync + 'static + Default
{
}
impl<T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static + Default> ReactiveMapValue
    for T
{
}

/// A map's entries, held in the order the store lists them.
///
/// Keyed by the key itself, ordered by the name it borrows. A store orders its
/// keys by their levels, and an entry is one level, so the name's own order is
/// the store's - the two cannot come apart, because the cache is not holding a
/// second thing to compare.
///
/// A read takes a version and holds nothing, so a walk neither blocks a writer
/// nor waits for one, whatever thread either is on. A write publishes a new
/// version that shares every node it did not touch, so it stays O(log n)
/// rather than copying the map.
///
/// It is a trade, not a free win. Writes and memory both cost more than a
/// `RwLock<BTreeMap>` would, and
/// `crates/main/amethystate/benches/map_snapshot_bench.rs` measures against
/// one.
///
/// What it buys is that writing during a walk cannot deadlock. It does not
/// make it correct: the walk goes on yielding its own version, so a write made
/// inside the loop is invisible to the rest of it. `for k in keys { remove(k) }`
/// wants exactly that; a loop that writes and reads the same key back gets a
/// stale answer and nothing says so. Loud failure was traded for quiet
/// staleness.
///
/// The shape is not selectable per map, though it could be - the backing type
/// is private either way. Nothing would pick the locking one: it wins only
/// where a map is written in bulk and never walked while it is written, and a
/// map nobody observes is the store's job.
pub struct MapCache<K, V> {
    entries: ArcSwap<Snapshot<K, V>>,
}

impl<K: AsRef<str>, V> Default for MapCache<K, V> {
    fn default() -> Self {
        Self {
            entries: ArcSwap::from_pointee(RedBlackTreeMapSync::new_sync()),
        }
    }
}

/// A key held as the cache orders it: by the name it borrows, never by anything
/// the key type decides for itself.
///
/// The invariant the cache rests on is that its order is the store's. Requiring
/// `K: Ord` would ask the key type to agree and have no way to check; this
/// takes the ordering away from it instead, so a key whose own `Ord` says
/// something else cannot make a listing disagree with a scan.
#[derive(Clone)]
struct ByName<K>(K);

impl<K: AsRef<str>> Borrow<str> for ByName<K> {
    fn borrow(&self) -> &str {
        self.0.as_ref()
    }
}

impl<K: AsRef<str>> PartialEq for ByName<K> {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ref() == other.0.as_ref()
    }
}

impl<K: AsRef<str>> Eq for ByName<K> {}

impl<K: AsRef<str>> Ord for ByName<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_ref().cmp(other.0.as_ref())
    }
}

impl<K: AsRef<str>> PartialOrd for ByName<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: AsRef<str> + Clone, V: Clone> MapCache<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<V> {
        self.entries.load().get(name).cloned()
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.entries.load().contains_key(name)
    }

    /// The key as the map holds it, for a caller that looked one up by the name
    /// it borrows and needs the owned form back.
    pub fn owned_key(&self, name: &str) -> Option<K> {
        self.entries
            .load()
            .get_key_value(name)
            .map(|(key, _)| key.0.clone())
    }

    pub fn len(&self) -> usize {
        self.entries.load().size()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.load().is_empty()
    }

    pub fn clear(&self) {
        self.entries
            .store(Arc::new(RedBlackTreeMapSync::new_sync()));
    }

    /// The entries themselves, in order, without copying any of them.
    ///
    /// The version is taken when the view is, so writes that land afterwards
    /// are not in it however long it is held.
    pub fn view(&self) -> Entries<K, V> {
        Entries {
            held: self.entries.load_full(),
        }
    }

    /// Every key, in the order the contract promises, one at a time.
    pub fn keys(&self) -> Walk<K, V, K> {
        Walk::new(self.entries.load_full(), |key, _| key.0.clone())
    }

    /// Every entry, in that order, one at a time.
    pub fn entries(&self) -> Walk<K, V, (K, V)> {
        Walk::new(self.entries.load_full(), |key, value| {
            (key.0.clone(), value.clone())
        })
    }
}

type Snapshot<K, V> = RedBlackTreeMapSync<ByName<K>, V>;
type Held<K, V> = Arc<Snapshot<K, V>>;
type Pairs<'a, K, V> = <&'a Snapshot<K, V> as IntoIterator>::IntoIter;
/// A borrowed walk of one version, handing back the key beside its value.
///
/// Its own type rather than a mapped iterator, so how the cache holds a key
/// stays inside this module.
pub struct Values<'a, K: AsRef<str>, V> {
    pairs: Pairs<'a, K, V>,
}

impl<'a, K: AsRef<str>, V> Iterator for Values<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.pairs.next().map(|(key, value)| (&key.0, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.pairs.size_hint()
    }
}

impl<K: AsRef<str>, V> DoubleEndedIterator for Values<'_, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.pairs.next_back().map(|(key, value)| (&key.0, value))
    }
}

/// A walk of a [`MapCache`], in order, taking nothing it is not asked for.
///
/// The position is the iterator's own, so a whole pass costs one descent and
/// `take(n)` costs `n`.
///
/// The walk owns the version it started on. Writing to the same map while it is
/// alive is allowed from any thread, the walk included, and the walk keeps
/// handing back what its own version holds.
pub struct Walk<K: AsRef<str> + 'static, V: 'static, T> {
    pairs: Pairs<'static, K, V>,
    take: fn(&ByName<K>, &V) -> T,
    _held: Held<K, V>,
}

impl<K: AsRef<str>, V, T> Walk<K, V, T> {
    fn new(held: Held<K, V>, take: fn(&ByName<K>, &V) -> T) -> Self {
        // SAFETY: two invariants, both of which this module has to keep.
        //
        // The map lives in the `Arc`'s allocation and `_held` owns a strong
        // reference to it, so it outlives every borrow taken here however the
        // walk is moved. That is what lets the type carry no lifetime.
        //
        // The `'static` never reaches a caller: `new` is private, and `take` is
        // `for<'x> fn(&'x ByName<K>, &'x V) -> T`, which cannot return either
        // argument. So no `T` can be a reference into the map, and the only
        // `&'static` that exists is the temporary inside `next` and `next_back`.
        let pairs =
            unsafe { std::mem::transmute::<Pairs<'_, K, V>, Pairs<'static, K, V>>(held.iter()) };

        Self {
            pairs,
            take,
            _held: held,
        }
    }
}

impl<K: AsRef<str>, V, T> Iterator for Walk<K, V, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.pairs
            .next()
            .map(|(key, value)| (self.take)(key, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.pairs.size_hint()
    }
}

impl<K: AsRef<str>, V, T> DoubleEndedIterator for Walk<K, V, T> {
    fn next_back(&mut self) -> Option<T> {
        self.pairs
            .next_back()
            .map(|(key, value)| (self.take)(key, value))
    }
}

impl<K: AsRef<str>, V, T> ExactSizeIterator for Walk<K, V, T> {}

/// One version of a [`MapCache`], in order, borrowed rather than copied.
pub struct Entries<K, V> {
    held: Held<K, V>,
}

impl<'e, K: AsRef<str>, V> IntoIterator for &'e Entries<K, V> {
    type Item = (&'e K, &'e V);
    type IntoIter = Values<'e, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        Values {
            pairs: self.held.iter(),
        }
    }
}

impl<K: AsRef<str>, V> Entries<K, V> {
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&K, &V)> {
        self.held.iter().map(|(key, value)| (&key.0, value))
    }

    pub fn len(&self) -> usize {
        self.held.size()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

impl<K: AsRef<str> + Clone, V: Clone> MapCache<K, V> {
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        let replaced = self
            .entries
            .rcu(|current| current.insert(ByName(key.clone()), value.clone()));

        replaced.get(key.as_ref()).cloned()
    }

    pub fn remove(&self, name: &str) -> Option<(K, V)> {
        let previous = self.entries.rcu(|current| current.remove(name));

        previous
            .get_key_value(name)
            .map(|(key, value)| (key.0.clone(), value.clone()))
    }
}

impl<K: AsRef<str> + Debug, V: Debug> Debug for MapCache<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = self.entries.load();

        f.debug_map()
            .entries(held.iter().map(|(key, value)| (&key.0, value)))
            .finish()
    }
}

pub struct ReactiveMapCore<K, V> {
    pub interceptors_any: Arc<Mutex<Vec<(u64, InterceptorAny<K, V>)>>>,
    pub interceptors_key: Arc<DashMap<K, Vec<(u64, InterceptorKey<K, V>)>>>,
    pub subscribers_any: Arc<Mutex<Vec<(u64, SubscriberAny<K, V>, SubscriptionMeta)>>>,
    pub subscribers_key: Arc<DashMap<K, Vec<(u64, SubscriberKey<K, V>, SubscriptionMeta)>>>,
    pub next_id: Arc<AtomicU64>,
    pub intercept_depth: Arc<AtomicUsize>,
    pub cache: Arc<MapCache<K, V>>,
}

impl<K, V> Clone for ReactiveMapCore<K, V> {
    fn clone(&self) -> Self {
        Self {
            interceptors_any: self.interceptors_any.clone(),
            interceptors_key: self.interceptors_key.clone(),
            subscribers_any: self.subscribers_any.clone(),
            subscribers_key: self.subscribers_key.clone(),
            next_id: self.next_id.clone(),
            intercept_depth: self.intercept_depth.clone(),
            cache: self.cache.clone(),
        }
    }
}

struct Counted<'a, T>(&'a Mutex<Vec<T>>);

impl<T> Debug for Counted<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.try_lock() {
            Ok(list) => write!(f, "{}", list.len()),
            Err(_) => f.write_str("<locked>"),
        }
    }
}

impl<K: AsRef<str> + Debug + Hash + Eq, V: Debug> Debug for ReactiveMapCore<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReactiveMapCore")
            .field("cache", &self.cache)
            .field("interceptors_any", &Counted(&self.interceptors_any))
            .field("subscribers_any", &Counted(&self.subscribers_any))
            .finish()
    }
}

impl<K: ReactiveMapKey, V: ReactiveMapValue> Default for ReactiveMapCore<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: ReactiveMapKey, V: ReactiveMapValue> ReactiveMapCore<K, V> {
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// The same. A tree has nothing to reserve, so `entries` is ignored; the
    /// signature stays because a caller loading a store knows the size and
    /// should not have to know that.
    pub fn with_capacity(_entries: usize) -> Self {
        Self {
            interceptors_any: Arc::new(Mutex::new(Vec::new())),
            interceptors_key: Arc::new(DashMap::new()),
            subscribers_any: Arc::new(Mutex::new(Vec::new())),
            subscribers_key: Arc::new(DashMap::new()),
            next_id: Arc::new(AtomicU64::new(0)),
            intercept_depth: Arc::new(AtomicUsize::new(0)),
            cache: Arc::new(MapCache::new()),
        }
    }

    #[track_caller]
    pub fn subscribe_any<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        let location = Location::caller();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let meta = SubscriptionMeta {
            id,
            location,
            name: None,
        };
        held(&self.subscribers_any).push((id, Arc::new(callback), meta));

        let subs_for_name = self.subscribers_any.clone();
        let set_name = Arc::new(move |name: &'static str| {
            label(&mut held(&subs_for_name), id, name);
        });
        let subs_for_cleanup = self.subscribers_any.clone();
        SignalSubscription::new(
            id,
            location,
            set_name,
            Arc::new(move |id| {
                forget(&mut held(&subs_for_cleanup), id);
            }),
        )
    }

    #[track_caller]
    pub fn subscribe_key<F>(&self, key: K, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        let location = Location::caller();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let meta = SubscriptionMeta {
            id,
            location,
            name: None,
        };
        self.subscribers_key
            .entry(key.clone())
            .or_default()
            .push((id, Arc::new(callback), meta));

        let subs_for_name = self.subscribers_key.clone();
        let key_for_name = key.clone();
        let set_name = Arc::new(move |name: &'static str| {
            if let Some(mut list) = subs_for_name.get_mut(&key_for_name) {
                label(&mut list, id, name);
            }
        });
        let subs_for_cleanup = self.subscribers_key.clone();
        SignalSubscription::new(
            id,
            location,
            set_name,
            Arc::new(move |id| {
                if let Some(mut list) = subs_for_cleanup.get_mut(&key) {
                    forget(&mut list, id);
                }
                subs_for_cleanup.remove_if(&key, |_, list| list.is_empty());
            }),
        )
    }

    pub fn intercept<F>(&self, path: StorePath, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        held(&self.interceptors_any).push((id, Arc::new(callback)));
        let subs = self.interceptors_any.clone();
        InterceptDisposer {
            id,
            path,
            cleanup: Arc::new(move |id| {
                held(&subs).retain(|(i, _)| *i != id);
            }),
        }
    }

    pub fn intercept_key<F>(&self, key: K, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.interceptors_key
            .entry(key.clone())
            .or_default()
            .push((id, Arc::new(callback)));
        let subs = self.interceptors_key.clone();
        InterceptDisposer {
            id,
            path: StorePath::root(),
            cleanup: Arc::new(move |id| {
                if let Some(mut list) = subs.get_mut(&key) {
                    list.retain(|(i, _)| *i != id);
                }
                subs.remove_if(&key, |_, list| list.is_empty());
            }),
        }
    }

    pub fn run_interceptors(
        &self,
        path: StorePath,
        mut change: MapChange<K, V>,
    ) -> Result<MapChange<K, V>, String> {
        let Some(_guard) = InterceptGuard::enter(&self.intercept_depth, path) else {
            return Err("interceptors nested too deep".to_string());
        };

        if let Some(key) = change.key().cloned() {
            let interceptors = self
                .interceptors_key
                .get(&key)
                .map(|entry| entry.clone())
                .unwrap_or_default();
            for (_, interceptor) in interceptors {
                if let Some(new_change) = interceptor(change.clone()) {
                    change = new_change;
                } else {
                    return Err("refused by an interceptor on that key".to_string());
                }
            }
        }

        let interceptors_any = held(&self.interceptors_any).clone();
        for (_, interceptor) in interceptors_any {
            if let Some(new_change) = interceptor(change.clone()) {
                change = new_change;
            } else {
                return Err("refused by an interceptor on the map".to_string());
            }
        }

        Ok(change)
    }

    /// Fires every subscriber interested in `change`.
    ///
    /// The callbacks are collected before any of them runs, and every guard is
    /// released first. A subscriber reacting to a change by writing to the same
    /// map is ordinary, and neither a `Mutex` nor a `DashMap` shard is
    /// reentrant, so holding either across the calls deadlocks the thread.
    pub fn notify(&self, change: &MapChange<K, V>) {
        let keyed: Vec<_> = match change.key() {
            Some(k) => self
                .subscribers_key
                .get(k)
                .map(|entries| {
                    entries
                        .iter()
                        .map(|(_, cb, meta)| (cb.clone(), *meta))
                        .collect()
                })
                .unwrap_or_default(),

            // A clear is about every key at once, so everyone watching a key
            // hears it. Told about nothing, an entry cell goes on reporting a
            // value the map no longer has, and has no later change to correct
            // it.
            None => self
                .subscribers_key
                .iter()
                .flat_map(|entries| {
                    entries
                        .value()
                        .iter()
                        .map(|(_, cb, meta)| (cb.clone(), *meta))
                        .collect::<Vec<_>>()
                })
                .collect(),
        };

        let any: Vec<_> = held(&self.subscribers_any)
            .iter()
            .map(|(_, cb, meta)| (cb.clone(), *meta))
            .collect();

        for (cb, meta) in keyed {
            tracing::trace!(
                target: "amethystate",
                subscription_id = meta.id,
                name = meta.name,
                location = format!("{}:{}", meta.location.file(), meta.location.line()),
                "map signal emit → key subscription fire",
            );
            cb(change);
        }

        for (cb, meta) in any {
            tracing::trace!(
                target: "amethystate",
                subscription_id = meta.id,
                name = meta.name,
                location = format!("{}:{}", meta.location.file(), meta.location.line()),
                "map signal emit → any subscription fire",
            );
            cb(change);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_lists_entries_in_the_order_the_store_keys_them() {
        let cache = MapCache::<String, u8>::new();
        let names = ["a1b", "a.b", "10", "9", "100", "a\\b", "cpu", "Cpu"];

        for (n, name) in names.iter().enumerate() {
            cache.insert((*name).to_string(), n as u8);
        }

        let under = StorePath::segment("m");
        let mut by_store = names.to_vec();
        by_store.sort_by_key(|name| under.push(name).key().as_bytes().to_vec());

        assert_eq!(cache.keys().collect::<Vec<_>>(), by_store);
    }

    #[test]
    fn a_name_holding_the_separator_is_one_entry() {
        let cache = MapCache::<String, u8>::new();

        cache.insert("dark.mode".to_string(), 1);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("dark.mode"), Some(1));
        assert_eq!(cache.get("dark"), None);
        assert_eq!(cache.get("dark\\.mode"), None);
        assert_eq!(cache.keys().collect::<Vec<_>>(), ["dark.mode"]);
    }

    #[test]
    fn a_dropped_key_subscription_leaves_no_entry_behind() {
        let core = ReactiveMapCore::<String, u64>::new();

        for i in 0..64u64 {
            let key = format!("col{i}");
            let sub = core.subscribe_key(key.clone(), |_| {});
            let held = core.intercept_key(key, Some);
            drop(sub);
            held.remove();
        }

        assert_eq!(core.subscribers_key.len(), 0);
        assert_eq!(core.interceptors_key.len(), 0);
    }

    #[test]
    fn a_key_still_watched_by_someone_keeps_its_entry() {
        let core = ReactiveMapCore::<String, u64>::new();

        let first = core.subscribe_key("cpu".to_string(), |_| {});
        let second = core.subscribe_key("cpu".to_string(), |_| {});

        drop(first);
        assert_eq!(core.subscribers_key.len(), 1);

        drop(second);
        assert_eq!(core.subscribers_key.len(), 0);
    }
}
