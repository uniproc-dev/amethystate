use crate::Store;
use crate::reactive::watch::{Watch, Watchable};
use crate::store::StoreSubscription;
use crate::store::facts::{Facts, Prefix};
use crate::store::sync_backend::SyncBridge;
use crate::store::{Durable, StoreBackend};
use amethystate_core::path::StorePath;
use amethystate_core::{
    Entries, InterceptDisposer, MapChange, ReactiveMapCore, SignalSubscription, Walk,
};
use error_stack::{Report, ResultExt};
use std::borrow::Borrow;
use std::fmt::{self, Debug, Display};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct MapInner<K, V> {
    pub(crate) core: ReactiveMapCore<K, V>,
    pub(crate) path: StorePath,
    pub(crate) instance_id: Uuid,
    pub(crate) store: Store,
    pub(crate) store_sub: Arc<StoreSubscription>,
}

/// A keyed collection in the store, with subscriptions per key.
///
/// The map is resident: it holds every entry in memory, built by one scan when
/// the map is opened and kept current by the store subscription afterwards.
/// Reads are answered from there, so they never touch the disk and never
/// decode - and they see writes that are still buffered as well as those
/// already committed, including edits another process made to the file.
///
/// That residency is the trade this type makes, and it sets the size it is for:
/// thousands of entries, not millions. A map that will not fit in memory wants
/// the database directly.
pub struct ReactiveMap<K, V> {
    pub(crate) inner: Arc<MapInner<K, V>>,
}

use crate::reactive::error::{ReactiveMapError, ReactiveMapResult};
pub use amethystate_core::primitives::map_core::{ReactiveMapKey, ReactiveMapValue};

/// Another handle on the same map **and the same instance id**, so writes
/// through it are indistinguishable from writes through the original.
///
/// [`ReactiveMap::fork`] is the one that gives a new id.
impl<K, V> Clone for ReactiveMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V> Debug for ReactiveMap<K, V>
where
    K: Debug + ReactiveMapKey,
    V: Debug + ReactiveMapValue,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("ReactiveMap");
        d.field("path", &self.inner.path);

        d.field("cache_entries", &self.inner.core.cache.len());

        d.field("core", &self.inner.core).finish()
    }
}

