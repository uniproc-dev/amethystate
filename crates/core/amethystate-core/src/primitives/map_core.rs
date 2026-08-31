use crate::SignalSubscription;
use crate::change::MapChange;
use crate::facts::Facts;
use crate::path::{StorePath, escape_name};
use crate::primitives::error::{ReactiveMapResult, WriteError};
use crate::primitives::intercept::{InterceptDisposer, InterceptGuard};
use crate::primitives::signal::SubscriptionMeta;
use arc_swap::ArcSwap;
use dashmap::DashMap;
use error_stack::ResultExt;
use rpds::RedBlackTreeMapSync;
use smol_str::SmolStr;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::{self, Debug, Display};
use std::hash::Hash;
use std::panic::Location;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Where a map's entries live, relative to the map itself.
pub trait MapEntryPath {
    /// The path of the entry `key` names, or why that key cannot name one.
    fn entry(&self, key: &impl Display) -> ReactiveMapResult<StorePath>;
}

impl MapEntryPath for StorePath {
    fn entry(&self, key: &impl Display) -> ReactiveMapResult<StorePath> {
        let key = key.to_string();

        self.try_push(&key)
            .change_context(WriteError::Path)
            .attach_prefix(self)
            .attach_entry(&key)
    }
}

pub type InterceptorAny<K, V> =
    Arc<dyn Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static>;
pub type InterceptorKey<K, V> =
    Arc<dyn Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static>;
pub type SubscriberAny<K, V> = Arc<dyn Fn(&MapChange<K, V>) + Send + Sync + 'static>;
pub type SubscriberKey<K, V> = Arc<dyn Fn(&MapChange<K, V>) + Send + Sync + 'static>;

pub trait ReactiveMapKey: FromStr + Display + Clone + Hash + Eq + Send + Sync + 'static {}
impl<T: FromStr + Display + Clone + Hash + Eq + Send + Sync + 'static> ReactiveMapKey for T {}

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
/// Keyed by the escaped name rather than by `K`, because the contract's order
/// is the order a scan hands the keys back in - `[10, 100, 9]` for numeric
/// keys, not `K: Ord`'s `[9, 10, 100]`. The key `K` rides along in the value so
/// a listing does not have to parse it back.
///
/// A read takes a version and holds nothing, so a walk neither blocks a writer
/// nor waits for one, whatever thread either is on. A write publishes a new
/// version that shares every node it did not touch, so it stays O(log n)
/// rather than copying the map.
///
/// It is a trade, not a free win. Writes and memory both cost more than the
/// `RwLock<BTreeMap>` this replaced, and `benches/map_snapshot_bench.rs`
/// measures against it.
///
/// What it buys is that writing during a walk stops deadlocking. It does not
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

impl<K, V> Default for MapCache<K, V> {
    fn default() -> Self {
        Self {
            entries: ArcSwap::from_pointee(RedBlackTreeMapSync::new_sync()),
        }
    }
}

fn escaped_key<Q: Display + ?Sized>(key: &Q) -> SmolStr {
    SmolStr::new(escape_name(&key.to_string()))
}

impl<K: Clone, V: Clone> MapCache<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get<Q: Display + ?Sized>(&self, key: &Q) -> Option<V> {
        self.entries
            .load()
            .get(escaped_key(key).as_str())
            .map(|(_, value)| value.clone())
    }

    pub fn contains_key<Q: Display + ?Sized>(&self, key: &Q) -> bool {
        self.entries.load().contains_key(escaped_key(key).as_str())
    }

    /// The key as the map holds it, for a caller that looked one up by
    /// something it borrows from and needs the owned form back.
    pub fn owned_key<Q: Display + ?Sized>(&self, key: &Q) -> Option<K> {
        self.entries
            .load()
            .get(escaped_key(key).as_str())
            .map(|(key, _)| key.clone())
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
        Walk::new(self.entries.load_full(), |(key, _)| key.clone())
    }

    /// Every entry, in that order, one at a time.
    pub fn entries(&self) -> Walk<K, V, (K, V)> {
        Walk::new(self.entries.load_full(), Clone::clone)
    }
}

type Snapshot<K, V> = RedBlackTreeMapSync<SmolStr, (K, V)>;
type Held<K, V> = Arc<Snapshot<K, V>>;
type Pairs<'a, K, V> = <&'a Snapshot<K, V> as IntoIterator>::IntoIter;
type Values<'a, K, V> =
    std::iter::Map<Pairs<'a, K, V>, fn((&'a SmolStr, &'a (K, V))) -> &'a (K, V)>;

fn value_of<'a, K, V>((_, entry): (&'a SmolStr, &'a (K, V))) -> &'a (K, V) {
    entry
}

/// A walk of a [`MapCache`], in order, taking nothing it is not asked for.
///
/// The position is the iterator's own, so a whole pass costs one descent and
/// `take(n)` costs `n`.
///
/// The walk owns the version it started on. Writing to the same map while it is
/// alive is allowed from any thread, the walk included, and the walk keeps
/// handing back what its own version holds.
pub struct Walk<K: 'static, V: 'static, T> {
    pairs: Pairs<'static, K, V>,
    take: fn(&(K, V)) -> T,
    _held: Held<K, V>,
}

