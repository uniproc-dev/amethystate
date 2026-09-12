use crate::MapSignal;
use amethystate::{MapChange, ReactiveMapKey, ReactiveMapValue};
use amethystate_arena::{AmeStateFrameworkNested, DefaultArena, FieldHandle, MapHandle};
use dioxus::core::{Callback, spawn, use_hook};
use dioxus::hooks::{try_use_context, use_callback, use_context};
use dioxus::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

pub type Handle<S> = <S as AmeStateFrameworkNested>::Handle;

#[cfg(target_arch = "wasm32")]
pub fn use_amethystate<S>() -> S::Handle
where
    S: AmeStateFrameworkNested + 'static,
{
    if let Some(handle) = try_use_context::<S::Handle>() {
        return handle;
    }

    panic!(
        "amethystate-dioxus: State slice '{}' was not initialized! \
         Make sure to include it in preload_slices!(...) at the root AmeStateProvider.",
        std::any::type_name::<S>()
    );
}

#[cfg(not(target_arch = "wasm32"))]
pub fn use_amethystate<S>() -> S::Handle
where
    S: amethystate_arena::AmeStateFramework<crate::DioxusBackend> + 'static,
{
    if let Some(handle) = try_use_context::<S::Handle>() {
        return handle;
    }

    let store = try_use_context::<amethystate::Store>().unwrap_or_else(|| {
        panic!(
            "amethystate-dioxus: Store not found in context while trying to initialize '{}'. \
             Make sure AmeStateProvider is rendered at the root of your application.",
            std::any::type_name::<S>()
        );
    });
    let arena = try_use_context::<DefaultArena>().unwrap_or_else(|| {
        panic!(
            "amethystate-dioxus: DefaultArena not found in context while trying to initialize '{}'. \
             Make sure AmeStateProvider is rendered at the root of your application.",
            std::any::type_name::<S>()
        );
    });

    let handle = use_hook(|| {
        let state = S::load_slice(&store).unwrap_or_else(|err| {
            panic!(
                "amethystate-dioxus: Failed to load state slice '{}': {err}",
                std::any::type_name::<S>()
            );
        });
        state.register(&arena)
    });

    use_context_provider(|| handle);
    handle
}

pub fn use_field<T>(handle: FieldHandle<T>) -> (ReadSignal<T>, Callback<T>)
where
    T: DeserializeOwned + Serialize + Clone + Send + Sync + PartialEq + 'static,
{
    let arena = use_context::<DefaultArena>();
    let mut signal = use_signal(|| arena.get_field(handle));

    let tx = use_hook(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<T>();

        spawn(async move {
            while let Some(val) = rx.recv().await {
                signal.set(val);
            }
        });

        tx
    });

    let arena_clone = arena.clone();

    use_hook(move || {
        let sub = arena.subscribe_field(handle, move |val| {
            let _ = tx.send(val.clone());
        });
        Arc::new(sub)
    });

    let setter = use_callback(move |val: T| {
        let _ = arena_clone.set_field(handle, val);
    });

    (signal.into(), setter)
}

pub fn use_read_only_field<T>(handle: FieldHandle<T>) -> ReadSignal<T>
where
    T: DeserializeOwned + Serialize + Clone + Send + Sync + PartialEq + 'static,
{
    let arena = use_context::<DefaultArena>();
    let mut signal = use_signal(|| arena.get_field(handle));

    let tx = use_hook(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<T>();

        spawn(async move {
            while let Some(val) = rx.recv().await {
                signal.set(val);
            }
        });

        tx
    });

    use_hook(move || {
        let sub = arena.subscribe_field(handle, move |val| {
            let _ = tx.send(val.clone());
        });
        Arc::new(sub)
    });

    signal.into()
}

pub fn use_map<K, V>(handle: MapHandle<K, V>) -> MapSignal<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let arena = use_context::<DefaultArena>();
    let mut signal = use_signal(|| {
        arena
            .get_map_entries(handle)
            .into_iter()
            .collect::<HashMap<K, V>>()
    });

    let tx = use_hook(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<HashMap<K, V>>();
        spawn(async move {
            while let Some(val) = rx.recv().await {
                signal.set(val);
            }
        });
        tx
    });

    let arena_sub = arena.clone();
    use_hook(move || {
        let arena_sub_sub = arena_sub.clone();
        let sub = arena_sub.subscribe_map_any(handle, move |_| {
            let entries = arena_sub_sub.get_map_entries(handle).into_iter().collect();
            let _ = tx.send(entries);
        });
        Arc::new(sub)
    });

    let arena_set = arena.clone();
    let _set = use_callback(move |(key, val)| {
        let _ = arena_set.set_map_entry(handle, key, val);
    });

    let arena_insert = arena.clone();
    let _insert = use_callback(move |(key, val)| {
        let _ = arena_insert.set_map_entry(handle, key, val);
    });

    let arena_remove = arena.clone();
    let _remove = use_callback(move |key: K| {
        let _ = arena_remove.remove_map_entry(handle, &key);
    });

    let arena_clear = arena.clone();
    let _clear = use_callback(move |_| {
        let _ = arena_clear.clear_map(handle);
    });

    MapSignal::new(signal.into(), _set, _insert, _remove, _clear)
}

pub fn use_map_entry<K, V>(handle: MapHandle<K, V>, key: K) -> ReadSignal<Option<V>>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let arena = use_context::<DefaultArena>();
    let mut signal = use_signal(|| arena.get_map_entry(handle, &key));

    let tx = use_hook(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<V>>();

        spawn(async move {
            while let Some(val) = rx.recv().await {
                signal.set(val);
            }
        });

        tx
    });

    let key_clone = key.clone();
    use_hook(move || {
        let sub = arena.subscribe_map_key(handle, key_clone, move |change| match change {
            MapChange::Insert { value, .. }
            | MapChange::Update {
                new_value: value, ..
            } => {
                let _ = tx.send(Some(value.clone()));
            }
            MapChange::Remove { .. } | MapChange::Clear { .. } => {
                let _ = tx.send(None);
            }
        });
        Arc::new(sub)
    });

    signal.into()
}

pub fn use_map_subscribe_any<K, V, F>(handle: MapHandle<K, V>, callback: F)
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
    F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
{
    let arena = use_context::<DefaultArena>();
    use_hook(move || {
        let sub = arena.subscribe_map_any(handle, callback);
        Arc::new(sub)
    });
}

pub fn use_map_subscribe_key<K, V, F>(handle: MapHandle<K, V>, key: K, callback: F)
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
    F: Fn(&MapChange<K, V>) + Send + Sync + 'static,
{
    let arena = use_context::<DefaultArena>();
    use_hook(move || {
        let sub = arena.subscribe_map_key(handle, key, callback);
        Arc::new(sub)
    });
}