impl<K, V> ReactiveMap<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    /// The same map under a new instance id.
    ///
    /// Provenance travels with the id, so a subscription can tell its own
    /// writes from someone else's. [`Clone`] keeps the id instead; the book
    /// covers the pair in [clone and fork](https://uniproc-dev.github.io/amethystate/concepts/subscriptions/#clone-and-fork).
    pub fn fork(&self) -> Self {
        self.fork_with_id(Uuid::new_v4())
    }

    /// The id writes through this handle carry, so a subscriber can tell them
    /// from someone else's.
    pub fn instance_id(&self) -> Uuid {
        self.inner.instance_id
    }

    /// Where the map lives in the store.
    ///
    /// Lent, not handed over - see [`Field::path`](crate::Field::path).
    pub fn path(&self) -> &StorePath {
        &self.inner.path
    }

    /// [`ReactiveMap::fork`] with the instance id chosen rather than
    /// generated.
    pub fn fork_with_id(&self, new_instance_id: Uuid) -> Self {
        Self {
            inner: Arc::new(MapInner {
                core: self.inner.core.clone(),
                path: self.inner.path.clone(),
                instance_id: new_instance_id,
                store: self.inner.store.clone(),
                store_sub: self.inner.store_sub.clone(),
            }),
        }
    }

    /// The value under `key`, or `None` when there is none.
    ///
    /// Read from the map's projection - see the type docs - so it costs a
    /// lookup and no disk.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// assert_eq!(widths.get("cpu"), None);
    /// widths.insert("cpu".into(), &120).unwrap();
    /// assert_eq!(widths.get("cpu"), Some(120));
    /// ```
    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        self.inner.core.cache.get(key)
    }

    /// Whether `key` has a value, without cloning it.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// assert!(!widths.contains_key("cpu"));
    /// widths.insert("cpu".into(), &120).unwrap();
    /// assert!(widths.contains_key("cpu"));
    /// ```
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        self.inner.core.cache.contains_key(key)
    }

    /// Every entry, sorted by key.
    ///
    /// Sorted by the key's string form rather than by the key type's own `Ord`
    /// - see [`ReactiveMap::keys`], where the difference is worked through.
    ///
    /// Read from the map's projection, so nothing is decoded and the disk is
    /// not touched. The projection is already in this order, so nothing is
    /// sorted, and nothing is taken that is not asked for: `take(50)` clones
    /// fifty entries whatever the map holds.
    ///
    /// The walk owns the version it started on, so writing to this map while it
    /// is alive is allowed and the walk goes on handing back what it took.
    pub fn entries(&self) -> Walk<K, V, (K, V)> {
        self.inner.core.cache.entries()
    }

    /// The projection itself, held open, for a caller that wants to look
    /// rather than to take.
    ///
    /// [`ReactiveMap::entries`] clones what it hands out, because `Iterator`
    /// cannot lend from the thing it is iterating - `Item` is fixed on the
    /// impl and cannot borrow from `&mut self`. This lends instead: bind it,
    /// then `iter()`, and nothing is cloned that the caller does not clone.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    /// widths.insert("cpu".into(), &120).unwrap();
    /// widths.insert("mem".into(), &80).unwrap();
    ///
    /// let held = widths.view();
    /// let total: u64 = held.iter().map(|(_, width)| width).sum();
    ///
    /// assert_eq!(total, 200);
    ///
    /// // A `for` keeps the temporary alive for the whole loop, so the binding
    /// // is only needed when what is read outlives the loop.
    /// let mut widest = 0;
    /// for (_, width) in &widths.view() {
    ///     widest = widest.max(*width);
    /// }
    ///
    /// assert_eq!(widest, 120);
    /// ```
    pub fn view(&self) -> Entries<K, V> {
        self.inner.core.cache.view()
    }

    /// Every key, sorted. Values are neither read nor deserialized.
    ///
    /// Sorted the way the store orders the keys these names become, not by the
    /// key type's own `Ord`, so a scan and a map list their entries alike. For
    /// numbers that shows as text order - `"10"` before `"9"` - and for a name
    /// holding the separator it shows as the escape: the store sorts `a.b`
    /// after `a1b`, because the key it writes begins `a\.`.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// // Inserted out of order.
    /// widths.insert("mem".into(), &80).unwrap();
    /// widths.insert("cpu".into(), &120).unwrap();
    /// widths.insert("disk".into(), &60).unwrap();
    ///
    /// assert_eq!(widths.keys().collect::<Vec<_>>(), ["cpu", "disk", "mem"]);
    ///
    /// // Numeric keys sort as text, so 10 lands before 9.
    /// let ports = store.kv().map::<u16, bool>("ports").unwrap();
    /// ports.insert(9, &true).unwrap();
    /// ports.insert(10, &true).unwrap();
    /// ports.insert(100, &true).unwrap();
    /// assert_eq!(ports.keys().collect::<Vec<_>>(), [10, 100, 9]);
    ///
    /// // A name holding the separator sorts by the key it becomes, so it lands
    /// // where a scan puts it rather than where the bare name would.
    /// let odd = store.kv().map::<String, u8>("odd").unwrap();
    /// odd.insert("a.b".into(), &1).unwrap();
    /// odd.insert("a1b".into(), &2).unwrap();
    /// assert_eq!(odd.keys().collect::<Vec<_>>(), ["a1b", "a.b"]);
    /// ```
    ///
    /// Only the keys asked for are cloned, and the value beside each is not
    /// touched. The walk owns the version it started on.
    pub fn keys(&self) -> Walk<K, V, K> {
        self.inner.core.cache.keys()
    }

    /// How many entries the map holds.
    ///
    /// Answered from the map's projection, so it is a counter read rather than
    /// a scan, and it counts buffered writes that have not reached disk yet.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// assert_eq!(widths.len(), 0);
    /// widths.insert("cpu".into(), &120).unwrap();
    /// widths.insert("mem".into(), &80).unwrap();
    /// assert_eq!(widths.len(), 2);
    ///
    /// // Writing an existing key does not add one.
    /// widths.update("cpu", &200).unwrap();
    /// assert_eq!(widths.len(), 2);
    /// ```
    pub fn len(&self) -> usize {
        self.inner.core.cache.len()
    }

    /// Whether the map holds nothing.
    pub fn is_empty(&self) -> bool {
        self.inner.core.cache.is_empty()
    }

    /// Calls `callback` on every change to any key.
    ///
    /// The subscription lives as long as the returned handle: dropping it
    /// unsubscribes. Callbacks run on whichever thread made the change, which
    /// may be the file watcher's when the store was edited outside the
    /// process - hence the `Send + Sync` bound.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    /// # use amethystate::MapChange;
    /// # use std::sync::{Arc, Mutex};
    /// let seen = Arc::new(Mutex::new(Vec::new()));
    /// let sink = Arc::clone(&seen);
    ///
    /// let sub = widths.subscribe_any(move |change| {
    ///     sink.lock().unwrap().push(match change {
    ///         MapChange::Insert { .. } => "insert",
    ///         MapChange::Update { .. } => "update",
    ///         MapChange::Remove { .. } => "remove",
    ///         MapChange::Clear { .. } => "clear",
    ///     });
    /// });
    ///
    /// widths.insert("cpu".into(), &120).unwrap();
    /// widths.update("cpu", &200).unwrap();
    /// widths.remove("cpu").unwrap();
    /// assert_eq!(*seen.lock().unwrap(), ["insert", "update", "remove"]);
    ///
    /// drop(sub);
    /// widths.insert("mem".into(), &80).unwrap();
    /// assert_eq!(seen.lock().unwrap().len(), 3, "dropping the handle unsubscribed");
    /// ```
    ///
    /// A callback sees the change while it is still buffered, before it
    /// reaches disk - see [the module docs](crate::reactive).
    #[track_caller]
    pub fn subscribe_any<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.inner.core.subscribe_any(callback)
    }

    /// Calls `callback` only for changes to one key.
    ///
    /// Cheaper than filtering inside [`ReactiveMap::subscribe_any`]: changes
    /// to other keys never reach the callback at all.
    /// Calls `callback` only for changes to one key.
    ///
    /// Cheaper than filtering inside [`ReactiveMap::subscribe_any`]: changes
    /// to other keys never reach the callback at all.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    /// # use std::sync::{Arc, Mutex};
    /// let hits = Arc::new(Mutex::new(0u32));
    /// let sink = Arc::clone(&hits);
    ///
    /// let _sub = widths.subscribe_key("cpu".to_string(), move |_| {
    ///     *sink.lock().unwrap() += 1;
    /// });
    ///
    /// widths.insert("cpu".into(), &120).unwrap();
    /// widths.insert("mem".into(), &80).unwrap();   // a different key
    /// widths.update("cpu", &200).unwrap();
    ///
    /// assert_eq!(*hits.lock().unwrap(), 2, "only the two writes to `cpu`");
    /// ```
    #[track_caller]
    pub fn subscribe_key<F>(&self, key: K, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.inner.core.subscribe_key(key, callback)
    }

    /// Configures a subscription. See [`Watch`].
    ///
    /// Map changes are events, and every one is delivered.
    /// [`Watch::external`] is how a subscriber skips the changes this handle
    /// made itself.
    pub fn subscription_with(&self) -> Watch<Self> {
        Watch::new(self.clone())
    }
}

