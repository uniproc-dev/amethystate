use crate::MapSignal;
use amethystate::MapChange;
use amethystate::client::{AsyncSubscriptionBackend, Field, ReactiveMap};
use amethystate::core::primitives::map_core::{ReactiveMapKey, ReactiveMapValue};
use amethystate::reactive::FieldValue;
use futures::channel::mpsc;
use serde::Deserialize;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

#[hook]
pub fn use_read_only_field<T, B>(field: Field<T, B>) -> T
where
    T: FieldValue + PartialEq,
    B: AsyncSubscriptionBackend,
{
    let value = use_state(|| field.value());

    {
        let value = value.clone();
        use_effect_with((), move |_| {
            let (tx, mut rx) = mpsc::unbounded::<T>();

            let sub = field.subscribe(move |val| {
                let _ = tx.unbounded_send(val.clone());
            });

            spawn_local(async move {
                use futures::StreamExt;
                while let Some(val) = rx.next().await {
                    value.set(val);
                }
            });

            move || drop(sub)
        });
    }

    (*value).clone()
}

#[hook]
pub fn use_field<T, B>(field: Field<T, B>) -> (T, Callback<T>)
where
    T: FieldValue + PartialEq,
    B: AsyncSubscriptionBackend,
{
    let value = use_state(|| field.value());

    {
        let value = value.clone();
        let field = field.clone();
        use_effect_with((), move |_| {
            let (tx, mut rx) = mpsc::unbounded::<T>();

            let sub = field.subscribe_external(move |val| {
                let _ = tx.unbounded_send(val.clone());
            });

            spawn_local(async move {
                use futures::StreamExt;
                while let Some(val) = rx.next().await {
                    value.set(val);
                }
            });

            move || drop(sub)
        });
    }

    let setter = {
        let value = value.clone();
        let field = field.clone();
        Callback::from(move |val: T| {
            let old = (*value).clone();
            value.set(val.clone());
            let field = field.clone();
            let value = value.clone();
            spawn_local(async move {
                if field.set(val).await.is_err() {
                    value.set(old);
                }
            });
        })
    };

    ((*value).clone(), setter)
}

#[hook]
pub fn use_map<K, V, B>(map: ReactiveMap<K, V, B>) -> MapSignal<K, V>
where
    K: ReactiveMapKey + for<'de> Deserialize<'de>,
    V: ReactiveMapValue,
    B: AsyncSubscriptionBackend,
{
    let state = use_state(|| map.values().unwrap_or_default());

    {
        let state = state.clone();
        let map_sub = map.clone();
        let map_vals = map.clone();
        use_effect_with((), move |_| {
            let (tx, mut rx) = mpsc::unbounded::<()>();

            let sub = map_sub.subscribe_any_external(move |_| {
                let _ = tx.unbounded_send(());
            });

            spawn_local(async move {
                use futures::StreamExt;
                while let Some(()) = rx.next().await {
                    if let Ok(entries) = map_vals.values() {
                        state.set(entries);
                    }
                }
            });

            move || drop(sub)
        });
    }

    let insert = {
        let state = state.clone();
        let map = map.clone();
        Callback::from(move |(key, val): (K, V)| {
            let old = (*state).clone();
            let mut next = old.clone();
            upsert(&mut next, key.clone(), val.clone());
            state.set(next);

            let map = map.clone();
            let state = state.clone();
            spawn_local(async move {
                if map.insert(key, &val).await.is_err() {
                    state.set(old);
                }
            });
        })
    };

    let set = {
        let state = state.clone();
        let map = map.clone();
        Callback::from(move |(key, val): (K, V)| {
            let old = (*state).clone();
            let mut next = old.clone();
            upsert(&mut next, key.clone(), val.clone());
            state.set(next);

            let map = map.clone();
            let state = state.clone();
            spawn_local(async move {
                if map.set(key, &val).await.is_err() {
                    state.set(old);
                }
            });
        })
    };

    let remove = {
        let state = state.clone();
        let map = map.clone();
        Callback::from(move |key: K| {
            let old = (*state).clone();
            let mut next = old.clone();
            next.retain(|(k, _)| k != &key);
            state.set(next);

            let map = map.clone();
            let state = state.clone();
            spawn_local(async move {
                if map.remove(key).await.is_err() {
                    state.set(old);
                }
            });
        })
    };

    let clear = {
        let state = state.clone();
        let map = map.clone();
        Callback::from(move |_: ()| {
            let old = (*state).clone();
            state.set(Vec::new());

            let map = map.clone();
            let state = state.clone();
            spawn_local(async move {
                if map.clear().await.is_err() {
                    state.set(old);
                }
            });
        })
    };

    MapSignal {
        entries: (*state).clone(),
        set,
        remove,
        clear,
        insert,
    }
}

#[hook]
pub fn use_map_entry<K, V, B>(map: ReactiveMap<K, V, B>, key: K) -> Option<V>
where
    K: ReactiveMapKey + PartialEq + for<'de> Deserialize<'de>,
    V: ReactiveMapValue,
    B: AsyncSubscriptionBackend,
{
    let value = use_state(|| map.get_sync(&key).ok().flatten());

    {
        let value = value.clone();
        let map_clone = map.clone();
        use_effect_with((), move |_| {
            let (tx, mut rx) = mpsc::unbounded::<Option<V>>();

            let sub = map_clone.subscribe_key_external(key, move |change| {
                let val = match change {
                    MapChange::Insert { value: v, .. } | MapChange::Update { new_value: v, .. } => {
                        Some(v.clone())
                    }
                    MapChange::Remove { .. } | MapChange::Clear { .. } => None,
                };
                let _ = tx.unbounded_send(val.clone());
            });

            spawn_local(async move {
                use futures::StreamExt;
                while let Some(val) = rx.next().await {
                    value.set(val);
                }
            });

            move || drop(sub)
        });
    }

    (*value).clone()
}

#[hook]
pub fn use_amethystate<S>() -> S
where
    S: Clone + PartialEq + 'static,
{
    use_context::<S>().unwrap_or_else(|| {
        panic!(
            "amethystate-yew: State slice '{}' not found in context. \
             Did you render AmeStateProvider and include it in provide_slices!?",
            std::any::type_name::<S>()
        )
    })
}

fn upsert<K: ReactiveMapKey, V: ReactiveMapValue>(entries: &mut Vec<(K, V)>, key: K, value: V) {
    match entries.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = value,
        None => {
            let at = entries
                .iter()
                .position(|(k, _)| k.to_string() > key.to_string())
                .unwrap_or(entries.len());
            entries.insert(at, (key, value));
        }
    }
}