impl<K, V, T> Walk<K, V, T> {
    fn new(held: Held<K, V>, take: fn(&(K, V)) -> T) -> Self {
        // SAFETY: two invariants, both of which this module has to keep.
        //
        // The map lives in the `Arc`'s allocation and `_held` owns a strong
        // reference to it, so it outlives every borrow taken here however the
        // walk is moved. That is what lets the type carry no lifetime.
        //
        // The `'static` never reaches a caller: `new` is private, and `take` is
        // `for<'x> fn(&'x (K, V)) -> T`, which cannot return its argument. So
        // no `T` can be a reference into the map, and the only `&'static` that
        // exists is the temporary inside `next` and `next_back`.
        let pairs =
            unsafe { std::mem::transmute::<Pairs<'_, K, V>, Pairs<'static, K, V>>(held.iter()) };

        Self {
            pairs,
            take,
            _held: held,
        }
    }
}

impl<K, V, T> Iterator for Walk<K, V, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.pairs.next().map(|(_, entry)| (self.take)(entry))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.pairs.size_hint()
    }
}

impl<K, V, T> DoubleEndedIterator for Walk<K, V, T> {
    fn next_back(&mut self) -> Option<T> {
        self.pairs.next_back().map(|(_, entry)| (self.take)(entry))
    }
}

impl<K, V, T> ExactSizeIterator for Walk<K, V, T> {}

/// One version of a [`MapCache`], in order, borrowed rather than copied.
pub struct Entries<K, V> {
    held: Held<K, V>,
}

impl<'e, K, V> IntoIterator for &'e Entries<K, V> {
    type Item = &'e (K, V);
    type IntoIter = Values<'e, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.held.iter().map(value_of as fn(_) -> _)
    }
}

impl<K, V> Entries<K, V> {
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &(K, V)> {
        self.held.iter().map(|(_, entry)| entry)
    }

    pub fn len(&self) -> usize {
        self.held.size()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

impl<K: Display + Clone, V: Clone> MapCache<K, V> {
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        let escaped = escaped_key(&key);

        let replaced = self
            .entries
            .rcu(|current| current.insert(escaped.clone(), (key.clone(), value.clone())));

        replaced.get(escaped.as_str()).map(|(_, old)| old.clone())
    }

    pub fn remove<Q: Display + ?Sized>(&self, key: &Q) -> Option<(K, V)> {
        let escaped = escaped_key(key);

        let previous = self.entries.rcu(|current| current.remove(escaped.as_str()));

        previous.get(escaped.as_str()).cloned()
    }
}

impl<K: Debug, V: Debug> Debug for MapCache<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = self.entries.load();

        f.debug_map()
            .entries(held.iter().map(|(_, (k, v))| (k, v)))
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

impl<K: Debug + Hash + Eq, V: Debug> Debug for ReactiveMapCore<K, V> {
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
        self.subscribers_any
            .lock()
            .unwrap()
            .push((id, Arc::new(callback), meta));

        let subs_for_name = self.subscribers_any.clone();
        let set_name = Arc::new(move |name: &'static str| {
            if let Ok(mut lock) = subs_for_name.lock()
                && let Some(entry) = lock.iter_mut().find(|(i, _, _)| *i == id)
            {
                entry.2.name = Some(name);
            }
        });
        let subs_for_cleanup = self.subscribers_any.clone();
        SignalSubscription {
            id,
            location,
            name: None,
            set_name,
            cleanup: Arc::new(move |id| {
                if let Ok(mut lock) = subs_for_cleanup.lock() {
                    lock.retain(|(i, _, _)| *i != id);
                }
            }),
        }
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
            if let Some(mut list) = subs_for_name.get_mut(&key_for_name)
                && let Some(entry) = list.iter_mut().find(|(i, _, _)| *i == id)
            {
                entry.2.name = Some(name);
            }
        });
        let subs_for_cleanup = self.subscribers_key.clone();
        SignalSubscription {
            id,
            location,
            name: None,
            set_name,
            cleanup: Arc::new(move |id| {
                if let Some(mut list) = subs_for_cleanup.get_mut(&key) {
                    list.retain(|(i, _, _)| *i != id);
                }
            }),
        }
    }

    pub fn intercept<F>(&self, path: StorePath, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.interceptors_any
            .lock()
            .unwrap()
            .push((id, Arc::new(callback)));
        let subs = self.interceptors_any.clone();
        InterceptDisposer {
            id,
            path,
            cleanup: Arc::new(move |id| {
                if let Ok(mut lock) = subs.lock() {
                    lock.retain(|(i, _)| *i != id);
                }
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

        let interceptors_any = self.interceptors_any.lock().unwrap().clone();
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
            None => Vec::new(),
        };

        let any: Vec<_> = self
            .subscribers_any
            .lock()
            .map(|lock| {
                lock.iter()
                    .map(|(_, cb, meta)| (cb.clone(), *meta))
                    .collect()
            })
            .unwrap_or_default();

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
