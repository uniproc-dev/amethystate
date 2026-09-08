---
title: Leptos
---

`amethystate-leptos` соединяет реактивное состояние с сигналами Leptos. К каждому полю обращаются через хуки, возвращающие родные типы Leptos — `ReadSignal<T>` и `SignalSetter<T>`. Компоненты перерисовываются только тогда, когда меняются те поля, которые они читают.

Хендлы полей - это `Copy`-индексы, поэтому их можно передавать вниз через пропсы компонентов без клонирования и без времён жизни.

*Примечание: эта интеграция рассчитана на фронтенды на WASM (CSR), в первую очередь для приложений Tauri.*

## Установка

```toml
[dependencies]
amethystate-leptos = { version = "0.20", features = ["tauri"] }
```

## Объявление состояния

Структуры фронтенда генерируются автоматически из ваших типов бэкенда через `amethystate-codegen`. Не пишите их руками.

```rust
// src/bindings/amethystate.rs
// GENERATED AUTOMATICALLY. DO NOT EDIT.
use amethystate_arena::amethystate_framework_arena;

#[amethystate_framework_arena]
#[::amethystate::amethystate(prefix = "settings", target = "tauri-wasm")]
pub struct AppSettings {
    pub username: String,
    pub counter: i32,
    #[amestate(nested)]
    pub theme: Theme,
}
```

Настройка кодогенерации - в главе [Интеграция с Tauri](./tauri).

## Провайдер и инициализация

Состояние загружается асинхронно по IPC. Оберните приложение в `AmeStateProvider` и объявите через `preload_slices!`, какие срезы загружать. Отрисовка приостановлена, пока не готовы все срезы.

```rust
use amethystate::tauri::TauriBackend;
use amethystate_leptos::{AmeStateProvider, preload_slices};
use leptos::prelude::*;

use crate::bindings::AppSettings;

#[component]
pub fn App() -> impl IntoView {
    let backend = TauriBackend::new();

    view! {
        <AmeStateProvider
            backend=backend
            init=preload_slices!(AppSettings)
            fallback=|| view! { <p>"Loading state..."</p> }
        >
            <MainLayout />
        </AmeStateProvider>
    }
}
```

## Доступ к состоянию

Внутри компонента вызовите `use_amethystate::<S>()`, чтобы получить корневой хендл среза. Хендл - это `Copy`-структура с полем на каждое поле состояния, и её можно передавать вниз как пропсы.

```rust
#[component]
fn MainLayout() -> impl IntoView {
    let state = use_amethystate::<AppSettings>();

    view! {
        <SettingsEditor state=state />
    }
}

#[component]
fn SettingsEditor(state: Handle<AppSettings>) -> impl IntoView {
    // state.username, state.counter, state.theme — all Copy handles
}
```

`Handle<S>` - псевдоним для сгенерированного типа хендла. Используйте его в объявлениях пропсов.

## Хуки

### use_field

Возвращает пару `(ReadSignal<T>, SignalSetter<T>)` для поля, в которое можно писать. Сеттер применяет изменение оптимистично на фронтенде и асинхронно сохраняет его на бэкенде. Если бэкенд вернёт ошибку, значение сбрасывается к последнему подтверждённому состоянию.

```rust
let (username, set_username) = use_field(state.username);

view! {
    <input
        prop:value=username
        on:input=move |e| set_username.set(event_target_value(&e))
    />
}
```

### use_read_only_field

Возвращает `ReadSignal<T>` для любого поля, без сеттера рядом, - для значения, которое компонент показывает и никогда не пишет.

```rust
let host = use_read_only_field(state.host);

view! {
    <p>"Connected to: " {host}</p>
}
```

### use_map

Возвращает `MapSignal<K, V>` для поля `ReactiveMap`, в которое можно писать. Сигнал держит снимок всех записей и обновляется на любое внешнее изменение. Он также открывает `insert`, `remove` и `clear`.

```rust
let map = use_map(state.env);

let on_add = move |_| {
    map.insert("NEW_KEY".to_string(), "value".to_string());
};

let on_remove = Callback::new(move |key: String| {
    map.remove(key);
});

view! {
    <For
        each=move || map.entries.get()
        key=|(k, _)| k.clone()
        children=move |(k, v)| {
            let k_clone = k.clone();
            view! {
                <div>
                    <code>{k} " = " {v}</code>
                    <button on:click=move |_| on_remove.run(k_clone.clone())>"Remove"</button>
                </div>
            }
        }
    />
    <button on:click=on_add>"Add Key"</button>
}
```

### use_map_entry

Подписывается на один ключ в `ReactiveMap`, возвращая `ReadSignal<Option<V>>`.

```rust
let proxy_port = use_map_entry(state.env, "HTTP_PROXY".to_string());

view! {
    <p>"Proxy Port: " {move || proxy_port.get().unwrap_or_else(|| "Not set".into())}</p>
}
```

## Примеры

- [`tauri-leptos`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-leptos) — полноценное приложение Tauri v2 с фронтендом на Leptos и WASM.
