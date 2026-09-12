use crate::DefaultArena;
use std::collections::HashMap;
use std::hash::Hash;

#[cfg(not(target_arch = "wasm32"))]
pub trait Backend: amethystate::StoreBackend {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: amethystate::StoreBackend> Backend for T {}

#[cfg(target_arch = "wasm32")]
pub trait Backend: amethystate::client::AsyncSubscriptionBackend {}
#[cfg(target_arch = "wasm32")]
impl<T: amethystate::client::AsyncSubscriptionBackend> Backend for T {}

pub trait ReactiveBackend: 'static {
    type Callback<T: Send + Sync + 'static>: Copy + 'static;
    type ReadSignal<T: Send + Sync + 'static>: Copy + 'static;
    type Storage: Backend;

    fn cb_call<T: Send + Sync + 'static>(cb: &Self::Callback<T>, val: T);
    fn rs_get<T: Clone + Send + Sync + 'static>(rs: &Self::ReadSignal<T>) -> T;
}

pub trait AmeStateFrameworkNested {
    type Handle: Copy + Send + Sync + 'static;
    fn register(&self, arena: &DefaultArena) -> Self::Handle;
}

#[cfg(not(target_arch = "wasm32"))]
pub trait AmeStateFramework<B: ReactiveBackend>:
    amethystate::AmeStateSlice + AmeStateFrameworkNested
{
}

#[cfg(not(target_arch = "wasm32"))]
impl<B: ReactiveBackend, T: amethystate::AmeStateSlice + AmeStateFrameworkNested>
    AmeStateFramework<B> for T
{
}

#[cfg(target_arch = "wasm32")]
pub trait AmeStateFramework<B: ReactiveBackend>:
    amethystate::client::AmeStateSliceAsync<B::Storage> + AmeStateFrameworkNested
{
}

#[cfg(target_arch = "wasm32")]
impl<
    B: ReactiveBackend,
    T: amethystate::client::AmeStateSliceAsync<B::Storage> + AmeStateFrameworkNested,
> AmeStateFramework<B> for T
{
}

pub struct MapSignal<B, K, V>
where
    B: ReactiveBackend,
    K: Clone + Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub entries: B::ReadSignal<HashMap<K, V>>,
    _set: B::Callback<(K, V)>,
    _insert: B::Callback<(K, V)>,
    _remove: B::Callback<K>,
    _clear: B::Callback<()>,
}
impl<B, K, V> MapSignal<B, K, V>
where
    B: ReactiveBackend,
    K: Clone + Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn new(
        entries: B::ReadSignal<HashMap<K, V>>,
        set: B::Callback<(K, V)>,
        insert: B::Callback<(K, V)>,
        remove: B::Callback<K>,
        clear: B::Callback<()>,
    ) -> Self {
        Self {
            _set: set,
            _remove: remove,
            _clear: clear,
            entries,
            _insert: insert,
        }
    }
}
impl<B, K, V> Copy for MapSignal<B, K, V>
where
    B: ReactiveBackend,
    K: Clone + Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
}

impl<B, K, V> Clone for MapSignal<B, K, V>
where
    B: ReactiveBackend,
    K: Clone + Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<B, K, V> MapSignal<B, K, V>
where
    B: ReactiveBackend,
    K: Clone + Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn set(&self, key: K, val: V) {
        B::cb_call(&self._set, (key, val));
    }

    pub fn insert(&self, key: K, val: V) {
        B::cb_call(&self._insert, (key, val));
    }

    pub fn remove(&self, key: K) {
        B::cb_call(&self._remove, key);
    }

    pub fn clear(&self) {
        B::cb_call(&self._clear, ());
    }

    pub fn len(&self) -> usize {
        B::rs_get(&self.entries).len()
    }

    pub fn is_empty(&self) -> bool {
        B::rs_get(&self.entries).is_empty()
    }
}