/// One key of a map, as a [`Watch`] source. Built by [`Watch::key`].
pub struct KeyOf<K, V> {
    map: ReactiveMap<K, V>,
    key: K,
}

impl<K, V> KeyOf<K, V> {
    pub(crate) fn new(map: ReactiveMap<K, V>, key: K) -> Self {
        Self { map, key }
    }
}

impl<K, V> Watchable for KeyOf<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    type Item = MapChange<K, V>;

    fn filterable(item: &MapChange<K, V>) -> bool {
        matches!(item, MapChange::Update { .. })
    }

    fn watch_id(&self) -> Uuid {
        self.map.inner.instance_id
    }

    #[track_caller]
    fn watch_raw<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>, Option<Uuid>) + Send + Sync + 'static,
    {
        self.map
            .inner
            .core
            .subscribe_key(self.key.clone(), move |change| {
                callback(change, change.source())
            })
    }
}

impl<K, V> Watchable for ReactiveMap<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    type Item = MapChange<K, V>;

    fn filterable(item: &MapChange<K, V>) -> bool {
        matches!(item, MapChange::Update { .. })
    }

    fn watch_id(&self) -> Uuid {
        self.inner.instance_id
    }

    #[track_caller]
    fn watch_raw<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>, Option<Uuid>) + Send + Sync + 'static,
    {
        self.inner
            .core
            .subscribe_any(move |change| callback(change, change.source()))
    }
}

