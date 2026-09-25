use crate::primitives::*;
use amethystate::client::{AsyncSubscriptionBackend, Field, ReactiveMap};
use amethystate::reactive::FieldValue;
use amethystate::{MapChange, ReactiveMapKey, ReactiveMapValue, SignalSubscription};
use amethystate_core::primitives::error::{ReactiveFieldResult, ReactiveMapResult};
use parking_lot::RwLock;
use serde::Deserialize;
use slotmap::{DefaultKey, SlotMap};
use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

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

    fn field<T: FieldValue>(&self, handle: FieldHandle<T>) -> Field<T, B> {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| field.clone())
    }

    fn map<K, V>(&self, handle: MapHandle<K, V>) -> ReactiveMap<K, V, B>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| map.clone())
    }

    pub fn register_field<T>(&self, field: Field<T, B>) -> FieldHandle<T>
    where
        T: FieldValue,
    {
        let key = self.storage.write().insert(Box::new(field));
        FieldHandle {
            key,
            _marker: PhantomData,
        }
    }

    pub fn get_field<T>(&self, handle: FieldHandle<T>) -> T
    where
        T: FieldValue,
    {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| field.value())
    }

    pub async fn set_field<T>(&self, handle: FieldHandle<T>, value: T) -> ReactiveFieldResult<()>
    where
        T: FieldValue,
    {
        self.field(handle).set(value).await
    }

    pub fn subscribe_external_field<T, F>(
        &self,
        handle: FieldHandle<T>,
        callback: F,
    ) -> SignalSubscription
    where
        T: FieldValue,
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| {
            field.subscribe_external(callback)
        })
    }

    pub fn subscribe_field<T, F>(&self, handle: FieldHandle<T>, callback: F) -> SignalSubscription
    where
        T: FieldValue,
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.with_item::<Field<T, B>, _, _>(handle.key, "Field", |field| field.subscribe(callback))
    }

    pub fn register_map<K, V>(&self, map: ReactiveMap<K, V, B>) -> MapHandle<K, V>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        let key = self.storage.write().insert(Box::new(map));
        MapHandle {
            key,
            _marker: PhantomData,
        }
    }

    pub fn get_map_entry<K, V>(&self, handle: MapHandle<K, V>, key: &K) -> Option<V>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        self.map(handle).get_sync(key).ok().flatten()
    }

    pub fn get_map_entries<K, V>(&self, handle: MapHandle<K, V>) -> Vec<(K, V)>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        self.map(handle).values().unwrap_or_default()
    }

    pub async fn set_map_entry<K, V>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
        value: V,
    ) -> ReactiveMapResult<()>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        self.map(handle).insert(key, &value).await
    }

    pub async fn remove_map_entry<K, V>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
    ) -> ReactiveMapResult<Option<V>>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        self.map(handle).remove(key).await
    }

    pub async fn clear_map<K, V>(&self, handle: MapHandle<K, V>) -> ReactiveMapResult<()>
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
    {
        self.map(handle).clear().await
    }

    pub fn subscribe_map_any<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_any(callback)
        })
    }

    pub fn subscribe_map_any_external<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_any_external(callback)
        })
    }

    pub fn subscribe_map_key<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_key(key, callback)
        })
    }

    pub fn subscribe_map_key_external<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey + for<'de> Deserialize<'de>,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V, B>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_key_external(key, callback)
        })
    }
}

impl<B: AsyncSubscriptionBackend> PartialEq for Arena<B> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }
}

impl<B: AsyncSubscriptionBackend> Eq for Arena<B> {}
