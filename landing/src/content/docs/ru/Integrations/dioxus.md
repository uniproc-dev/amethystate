---
title: Dioxus
---


`amethystate-dioxus` соединяет реактивное состояние с сигналами Dioxus. Каждое поле становится `ReadSignal`, который перерисовывает только те компоненты, которые его читают. Хендлы - это `Copy`-индексы, размещённые в арене, поэтому их можно свободно передавать между компонентами без клонирования.

## Установка

```toml
[dependencies]
amethystate-dioxus = "*"
```

## Объявление состояния

Поставьте `#[amethystate_framework_arena]` над `#[amethystate]` на каждой структуре, которую хотите использовать в компонентах:

```rust
use amethystate::amethystate;
use amethystate_macros_arena::amethystate_framework_arena;

#[amethystate_framework_arena]
#[amethystate(prefix = "settings")]
pub struct AppSettings {
    #[amestate(default = "Guest".to_string())]
    pub username: String,

    #[amestate(default = 0)]
    pub counter: i32,

    #[amestate(nested)]
    pub theme: Theme,
}

#[amethystate_framework_arena]
#[amethystate]
pub struct Theme {
    #[amestate(default = "light".to_string())]
    pub mode: String,
}
```

## Провайдер

Оберните приложение в `AmeStateProvider` и передайте хранилище:

```rust
#[component]
fn App() -> Element {
    let store = use_hook(|| {
        StoreBuilder::new("./app")
            .build()
            .expect("failed to open store")
    });

    rsx! {
        AmeStateProvider {
            store,
            Settings {}
        }
    }
}
```

## Доступ к состоянию

`use_amethystate::<S>()` возвращает `Handle<S>` — `Copy`-структуру с полем на каждое поле состояния. Передавайте её вниз, в дочерние компоненты, как проп:

```rust
#[component]
fn Settings() -> Element {
    let state = use_amethystate::<AppSettings>();
    // state.username, state.counter, state.theme.mode — all Copy handles
}
```

`Handle<S>` - псевдоним типа, который прячет сгенерированное имя хендла. Объявляйте пропсы через него:

```rust
#[component]
fn ThemeEditor(settings: Handle<AppSettings>) -> Element {
    // ...
}
```

## Хуки

### use_field

Возвращает пару `(ReadSignal<T>, Callback<T>)` для поля, в которое можно писать:

```rust
let (username, set_username) = use_field(state.username);

rsx! {
    input {
        value: "{username}",
        oninput: move |e| set_username(e.value()),
    }
}
```

### use_read_only_field

Возвращает `ReadSignal<T>` для любого поля, без сеттера рядом, - для значения, которое компонент показывает и никогда не пишет:

```rust
let host = use_read_only_field(state.host);
```

### use_map

Возвращает `MapSignal<K, V>` для поля `ReactiveMap`, в которое можно писать. Сигнал держит полный снимок и обновляется на любое внешнее изменение:

```rust
let map = use_map(state.env);

rsx! {
    for (k, v) in map.entries.read().clone() {
        div { "{k} = {v}" }
    }
    button { onclick: move |_| map.insert("KEY".into(), "value".into()), "Add" }
    button { onclick: move |_| map.remove("KEY".into()), "Remove" }
}
```

### use_map_entry

Подписывается на один ключ в `ReactiveMap`:

```rust
let entry = use_map_entry(state.env, "HTTP_PROXY".to_string());

rsx! { p { "{entry:?}" } }
```

## Фронтенд на WASM / Tauri

Для приложений Tauri с фронтендом на Dioxus и WASM провайдер и настройка отличаются. Бэкенд здесь `TauriBackend`, а не локальное хранилище, и срезы загружаются асинхронно до того, как приложение отрисуется.

Через `preload_slices!` объявляют, какие срезы загружать:

```rust
#[component]
fn App() -> Element {
    let backend = TauriBackend::new();

    rsx! {
        AmeStateProvider {
            backend,
            init: preload_slices!(AppSettings, Theme),
            Settings {}
        }
    }
}
```

`preload_slices!` приостанавливает отрисовку, пока все срезы не загрузятся из бэкенда Tauri. После этого `use_amethystate::<S>()` работает так же, как в нативном случае.

## Примеры

- [`dioxus-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/dioxus-settings)
