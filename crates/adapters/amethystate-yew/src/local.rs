use crate::MapSignal;
use amethystate::reactive::FieldValue;
use amethystate::{AmeStateSlice, Field, ReactiveMap, ReactiveMapKey, ReactiveMapValue, Store};
use futures::future::abortable;
use futures::{Stream, StreamExt};
use yew::platform::spawn_local;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct AmeStateProviderProps {
    pub store: Store,
    #[prop_or_default]
    pub children: Html,
}

#[function_component(AmeStateProvider)]
pub fn ame_state_provider(props: &AmeStateProviderProps) -> Html {
    html! {
        <ContextProvider<Store> context={props.store.clone()}>
            { props.children.clone() }
        </ContextProvider<Store>>
    }
}

fn follow<S>(changes: S, mut apply: impl FnMut() + 'static) -> impl FnOnce()
where
    S: Stream + Unpin + 'static,
{
    let (task, abort) = abortable(async move {
        let mut changes = changes;
        while changes.next().await.is_some() {
            apply();
        }
    });
    spawn_local(async move {
        let _ = task.await;
    });
    move || abort.abort()
}

#[hook]
pub fn use_read_only_field<T>(field: Field<T>) -> T
where
    T: FieldValue + PartialEq,
{
    let value = use_state_eq(|| field.get());

    {
        let value = value.clone();
        use_effect_with(field, move |field| {
            value.set(field.get());
            let source = field.clone();
            follow(field.subscription_with().stream(), move || {
                value.set(source.get())
            })
        });
    }

    (*value).clone()
}

#[hook]
pub fn use_field<T>(field: Field<T>) -> (T, Callback<T>)
where
    T: FieldValue + PartialEq,
{
    let value = use_read_only_field(field.clone());
    let setter = use_callback(field, |next: T, field| {
        let _ = field.set(next);
    });

    (value, setter)
}

#[hook]
pub fn use_map<K, V>(map: ReactiveMap<K, V>) -> MapSignal<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let entries = use_state(|| map.entries().collect::<Vec<_>>());

    {
        let entries = entries.clone();
        use_effect_with(map.clone(), move |map| {
            entries.set(map.entries().collect());
            let source = map.clone();
            follow(map.subscription_with().stream(), move || {
                entries.set(source.entries().collect())
            })
        });
    }

    let set = use_callback(map.clone(), |(key, value): (K, V), map| {
        let _ = map.insert(key, &value);
    });
    let insert = use_callback(map.clone(), |(key, value): (K, V), map| {
        let _ = map.insert(key, &value);
    });
    let remove = use_callback(map.clone(), |key: K, map| {
        let _ = map.remove(&key);
    });
    let clear = use_callback(map, |_: (), map| {
        let _ = map.clear();
    });

    MapSignal {
        entries: (*entries).clone(),
        set,
        insert,
        remove,
        clear,
    }
}

#[hook]
pub fn use_map_entry<K, V>(map: ReactiveMap<K, V>, key: K) -> Option<V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    let value = use_state(|| map.get(&key));

    {
        let value = value.clone();
        use_effect_with((map, key), move |(map, key)| {
            value.set(map.get(key));
            let (source, at) = (map.clone(), key.clone());
            follow(
                map.subscription_with().key(key.clone()).stream(),
                move || value.set(source.get(&at)),
            )
        });
    }

    (*value).clone()
}

#[hook]
pub fn use_amethystate<S>() -> S
where
    S: AmeStateSlice + Clone + 'static,
{
    let store = use_context::<Store>().unwrap_or_else(|| {
        panic!(
            "amethystate-yew: Store not found in context while loading '{}'. \
             Make sure AmeStateProvider is rendered at the root of your application.",
            std::any::type_name::<S>()
        )
    });

    let slice = use_memo(store, |store| {
        S::load_slice(store).unwrap_or_else(|err| {
            panic!(
                "amethystate-yew: failed to load state slice '{}': {err}",
                std::any::type_name::<S>()
            )
        })
    });

    (*slice).clone()
}
