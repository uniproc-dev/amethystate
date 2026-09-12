---
title: Yew
---

`amethystate-yew` bridges reactive state to Yew components. Each field is accessed via hooks returning native Yew types — `T` for reads and `Callback<T>` for writes. Components re-render only when the specific fields they read change.

*Note: This integration is designed for WASM frontends (CSR), primarily for Tauri applications.*

## Setup

```toml
[dependencies]
amethystate-yew = { version = "0.20", features = ["tauri"] }
```

## Defining state

The frontend structs are generated automatically from your backend types via `amethystate-codegen`. Do not write them by hand.

```rust
// src/bindings/amethystate.rs
// GENERATED AUTOMATICALLY. DO NOT EDIT.

#[::amethystate::amethystate(prefix = "settings", target = "tauri-wasm")]
pub struct AppSettings {
    pub username: String,
    pub counter: i32,
    #[amestate(nested)]
    pub theme: Theme,
    pub proxy: ProxyProfile,
    pub env: ReactiveMap<String, String>,
}

#[::amethystate::amethystate(target = "tauri-wasm")]
pub struct Theme {
    pub mode: String,
    pub background: String,
    pub foreground: String,
}
```

See the [Tauri integration](./tauri) chapter for codegen setup.

## Provider & Initialization

State is loaded asynchronously over IPC. Wrap your app in `AmeStateProvider` and declare which slices to load with `preload_slices!`. Rendering is suspended until all slices are ready.

```rust
use amethystate::tauri::TauriBackend;
use amethystate_yew::{preload_slices, AmeStateProvider};
use yew::prelude::*;

use crate::bindings::AppSettings;

#[function_component(App)]
pub fn app() -> Html {
    html! {
        <AmeStateProvider<TauriBackend>
            backend={TauriBackend::new()}
            init={preload_slices!(AppSettings)}
            fallback={html! { <p>{"Loading..."}</p> }}
        >
            <MainLayout />
        </AmeStateProvider<TauriBackend>>
    }
}
```

## Accessing state

Use `use_amethystate::<S>()` inside a component to get the root handle for a slice. The handle is a struct with a field for each state field — pass it down as a prop.

```rust
#[function_component(MainLayout)]
fn main_layout() -> Html {
    let state = use_amethystate::<AppSettings>();

    html! {
        <Settings state={state} />
    }
}

#[derive(Properties, PartialEq)]
pub struct SettingsProps {
    state: AppSettings,
}
```

Nested structs (marked `#[amestate(nested)]`) are passed by cloning their handle:

```rust
<ThemeEditor theme={props.state.theme.clone()} />
```

Plain struct fields (e.g. `ProxyProfile`) and `ReactiveMap` fields are wrapped in `Field<T>` and `ReactiveMap<K, V>` respectively, and are also passed by clone:

```rust
<ProxyEditor proxy={props.state.proxy.clone()} />
<EnvMapEditor env={props.state.env.clone()} />
```

## Hooks

### use_field

Returns a `(T, Callback<T>)` pair for a writable field. The setter applies the change optimistically on the frontend and persists it to the backend asynchronously. If the backend returns an error, the value is reset to the last confirmed state.

```rust
let (username, set_username) = use_field(props.state.username.clone());

html! {
    <input
        value={username}
        oninput={Callback::from(move |e: InputEvent| {
            let el: web_sys::HtmlInputElement = e.target_unchecked_into();
            set_username.emit(el.value());
        })}
    />
}
```

For plain struct fields, mutate and re-emit the whole value:

```rust
let (prof, set_prof) = use_field(props.proxy.clone());

// ...
let mut p = prof.clone();
p.name = el.value();
set_prof.emit(p);
```

### use_read_only_field

Returns a `T` for any field, with no setter beside it - for a value the component displays and never writes.

```rust
let host = use_read_only_field(state.host.clone());

html! {
    <p>{"Connected to: "}{host}</p>
}
```

### use_map

Returns a `MapSignal<K, V>` for a writable `ReactiveMap` field. The signal holds a snapshot of all entries and updates on any external change. It exposes `insert`, `set`, `remove`, and `clear` as direct methods.

```rust
#[derive(Properties, PartialEq)]
struct EnvMapProps {
    env: ReactiveMap<String, String>,
}

#[function_component(EnvMapEditor)]
fn env_map_editor(props: &EnvMapProps) -> Html {
    let map = use_map(props.env.clone());

    let on_add = {
        let map = map.clone();
        Callback::from(move |_| {
            map.insert("NEW_KEY".to_string(), "value".to_string());
        })
    };

    html! {
        <>
            { for map.entries.iter().map(|(k, v)| {
                let key = k.clone();
                let on_remove = {
                    let map = map.clone();
                    Callback::from(move |_| map.remove(key.clone()))
                };
                html! {
                    <div key={k.clone()}>
                        <code>{format!("{k} = {v}")}</code>
                        <button onclick={on_remove}>{"✕"}</button>
                    </div>
                }
            })}
            <button onclick={on_add}>{"Add Key"}</button>
        </>
    }
}
```

### use_map_entry

Subscribes to a single key in a `ReactiveMap`, returning an `Option<V>` that updates when that key changes.

```rust
let proxy_port = use_map_entry(state.env.clone(), "HTTP_PROXY".to_string());

html! {
    <p>{"Proxy Port: "}{proxy_port.unwrap_or_else(|| "Not set".into())}</p>
}
```

## Examples

- [`tauri-yew`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-yew) — Full Tauri v2 app with a Yew WASM frontend.