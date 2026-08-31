use crate::async_impl::{AsyncSubscriptionBackend, SubscriptionHandle};
use crate::path::StorePath;
use crate::primitives::error::{ReactiveMapError, ReactiveMapResult};
use crate::primitives::map_core::{ReactiveMapKey, ReactiveMapValue};
use crate::{InterceptDisposer, MapChange, ReactiveMapCore, SignalSubscription};
use error_stack::Report;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub struct ReactiveMap<K, V, B> {
    pub core: ReactiveMapCore<K, V>,
    pub prefix: StorePath,
    pub instance_id: Uuid,
    _subscription: Arc<Mutex<SubscriptionHandle>>,
    backend: B,
}

impl<K, V, B> Clone for ReactiveMap<K, V, B>
where
    B: Clone,
{
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            prefix: self.prefix.clone(),
            instance_id: self.instance_id,
            _subscription: self._subscription.clone(),
            backend: self.backend.clone(),
        }
    }
}

impl<K, V, B> PartialEq for ReactiveMap<K, V, B> {
    fn eq(&self, other: &Self) -> bool {
        self.prefix == other.prefix
            && self.instance_id == other.instance_id
            && Arc::ptr_eq(&self.core.next_id, &other.core.next_id)
    }
}

impl<K, V, B> Eq for ReactiveMap<K, V, B> {}

impl<K, V, B> Debug for ReactiveMap<K, V, B>
where
    K: Debug + Hash + Eq + Clone,
    V: Debug + Clone,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("ReactiveMap");
        d.field("prefix", &self.prefix);

        d.field("cache_entries", &self.core.cache.len());

        d.field("core", &self.core).finish()
    }
}

