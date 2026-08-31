---
title: Leptos
---

`amethystate-leptos` bridges reactive state to Leptos signals. Each field is accessed via hooks that return native Leptos types — `ReadSignal<T>` and `SignalSetter<T>`. Components re-render only when the specific fields they read change.

Field handles are `Copy` indices, so they can be passed down through component props without cloning or lifetimes.

*Note: This integration is designed for WASM frontends (CSR), primarily for Tauri applications.*

## Setup

```toml
[dependencies]
amethystate-leptos = { version = "0.20", features = ["tauri"] }
```

## Defining state

The frontend structs are generated automatically from your backend types via `amethystate-codegen`. Do not write them by hand.

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

See the [Tauri integration](./tauri) chapter for codegen setup.

## Provider & Initialization

State is loaded asynchronously over IPC. Wrap your app in `amethystateProvider` and declare which slices to load with `preload_slices!`. Rendering is suspended until all slices are ready.

```rust
use amethystate::tauri::TauriBackend;
use amethystate_leptos::{amethystateProvider, preload_slices};
use leptos::prelude::*;

use crate::bindings::AppSettings;

#[component]
pub fn App() -> impl IntoView {
    let backend = TauriBackend::new();

    view! {
        <amethystateProvider
            backend=backend
            init=preload_slices!(AppSettings)
            fallback=|| view! { <p>"Loading state..."</p> }
        >
            <MainLayout />
        </amethystateProvider>
    }
}
```

## Accessing state

Use `use_amethystate::<S>()` inside a component to get the root handle for a slice. The handle is a `Copy` struct with a field for each state field, which you can pass down as props.

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

`Handle<S>` is a type alias for the generated handle type. Use it in prop declarations.

## Hooks

### use_field

Returns a `(ReadSignal<T>, SignalSetter<T>)` pair for a writable field. The setter applies the change optimistically on the frontend and persists it to the backend asynchronously. If the backend returns an error, the value is reset to the last confirmed state.

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

Returns a `ReadSignal<T>` for any field, with no setter beside it - for a value the component displays and never writes.

```rust
let host = use_read_only_field(state.host);

view! {
    <p>"Connected to: " {host}</p>
}
```

### use_map

Returns a `MapSignal<K, V>` for a writable `ReactiveMap` field. The signal holds a snapshot of all entries and updates on any external change. It also exposes `insert`, `remove`, and `clear`.

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

Subscribes to a single key in a `ReactiveMap`, returning a `ReadSignal<Option<V>>`.

```rust
let proxy_port = use_map_entry(state.env, "HTTP_PROXY".to_string());

view! {
    <p>"Proxy Port: " {move || proxy_port.get().unwrap_or_else(|| "Not set".into())}</p>
}
```

## Examples

- [`tauri-leptos`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-leptos) — Full Tauri v2 app with a Leptos WASM frontend.