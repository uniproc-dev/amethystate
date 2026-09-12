use crate::primitives::*;
use amethystate::reactive::error::{ReactiveFieldResult, ReactiveMapResult};
use amethystate::{
    Field, MapChange, ReactiveMap, ReactiveMapKey, ReactiveMapValue, SignalSubscription,
};
use parking_lot::RwLock;
use serde::{Serialize, de::DeserializeOwned};
use slotmap::{DefaultKey, SlotMap};
use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

type ErasedItem = Box<dyn Any + Send + Sync>;

#[derive(Clone)]
pub struct Arena {
    storage: Arc<RwLock<SlotMap<DefaultKey, ErasedItem>>>,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(SlotMap::new())),
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

    pub fn register_field<T>(&self, field: Field<T>) -> FieldHandle<T>
    where
        T: Send + Sync + 'static,
    {
        let key = self.storage.write().insert(Box::new(field));
        FieldHandle {
            key,
            _marker: PhantomData,
        }
    }

    pub fn get_field<T>(&self, handle: FieldHandle<T>) -> T
    where
        T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
    {
        self.with_item::<Field<T>, _, _>(handle.key, "Field", |field| field.get())
    }

    pub fn set_field<T>(&self, handle: FieldHandle<T>, value: T) -> ReactiveFieldResult<()>
    where
        T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
    {
        self.with_item::<Field<T>, _, _>(handle.key, "Field", |field| field.set(value))
    }

    pub fn subscribe_external_field<T, F>(
        &self,
        handle: FieldHandle<T>,
        callback: F,
    ) -> SignalSubscription
    where
        T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.with_item::<Field<T>, _, _>(handle.key, "Field", |field| {
            field.subscription_with().external().register(callback)
        })
    }

    pub fn subscribe_field<T, F>(&self, handle: FieldHandle<T>, callback: F) -> SignalSubscription
    where
        T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
        F: for<'a> Fn(&'a T) + Send + Sync + 'static,
    {
        self.with_item::<Field<T>, _, _>(handle.key, "Field", |field| field.subscribe(callback))
    }
    pub fn register_map<K, V>(&self, map: ReactiveMap<K, V>) -> MapHandle<K, V>
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
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| map.get(key))
    }

    pub fn set_map_entry<K, V>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
        value: V,
    ) -> ReactiveMapResult<()>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| {
            map.insert(key, &value)
        })
    }

    pub fn subscribe_map_any<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_any(callback)
        })
    }

    pub fn subscribe_map_any_external<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscription_with().external().register(callback)
        })
    }

    pub fn subscribe_map_key_external<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscription_with()
                .key(key)
                .external()
                .register(callback)
        })
    }

    pub fn subscribe_map_key<K, V, F>(
        &self,
        handle: MapHandle<K, V>,
        key: K,
        callback: F,
    ) -> SignalSubscription
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
        F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| {
            map.subscribe_key(key, callback)
        })
    }

    pub fn get_map_entries<K, V>(&self, handle: MapHandle<K, V>) -> Vec<(K, V)>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| {
            map.entries().collect()
        })
    }

    pub fn remove_map_entry<K, V>(
        &self,
        handle: MapHandle<K, V>,
        key: &K,
    ) -> ReactiveMapResult<Option<V>>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| map.remove(key))
    }

    pub fn clear_map<K, V>(&self, handle: MapHandle<K, V>) -> ReactiveMapResult<()>
    where
        K: ReactiveMapKey,
        V: ReactiveMapValue,
    {
        self.with_item::<ReactiveMap<K, V>, _, _>(handle.key, "ReactiveMap", |map| map.clear())
    }
}

impl PartialEq for Arena {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }
}

impl Eq for Arena {}