impl<K, V, B> ReactiveMap<K, V, B>
where
    K: ReactiveMapKey + for<'de> Deserialize<'de>,
    V: ReactiveMapValue,
    B: AsyncSubscriptionBackend,
{
    pub fn fork(&self) -> Self {
        self.fork_with_id(Uuid::new_v4())
    }

    pub fn fork_with_id(&self, new_instance_id: Uuid) -> Self {
        Self {
            core: self.core.clone(),
            prefix: self.prefix.clone(),
            instance_id: new_instance_id,
            _subscription: self._subscription.clone(),
            backend: self.backend.clone(),
        }
    }

    pub fn new(prefix: StorePath, initial_values: HashMap<K, V>) -> Self
    where
        B: Default,
    {
        Self::new_with_backend(prefix, initial_values, B::default())
    }

    pub fn new_with_backend(prefix: StorePath, initial_values: HashMap<K, V>, backend: B) -> Self {
        Self::new_with_backend_and_id(prefix, initial_values, backend, Uuid::new_v4())
    }

    pub fn new_with_backend_and_id(
        prefix: StorePath,
        initial_values: HashMap<K, V>,
        backend: B,
        instance_id: Uuid,
    ) -> Self {
        let core = ReactiveMapCore::new();

        for (k, v) in initial_values {
            core.cache.insert(k, v);
        }

        let subscription = backend.subscribe_map(prefix.clone(), core.clone());

        Self {
            core,
            prefix,
            instance_id,
            _subscription: Arc::new(Mutex::new(subscription)),
            backend,
        }
    }

    pub fn get_sync(&self, key: &K) -> ReactiveMapResult<Option<V>> {
        Ok(self.core.cache.get(key).map(|v| v.clone()))
    }

    pub async fn get(&self, key: &K) -> ReactiveMapResult<Option<V>> {
        crate::map_get_async(&self.backend, &self.prefix, key).await
    }

    pub async fn remove(&self, key: K) -> ReactiveMapResult<Option<V>> {
        crate::map_remove_async(
            &self.backend,
            &self.core,
            self.prefix.clone(),
            key,
            Some(self.instance_id),
        )
        .await
    }

    /// The cached entries, in the order a scan lists them.
    pub fn values(&self) -> ReactiveMapResult<Vec<(K, V)>> {
        Ok(self.core.cache.entries().collect())
    }

    /// Every entry, sorted by key.
    pub async fn entries(&self) -> ReactiveMapResult<Vec<(K, V)>> {
        let mut entries: Vec<(K, V)> =
            crate::map_entries_async(&self.backend, &self.prefix).await?;
        entries.sort_by_key(|(k, _)| k.to_string());
        Ok(entries)
    }

    pub async fn update<F>(&self, key: K, f: F) -> ReactiveMapResult<Option<V>>
    where
        F: FnOnce(V) -> V,
    {
        if let Some(val) = self.get(&key).await? {
            let new_val = f(val);
            self.set(key, &new_val).await?;
            Ok(Some(new_val))
        } else {
            Err(Report::new(ReactiveMapError::KeyNotFound(key.to_string()))
                .attach(format!("map: {}", self.prefix)))
        }
    }

    pub async fn modify<F>(&self, key: K, f: F) -> ReactiveMapResult<()>
    where
        F: FnOnce(&mut V),
    {
        if let Some(mut val) = self.get(&key).await? {
            f(&mut val);
            self.set(key, &val).await
        } else {
            Err(Report::new(ReactiveMapError::KeyNotFound(key.to_string()))
                .attach(format!("map: {}", self.prefix)))
        }
    }

    pub async fn insert(&self, key: K, value: &V) -> ReactiveMapResult<()> {
        crate::map_insert_async(
            &self.backend,
            &self.core,
            self.prefix.clone(),
            key,
            value,
            Some(self.instance_id),
        )
        .await
    }

    pub async fn set(&self, key: K, value: &V) -> ReactiveMapResult<()> {
        crate::map_update_async(
            &self.backend,
            &self.core,
            self.prefix.clone(),
            key,
            value,
            Some(self.instance_id),
        )
        .await
    }

    /// Like `subscribe_any`, but skips values this handle rewrote itself.
    ///
    /// Only `Update` is filtered. A key appearing or disappearing - `Insert`,
    /// `Remove`, `Clear` - is delivered whoever caused it: that changes what
    /// the map holds rather than a value someone is editing, and a view
    /// listing the keys has to rebuild either way.
    ///
    /// One consequence worth knowing: `insert` comes back to you or not
    /// depending on whether the key was already there, since it is an `Insert`
    /// the first time and an `Update` after that.
    pub fn subscribe_any_external<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        let my_id = self.instance_id;
        self.core.subscribe_any(move |change| match change {
            MapChange::Update { source, .. } => {
                if *source != Some(my_id) {
                    callback(change);
                }
            }
            _ => callback(change),
        })
    }

    /// Like `subscribe_key`, but skips values this handle rewrote itself.
    ///
    /// Filters `Update` only, on the same reasoning as
    /// `subscribe_any_external`.
    pub fn subscribe_key_external<F>(&self, key: K, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        let my_id = self.instance_id;
        self.core.subscribe_key(key, move |change| match change {
            MapChange::Update { source, .. } => {
                if *source != Some(my_id) {
                    callback(change);
                }
            }
            _ => callback(change),
        })
    }

    pub async fn clear(&self) -> ReactiveMapResult<()> {
        crate::map_clear_async(
            &self.backend,
            &self.core,
            self.prefix.clone(),
            Some(self.instance_id),
        )
        .await
    }

    pub fn subscribe_any<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.core.subscribe_any(callback)
    }

    pub fn subscribe_key<F>(&self, key: K, callback: F) -> SignalSubscription
    where
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.core.subscribe_key(key, callback)
    }

    pub fn intercept<F>(&self, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        self.core.intercept(self.prefix.clone(), callback)
    }

    pub fn intercept_key<F>(&self, key: K, callback: F) -> InterceptDisposer
    where
        F: Fn(MapChange<K, V>) -> Option<MapChange<K, V>> + Send + Sync + 'static,
    {
        self.core.intercept_key(key, callback)
    }
}
