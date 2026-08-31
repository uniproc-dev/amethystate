use crate::primitives::*;
use amethystate::{AccessMode, MapChange, SignalSubscription, WritableMode};
use parking_lot::RwLock;

use amethystate::client::{AsyncSubscriptionBackend, Field, ReactiveMap};
use amethystate::core::error::{ReactiveFieldResult, ReactiveMapResult};
use amethystate::reactive::FieldValue;
use amethystate::{ReactiveMapKey, ReactiveMapValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use slotmap::{DefaultKey, SlotMap};
use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;
use uuid::Uuid;

type ErasedItem = Box<dyn Any + Send + Sync>;

#[derive(Clone)]
pub struct Arena<B: AsyncSubscriptionBackend> {
    storage: Arc<RwLock<SlotMap<DefaultKey, ErasedItem>>>,
    _backend: PhantomData<B>,
}

impl<B: AsyncSubscriptionBackend> Default for Arena<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: AsyncSubscriptionBackend> Arena<B> {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(SlotMap::new())),
            _backend: PhantomData,
        }
    }

    pub fn with_item<Item, R, F>(&self, key: DefaultKey, type_name: &str, f: F) -> R
    where
        Item: Any,
        F: FnOnce(&Item) -> R,
    {
        let storage = self.storage.read();
        let item = storage.get(key).unwrap_or_else(|| {
            panic!(
                "amethystate-arena: Attempted to access a dropped {}",
                type_name
            )
        });
        let target = item
            .downcast_ref::<Item>()
            .unwrap_or_else(|| panic!("amethystate-arena: Type mismatch for {}", type_name));
        f(target)
    }

    pub fn register_field<T>(&self, field: Field<T, B>) -> FieldHandle<T, WritableMode>
    where
        T: FieldValue,
    {
        let key = self.storage.write().insert(Box::new(field));
        FieldHandle {
            key,
            _marker: PhantomData,
        }
    }

    pub fn get_field<T, M>(&self, handle: FieldHandle<T, M>) -> T
    where
        T: FieldValue,
    {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| field.value())
    }

    pub async fn set_field<T>(
        &self,
        handle: WritableHandle<T>,
        value: T,
    ) -> ReactiveFieldResult<(), B::Error>
    where
        T: FieldValue,
    {
        let field = self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |f| f.clone());
        field.set(value).await
    }

    pub fn subscribe_external_field<T, M, F>(
        &self,
        handle: FieldHandle<T, M>,
        callback: F,
    ) -> SignalSubscription
    where
        T: FieldValue,
        M: AccessMode,
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| {
            field.subscription_with().external().register(callback)
        })
    }

    pub fn subscribe_field<T, M, F>(
        &self,
        handle: FieldHandle<T, M>,
        callback: F,
    ) -> SignalSubscription
    where
        T: FieldValue,
        M: AccessMode,
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| field.subscribe(callback))
    }

    pub fn register_map<K, V, M>(&self, map: ReactiveMap<K, V, B>) -> MapHandle<K, V, M>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
        M: AccessMode,
    {
        let key = self.storage.write().insert(Box::new(map));
        MapHandle {
            key,
            _marker: PhantomData,
        }
    }

    pub fn get_map_entry<K, V, M>(
        &self,
        handle: MapHandle<K, V, M>,
        key: &K,
    ) -> ReactiveMapResult<Option<V>, B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.get_sync(key)
    }

    pub async fn get_map_entry_async<K, V>(
        &self,
        handle: MapHandle<K, V>,
        key: &K,
    ) -> ReactiveMapResult<Option<V>, B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.get(key).await
    }

    pub async fn set_map_entry<K, V>(
        &self,
        handle: WritableMapHandle<K, V>,
        key: K,
        value: V,
    ) -> ReactiveMapResult<(), B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.set(key, &value).await
    }

    pub fn subscribe_map_any<K, V, F, M>(
        &self,
        handle: MapHandle<K, V, M>,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_any(callback)
        })
    }

    pub fn subscribe_map_any_external<K, V, M, F>(
        &self,
        handle: MapHandle<K, V, M>,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscription_with().external().register(callback)
        })
    }

    pub fn subscribe_map_key_external<K, V, F, M>(
        &self,
        handle: MapHandle<K, V, M>,
        key: K,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscription_with()
                .key(key)
                .external()
                .register(callback)
        })
    }

    pub fn subscribe_map_key<K, V, F, M>(
        &self,
        handle: MapHandle<K, V, M>,
        key: K,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_key(key, callback)
        })
    }

    pub async fn get_map_entries_async<K, V, M>(
        &self,
        handle: MapHandle<K, V, M>,
    ) -> ReactiveMapResult<Vec<(K, V)>, B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.entries().await.map(|hm| hm.into_iter().collect())
    }

    pub fn get_map_entries<K, V, M>(
        &self,
        handle: MapHandle<K, V, M>,
    ) -> ReactiveMapResult<Vec<(K, V)>, B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        M: AccessMode,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.values().map(|hm| hm.into_iter().collect())
    }

    pub async fn remove_map_entry<K, V>(
        &self,
        handle: WritableMapHandle<K, V>,
        key: K,
    ) -> ReactiveMapResult<Option<V>, B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.remove(key).await
    }

    pub async fn clear_map<K, V>(
        &self,
        handle: WritableMapHandle<K, V>,
    ) -> ReactiveMapResult<(), B::Error>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        let map =
            self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |m| m.clone());
        map.clear().await
    }
}

impl<B: AsyncSubscriptionBackend> PartialEq for Arena<B> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }
}

impl<B: AsyncSubscriptionBackend> Eq for Arena<B> {}
