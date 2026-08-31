---
title: Dioxus
---


`amethystate-dioxus` bridges reactive state to Dioxus signals. Each field becomes a `ReadSignal` that re-renders only the components that read it. Handles are `Copy` arena-allocated indices, so they can be passed freely between components without cloning.

## Setup

```toml
[dependencies]
amethystate-dioxus = "*"
```

## Defining state

Add `#[amethystate_dioxus]` above `#[amethystate]` on each struct you want to use in components:

```rust
use amethystate_dioxus::{amethystate_dioxus, amethystate};

#[amethystate_dioxus]
#[amethystate(prefix = "settings")]
pub struct AppSettings {
    #[amestate(default = "Guest".to_string())]
    pub username: String,

    #[amestate(default = 0)]
    pub counter: i32,

    #[amestate(nested)]
    pub theme: Theme,
}

#[amethystate_dioxus]
#[amethystate]
pub struct Theme {
    #[amestate(default = "light".to_string())]
    pub mode: String,
}
```

## Provider

Wrap your app in `amethystateProvider` and pass the store:

```rust
#[component]
fn App() -> Element {
    let store = use_hook(|| {
        StoreBuilder::new("./app")
            .build()
            .expect("failed to open store")
    });

    rsx! {
        amethystateProvider {
            store,
            Settings {}
        }
    }
}
```

## Accessing state

`use_amethystate::<S>()` returns a `Handle<S>` — a `Copy` struct with a field for each state field. Pass it down to child components as a prop:

```rust
#[component]
fn Settings() -> Element {
    let state = use_amethystate::<AppSettings>();
    // state.username, state.counter, state.theme.mode — all Copy handles
}
```

`Handle<S>` is a type alias that hides the generated handle name. Declare props with it:

```rust
#[component]
fn ThemeEditor(settings: Handle<AppSettings>) -> Element {
    // ...
}
```

## Hooks

### use_field

Returns a `(ReadSignal<T>, Callback<T>)` pair for a writable field:

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

Returns a `ReadSignal<T>` for any field, with no setter beside it - for a value the component displays and never writes:

```rust
let host = use_read_only_field(state.host);
```

### use_map

Returns a `MapSignal<K, V>` for a writable `ReactiveMap` field. The signal holds the full snapshot and updates on any external change:

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

Subscribes to a single key in a `ReactiveMap`:

```rust
let entry = use_map_entry(state.env, "HTTP_PROXY".to_string());

rsx! { p { "{entry:?}" } }
```

## WASM / Tauri frontend

For Tauri apps with a Dioxus WASM frontend, the provider and setup differ. The backend is a `TauriBackend` instead of a local store, and slices are loaded asynchronously before the app renders.

Use `preload_slices!` to declare which slices to load:

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

`preload_slices!` suspends rendering until all slices are loaded from the Tauri backend. After that, `use_amethystate::<S>()` works the same as in the native case.

## Examples

- [`dioxus-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/dioxus-settings)