impl<K, V> ReactiveMap<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    /// Reads the key, hands the value to `f` and writes back what it returns,
    /// then yields the new value. [`ReactiveMap::modify`] does the same
    /// through `&mut` instead. Fails with
    /// [`ReactiveMapError::KeyNotFound`] when the key is absent.
    pub fn update_with<Q, F>(&self, key: &Q, f: F) -> ReactiveMapResult<Option<V>>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
        F: FnOnce(V) -> V,
    {
        if let Some(val) = self.get(key) {
            let new_val = f(val);
            self.update(key, &new_val)?;
            Ok(Some(new_val))
        } else {
            Err(Report::new(ReactiveMapError::KeyNotFound(key.to_string()))
                .attach(Prefix(self.inner.path.clone())))
        }
    }

    /// Reads the key, lets `f` change the value in place and writes it back.
    /// Fails with [`ReactiveMapError::KeyNotFound`] when the key is absent.
    pub fn modify<Q, F>(&self, key: &Q, f: F) -> ReactiveMapResult<()>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
        F: FnOnce(&mut V),
    {
        if let Some(mut val) = self.get(key) {
            f(&mut val);
            self.update(key, &val)
        } else {
            Err(Report::new(ReactiveMapError::KeyNotFound(key.to_string()))
                .attach(Prefix(self.inner.path.clone())))
        }
    }

    /// The same writes, each returning only once the change is on disk.
    ///
    /// `insert`, `update` and friends leave the change in the write buffer,
    /// where a crash loses it; these pay a commit to close that window.
    pub fn durable(&self) -> Durable<'_, Self> {
        Durable(self)
    }

    /// Writes a key that is already there, and fails with
    /// [`ReactiveMapError::KeyNotFound`] when it is not.
    ///
    /// Subscribers receive [`MapChange::Update`], which carries the previous
    /// value, and a key that does not exist has none. Use
    /// [`ReactiveMap::insert`] to add one.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// // `update` refuses a key that is not there yet.
    /// assert!(widths.update("cpu", &120).is_err());
    ///
    /// // `insert` is the one that adds it.
    /// widths.insert("cpu".into(), &120).unwrap();
    /// assert_eq!(widths.get("cpu"),Some(120));
    ///
    /// // Once the key exists, the strict write lands.
    /// widths.update("cpu", &200).unwrap();
    /// assert_eq!(widths.get("cpu"),Some(200));
    /// ```
    pub fn update<Q>(&self, key: &Q, value: &V) -> ReactiveMapResult<()>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        let Some(owned) = self.inner.core.cache.owned_key(key) else {
            return Err(Report::new(ReactiveMapError::KeyNotFound(key.to_string()))
                .attach(Prefix(self.inner.path.clone())));
        };

        let backend = SyncBridge::new(self.inner.store.clone());
        Ok(amethystate_core::map_update(
            &backend,
            &self.inner.core,
            self.inner.path.clone(),
            owned,
            value,
            Some(self.inner.instance_id),
        )?)
    }

    /// Writes a key whether or not it is already there, like
    /// [`std::collections::HashMap::insert`].
    ///
    /// Subscribers see [`MapChange::Insert`] for a new key and
    /// [`MapChange::Update`] for one that existed.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// // A key that was not there.
    /// widths.insert("cpu".into(), &120).unwrap();
    /// assert_eq!(widths.get("cpu"),Some(120));
    ///
    /// // The same call replaces one that was.
    /// widths.insert("cpu".into(), &200).unwrap();
    /// assert_eq!(widths.get("cpu"),Some(200));
    /// assert_eq!(widths.len(),1, "replacing is not adding");
    /// ```
    pub fn insert(&self, key: K, value: &V) -> ReactiveMapResult<()> {
        let backend = SyncBridge::new(self.inner.store.clone());
        Ok(amethystate_core::map_insert(
            &backend,
            &self.inner.core,
            self.inner.path.clone(),
            key,
            value,
            Some(self.inner.instance_id),
        )?)
    }

    /// Drops a key and yields the value it held, or `None` if there was
    /// none. Removing an absent key is not an error.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// widths.insert("cpu".into(), &120).unwrap();
    /// assert_eq!(widths.remove("cpu").unwrap(), Some(120));
    /// assert_eq!(widths.remove("cpu").unwrap(), None);
    /// ```
    pub fn remove<Q>(&self, key: &Q) -> ReactiveMapResult<Option<V>>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        let Some(owned) = self.inner.core.cache.owned_key(key) else {
            return Ok(None);
        };

        let backend = SyncBridge::new(self.inner.store.clone());
        Ok(amethystate_core::map_remove(
            &backend,
            &self.inner.core,
            self.inner.path.clone(),
            owned,
            Some(self.inner.instance_id),
        )?)
    }

    /// Drops every entry, as one operation rather than a removal per key.
    ///
    /// Subscribers see a single [`MapChange::Clear`], not one `Remove` each.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// widths.insert("cpu".into(), &120).unwrap();
    /// widths.insert("mem".into(), &80).unwrap();
    /// widths.clear().unwrap();
    /// assert!(widths.is_empty());
    /// ```
    pub fn clear(&self) -> ReactiveMapResult<()> {
        let backend = SyncBridge::new(self.inner.store.clone());
        Ok(amethystate_core::map_clear(
            &backend,
            &self.inner.core,
            self.inner.path.clone(),
            Some(self.inner.instance_id),
        )?)
    }

    /// Installs a callback that sees every change to any key before it lands
    /// and may rewrite or reject it.
    ///
    /// Returning `None` drops the change; returning a different
    /// [`MapChange`] stores that instead. The interceptor stays installed
    /// until [`InterceptDisposer::remove`] is called - dropping the disposer
    /// only forgets the handle, unlike a subscription, which ends on drop.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// # use amethystate::MapChange;
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    ///
    /// let guard = widths.intercept(|change| match change {
    ///     MapChange::Insert { value, .. } if value > 500 => None,
    ///     other => Some(other),
    /// });
    ///
    /// widths.insert("cpu".into(), &120).unwrap();
    /// assert!(widths.insert("mem".into(), &900).is_err(), "over the limit");
    /// assert_eq!(widths.len(),1);
    ///
    /// guard.remove();
    /// widths.insert("mem".into(), &900).unwrap();
    /// assert_eq!(widths.len(),2);
    /// ```
    ///
    /// Interceptors run before the value reaches the buffer at all - see
    /// [the module docs](crate::reactive) for where that sits relative to disk.
    pub fn intercept<F>(&self, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        self.inner.core.intercept(self.inner.path.clone(), callback)
    }

    /// [`ReactiveMap::intercept`] narrowed to one key.
    ///
    /// Changes to other keys pass through untouched, so this is both cheaper
    /// and safer than matching on the key inside a map-wide interceptor.
    pub fn intercept_key<F>(&self, key: K, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        self.inner.core.intercept_key(key, callback)
    }
}

impl<K, V> PartialEq for ReactiveMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.path == other.inner.path
            && self.inner.instance_id == other.inner.instance_id
            && Arc::ptr_eq(&self.inner.core.next_id, &other.inner.core.next_id)
    }
}

impl<K, V> Eq for ReactiveMap<K, V> {}

impl<K, V> Durable<'_, ReactiveMap<K, V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    /// Writes a key that is already there; an absent one gives
    /// [`ReactiveMapError::KeyNotFound`].
    ///
    /// Returns only once the change is on disk rather than buffered.
    pub fn update<Q>(&self, key: &Q, value: &V) -> ReactiveMapResult<()>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        self.0.update(key, value)?;
        self.commit()
    }

    /// Writes a key that is already there; an absent one gives
    /// [`ReactiveMapError::KeyNotFound`].
    ///
    /// Resolves once the change is on disk rather than buffered.
    /// Like every future, this does nothing until awaited - the write
    /// included. See [`Durable::set_async`].
    pub async fn update_async<Q>(&self, key: &Q, value: &V) -> ReactiveMapResult<()>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        self.0.update(key, value)?;
        self.commit_async().await
    }

    /// Writes a key whether or not it is already there.
    ///
    /// Returns only once the change is on disk rather than buffered.
    pub fn insert(&self, key: K, value: &V) -> ReactiveMapResult<()> {
        self.0.insert(key, value)?;
        self.commit()
    }

    /// Writes a key whether or not it is already there.
    ///
    /// Resolves once the change is on disk rather than buffered.
    /// Like every future, this does nothing until awaited - the write
    /// included. See [`Durable::set_async`].
    pub async fn insert_async(&self, key: K, value: &V) -> ReactiveMapResult<()> {
        self.0.insert(key, value)?;
        self.commit_async().await
    }

    /// Drops a key and yields the value it held.
    ///
    /// Returns only once the change is on disk rather than buffered.
    pub fn remove<Q>(&self, key: &Q) -> ReactiveMapResult<Option<V>>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        let previous = self.0.remove(key)?;
        self.commit()?;
        Ok(previous)
    }

    /// Drops a key and yields the value it held.
    ///
    /// Resolves once the change is on disk rather than buffered.
    /// Like every future, this does nothing until awaited - the write
    /// included. See [`Durable::set_async`].
    pub async fn remove_async<Q>(&self, key: &Q) -> ReactiveMapResult<Option<V>>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
    {
        let previous = self.0.remove(key)?;
        self.commit_async().await?;
        Ok(previous)
    }

    /// Drops every entry, as one operation rather than a removal per key.
    ///
    /// Returns only once the change is on disk rather than buffered.
    pub fn clear(&self) -> ReactiveMapResult<()> {
        self.0.clear()?;
        self.commit()
    }

    /// Drops every entry, as one operation rather than a removal per key.
    ///
    /// Resolves once the change is on disk rather than buffered.
    /// Like every future, this does nothing until awaited - the write
    /// included. See [`Durable::set_async`].
    pub async fn clear_async(&self) -> ReactiveMapResult<()> {
        self.0.clear()?;
        self.commit_async().await
    }

    /// Reads the key, writes back what `f` returns, and yields the new value.
    ///
    /// Returns only once the change is on disk rather than buffered.
    pub fn update_with<Q, F>(&self, key: &Q, f: F) -> ReactiveMapResult<Option<V>>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
        F: FnOnce(V) -> V,
    {
        let value = self.0.update_with(key, f)?;
        self.commit()?;
        Ok(value)
    }

    /// Reads the key, writes back what `f` returns, and yields the new value.
    ///
    /// Resolves once the change is on disk rather than buffered.
    /// Like every future, this does nothing until awaited - the write
    /// included. See [`Durable::set_async`].
    pub async fn update_with_async<Q, F>(&self, key: &Q, f: F) -> ReactiveMapResult<Option<V>>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
        F: FnOnce(V) -> V,
    {
        let value = self.0.update_with(key, f)?;
        self.commit_async().await?;
        Ok(value)
    }

    /// Reads the key, lets `f` change the value in place, and writes it back.
    ///
    /// Returns only once the change is on disk rather than buffered.
    pub fn modify<Q, F>(&self, key: &Q, f: F) -> ReactiveMapResult<()>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
        F: FnOnce(&mut V),
    {
        self.0.modify(key, f)?;
        self.commit()
    }

    /// Like every future, this does nothing until awaited - the write
    /// included. See [`Durable::set_async`].
    pub async fn modify_async<Q, F>(&self, key: &Q, f: F) -> ReactiveMapResult<()>
    where
        K: Borrow<Q>,
        Q: Display + ?Sized,
        F: FnOnce(&mut V),
    {
        self.0.modify(key, f)?;
        self.commit_async().await
    }

    fn commit(&self) -> ReactiveMapResult<()> {
        self.0
            .inner
            .store
            .flush_prefix(&self.0.inner.path)
            .change_context(ReactiveMapError::Storage)
            .attach_prefix(&self.0.inner.path)?;
        Ok(())
    }

    async fn commit_async(&self) -> ReactiveMapResult<()> {
        self.0
            .inner
            .store
            .flush_async()
            .await
            .change_context(ReactiveMapError::Storage)
            .attach_prefix(&self.0.inner.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    struct TestScope;
    impl crate::StateScope for TestScope {
        const PATH: StorePath = StorePath::from_static(&["test"], "test");
        const KEY: &'static str = "test";
    }

    use super::*;

    use crate::store::StorageError;
    use crate::test_utils::unique_store;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tracing_test::traced_test;

    /// `*_external` filters `Update` and nothing else: a value this handle
    /// rewrote is its own business, but a key appearing or disappearing changes
    /// what the map holds and goes to everyone.
    ///
    /// Pins the whole matrix, where `test_map_subscribe_external` covers only
    /// `Insert` and `Update`.
    #[test]
    fn external_subscriptions_filter_own_updates_only() {
        let store = unique_store("external-own-changes");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test_map", "external"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .expect("map should be created");

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let any = seen.clone();
        let _any_sub = map.subscription_with().external().register(move |change| {
            any.lock().unwrap().push(match change {
                MapChange::Insert { .. } => "insert".into(),
                MapChange::Update { .. } => "update".into(),
                MapChange::Remove { .. } => "remove".into(),
                MapChange::Clear { .. } => "clear".into(),
            });
        });

        map.insert("a".to_string(), &1).unwrap(); // Insert - delivered
        map.update("a", &2).unwrap(); // Update - filtered
        map.insert("a".to_string(), &3).unwrap(); // Update - filtered
        map.remove("a").unwrap(); // Remove - delivered

        assert_eq!(
            *seen.lock().unwrap(),
            vec!["insert".to_string(), "remove".to_string()],
            "own updates stay hidden, own structural changes do not"
        );

        seen.lock().unwrap().clear();
        map.insert("b".to_string(), &4).unwrap();
        seen.lock().unwrap().clear();
        map.clear().unwrap();

        assert_eq!(
            *seen.lock().unwrap(),
            vec!["clear".to_string()],
            "clear is one event, not one per key it dropped"
        );

        seen.lock().unwrap().clear();
        let other = map.fork();
        other.insert("c".to_string(), &5).unwrap();
        other.update("c", &6).unwrap();

        assert_eq!(
            *seen.lock().unwrap(),
            vec!["insert".to_string(), "update".to_string()],
            "another handle's updates must arrive"
        );
    }

    /// The keyed variant filters on the same rule, scoped to one key.
    #[test]
    fn external_key_subscription_filters_own_updates_only() {
        let store = unique_store("external-own-key");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test_map", "external_key"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .expect("map should be created");

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let keyed = seen.clone();
        let _key_sub = map
            .subscription_with()
            .key("a".to_string())
            .external()
            .register(move |change| {
                keyed.lock().unwrap().push(match change {
                    MapChange::Insert { .. } => "insert".into(),
                    MapChange::Update { .. } => "update".into(),
                    MapChange::Remove { .. } => "remove".into(),
                    MapChange::Clear { .. } => "clear".into(),
                });
            });

        map.insert("a".to_string(), &1).unwrap();
        map.update("a", &2).unwrap();
        map.insert("b".to_string(), &3).unwrap(); // other key, not ours
        map.remove("a").unwrap();

        assert_eq!(
            *seen.lock().unwrap(),
            vec!["insert".to_string(), "remove".to_string()],
            "own update filtered, own structural changes delivered, other keys ignored"
        );
    }

    #[test]
    fn test_map_crud_logic() {
        let store = unique_store("crud");
        let path = StorePath::from_segments(["test_map", "data"]);

        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                path,
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        map.insert("a".into(), &10).unwrap();
        assert_eq!(map.get("a"), Some(10));
        assert_eq!(map.len(), 1);

        map.update("a", &20).unwrap();
        assert_eq!(map.get("a"), Some(20));

        let res = map.update("missing", &30);
        assert!(matches!(
            res.unwrap_err().current_context(),
            ReactiveMapError::KeyNotFound(_)
        ));

        map.insert("b".into(), &100).unwrap();
        let entries: Vec<_> = map.entries().collect();
        assert_eq!(entries.len(), 2);

        let removed = map.remove("a").unwrap();
        assert_eq!(removed, Some(20));
        assert_eq!(map.len(), 1);

        store.save_now().unwrap();
        assert_eq!(map.get("a"), None);
    }

    #[test]
    fn test_map_intercept_and_reject() {
        let store = unique_store("reject");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "intercept"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        map.intercept(|change| match change {
            MapChange::Insert { value, .. }
            | MapChange::Update {
                new_value: value, ..
            } if value < 0 => None,
            _ => Some(change),
        });

        let res = map.insert("val".into(), &-1);
        assert!(matches!(
            res.unwrap_err().current_context(),
            ReactiveMapError::Intercepted
        ));

        store.save_now().unwrap();
        assert_eq!(map.get("val"), None);

        map.insert("val".into(), &10).unwrap();
        assert_eq!(map.get("val"), Some(10));
    }

    #[test]
    fn test_map_intercept_transform() {
        let store = unique_store("transform");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "transform"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        map.intercept(|change| match change {
            MapChange::Insert { key, value, source } => Some(MapChange::Insert {
                key,
                value: value * 2,
                source,
            }),
            MapChange::Update {
                key,
                old_value,
                new_value,
                source,
            } => Some(MapChange::Update {
                key,
                old_value,
                new_value: new_value * 2,
                source,
            }),
            _ => Some(change),
        });

        map.insert("x".into(), &5).unwrap();
        assert_eq!(map.get("x"), Some(10));
    }

    #[test]
    fn test_map_subscriptions() {
        let store = unique_store("subs");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "subs"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let e_clone = events.clone();

        let _sub = map.subscribe_any(move |change| {
            e_clone.lock().unwrap().push(change.clone());
        });

        map.insert("key1".into(), &1).unwrap();
        map.update("key1", &2).unwrap();
        map.remove("key1").unwrap();

        std::thread::sleep(Duration::from_millis(100));

        let res = events.lock().unwrap();

        assert!(res.len() >= 3);
        assert!(matches!(res[0], MapChange::Insert { .. }));
        assert!(matches!(res[1], MapChange::Update { .. }));
        assert!(matches!(res[2], MapChange::Remove { .. }));
    }

    #[test]
    fn test_reentrancy_guard() {
        let store = unique_store("reentrancy");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "reentrancy"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        let map_clone = map.clone();
        map.intercept(move |change| {
            if let MapChange::Update { key, .. } = &change
                && key == "a"
            {
                let _ = map_clone.update("a", &999);
            }
            Some(change)
        });

        map.insert("a".into(), &1).unwrap();
        map.update("a", &2).unwrap();

        assert_eq!(map.get("a"), Some(2));
    }

    #[test]
    fn test_map_clear() {
        let store = unique_store("clear");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "clear"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        map.insert("k1".into(), &1).unwrap();
        map.insert("k2".into(), &2).unwrap();

        assert_eq!(map.len(), 2);

        let clear_events_count = Arc::new(AtomicUsize::new(0));
        let clear_events_count_clone = clear_events_count.clone();

        let _sub = map.subscribe_any(move |change| {
            if let MapChange::Clear { .. } = change {
                clear_events_count_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        map.clear().unwrap();
        store.save_now().unwrap();

        assert_eq!(map.len(), 0);
        assert!(map.is_empty());

        assert_eq!(clear_events_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_contains_key_and_cleanup() {
        let store = unique_store("contains");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "contains"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        map.insert("key1".into(), &1).unwrap();

        assert!(map.contains_key("key1"));
        assert!(!map.contains_key("key2"));

        let call_count = Arc::new(AtomicUsize::new(0));
        let c_clone = call_count.clone();
        {
            let _sub = map.subscribe_any(move |_| {
                c_clone.fetch_add(1, Ordering::SeqCst);
            });
            map.update("key1", &2).unwrap();
            assert_eq!(call_count.load(Ordering::SeqCst), 1);
        }

        map.update("key1", &3).unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_key_specific_logic() {
        let store = unique_store("key_spec");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "keyspec"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        map.insert("target".into(), &10).unwrap();
        map.insert("other".into(), &20).unwrap();

        let target_calls = Arc::new(AtomicUsize::new(0));
        let t_clone = target_calls.clone();
        let _sub = map.subscribe_key("target".into(), move |_| {
            t_clone.fetch_add(1, Ordering::SeqCst);
        });

        map.update("target", &11).unwrap();
        map.update("other", &21).unwrap();
        assert_eq!(target_calls.load(Ordering::SeqCst), 1);

        map.intercept_key("target".into(), |change| {
            if let MapChange::Update { new_value, .. } = change
                && new_value > 100
            {
                return None;
            }
            Some(change)
        });

        map.update("target", &50).unwrap();
        let res = map.update("target", &150);
        assert!(matches!(
            res.unwrap_err().current_context(),
            ReactiveMapError::Intercepted
        ));

        map.update("other", &150).unwrap();
    }

    /// Reopening a map's path under a different key type is a refusal, not a
    /// filter.
    ///
    /// Both entries here are the map's - they are under its own path - and
    /// neither can be read as `<i32, i32>`. Dropping them would leave a map
    /// short of what the store holds, and the next write of it whole would
    /// delete them. Changing the type of a stored map is what migrations are
    /// for.
    #[test]
    fn a_map_reopened_at_another_type_refuses_rather_than_filtering() {
        let store = unique_store("parsing");
        let path = StorePath::from_segments(["test", "parse"]);

        {
            let map_str: ReactiveMap<String, String> =
                crate::store::reactive_map_with_path::<TestScope, _, _>(
                    &store,
                    path.clone(),
                    HashMap::new(),
                    Uuid::new_v4(),
                )
                .unwrap();

            map_str.insert("not_int_key".into(), &"1".into()).unwrap();
            map_str
                .insert("123".into(), &"invalid_value".into())
                .unwrap();
        }

        let err = crate::store::reactive_map_with_path::<TestScope, i32, i32>(
            &store,
            path,
            HashMap::new(),
            Uuid::new_v4(),
        )
        .unwrap_err();

        assert_eq!(
            err.current_context(),
            &StorageError::Codec,
            "a key that is not an `i32` and a value that is not one are both the \
             codec's refusal, not a read or an open failure"
        );

        let report = format!("{err:?}");
        assert!(
            report.contains("entry: 123") || report.contains("entry: not_int_key"),
            "the report names the entry it could not read: {report}"
        );
        assert!(
            report.contains("prefix: test.parse"),
            "the report names the map the entry is in: {report}"
        );
    }

    #[test]
    fn test_remove_edge_cases() {
        let store = unique_store("remove_edge");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "remove"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        let res = map.remove("none").unwrap();
        assert!(res.is_none());

        map.insert("ghost".into(), &1).unwrap();
        store.delete(["test", "remove", "ghost"]).unwrap();

        let res = map.remove("ghost").unwrap();
        assert!(res.is_none());
        assert!(!map.contains_key("ghost"));
    }

    #[test]
    #[traced_test]
    fn test_map_recursion_warning() {
        let store = unique_store("map_trace");
        let map: ReactiveMap<String, i32> =
            crate::store::reactive_map_with_path::<TestScope, _, _>(
                &store,
                ["test", "recursive_map"],
                HashMap::new(),
                Uuid::new_v4(),
            )
            .unwrap();

        let map_clone = map.clone();

        map.intercept(move |change| {
            if let Some(key) = change.key() {
                let _ = map_clone.insert(key.clone(), &999);
            }
            Some(change)
        });

        let _ = map.insert("key_a".into(), &1);

        assert!(logs_contain("maximum intercept depth reached"));
        assert!(logs_contain("test.recursive_map.key_a"));
    }
    #[test]
    fn test_map_subscribe_external() {
        let store = unique_store("map_external");
        let map = crate::store::reactive_map_with_path::<TestScope, String, i32>(
            &store,
            ["test", "external"],
            HashMap::new(),
            Uuid::new_v4(),
        )
        .unwrap();

        let fork = map.fork();

        let calls = Arc::new(AtomicUsize::new(0));
        let c_clone = calls.clone();

        let _sub = map.subscription_with().external().register(move |_| {
            c_clone.fetch_add(1, Ordering::SeqCst);
        });

        map.insert("a".into(), &1).unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "Creation (Insert) is NOT ignored"
        );

        map.update("a", &10).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "Own updates are ignored");

        fork.update("a", &20).unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "Fork updates are processed"
        );
    }
}
