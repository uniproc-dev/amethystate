---
title: Quick Start
sidebar:
  order: 3
---

The shortest path from nothing to a running store, with a pointer at each step
to the section that covers it properly.

## Declare the state

One attribute turns a struct's fields into persisted reactive ones. `prefix`
says where in the store they live.

<!-- shown: declaring a state struct -->
```rust
use amethystate::amethystate;

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
```
<!-- /shown -->

Defaults, nested structs, volatile fields, `AmeType`, read policies, and serde
interaction: [Defining structs](/amethystate/state/defining-structs/).

## Open the store

<!-- shown: opening a store you hold yourself -->
```rust
let store = StoreBuilder::new(settings)
    .disk(|d| d.debounce(Duration::from_millis(500)))
    .build()?;

let state = NetworkState::new_with(&store)?;
```
<!-- /shown -->

`new_with` takes a store you hold. Opening one for the whole process instead,
letting the platform decide where the file goes, what the extension becomes,
and how closing reports a failure: [Opening a store](/amethystate/store/opening/).

Timing, retries and what the store refuses to hold:
[Configuring a store](/amethystate/store/configuration/). Which engine holds
the file: [Installation](/amethystate/getting-started/installation/).

## Read, write, subscribe

<!-- shown: reading, writing and subscribing -->
```rust
println!("{}", state.host().get());

let _sub = state.port().subscribe(|port| {
    println!("port changed to {port}");
});

state.port().set(9090)?;
```
<!-- /shown -->

A write reaches memory at once and the disk on the debounce. Delivering
callbacks on your own thread, filtering out your own writes, and what a
subscription costs: [Subscriptions](/amethystate/concepts/subscriptions/).

To wait for the disk instead of the debounce:
[Durability](/amethystate/concepts/durability/).

## Keys you do not know at compile time

A map stores each entry at its own path, so entries can be added and observed
one at a time.

<!-- shown: a map whose keys are not known up front -->
```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, AmeType)]
pub struct AlertThresholds {
    pub warning: u64,
    pub critical: u64,
}

#[amethystate(prefix = "sys")]
pub struct SystemSettings {
    #[amestate(default = {
        "cpu": AlertThresholds { warning: 70, critical: 90 },
        "mem": AlertThresholds { warning: 80, critical: 95 }
    })]
    pub limits: ReactiveMap<String, AlertThresholds>,
}
```
<!-- /shown -->

<!-- shown: working with a map -->
```rust
state.limits().insert(
    "gpu".to_string(),
    &AlertThresholds {
        warning: 60,
        critical: 85,
    },
)?;

let cpu = state.limits().get("cpu");

for (key, value) in state.limits().entries() {
    println!("{key}: {value:?}");
}

let _sub = state.limits().subscribe_any(|change| {
    println!("{change:?}");
});
```
<!-- /shown -->

`entries()` walks in the store's own order, over the key's string form - so
numeric keys come back `10, 100, 9`.

For paths decided entirely at run time, with no struct at all:
[Kv](/amethystate/primitives/kv/). For a single value addressed by path:
[ReactiveCell](/amethystate/primitives/reactive-cell/).

## When the struct changes

Bumping a struct's `version` and declaring the steps between versions is how
data written by an older build is brought forward. A field renamed or retyped
*without* a bump is reported as drift and startup continues.

Open the store with `build_with_migration` whenever `#[migrate]` is in the binary:
`build` runs only the migrations declared by hand.

[Migrations](/amethystate/migrations/overview/).

## Persistent-only mode

For frameworks that own their update loop - egui, iced, ratatui - `mode =
"persistent"` makes the fields plain Rust values on a plain struct, saved when
you say so. Nothing is reactive, and the struct does not see changes made
elsewhere.

<!-- shown: a struct in persistent mode -->
```rust
#[amethystate(prefix = "kept", mode = "persistent")]
pub struct KeptSettings {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
```
<!-- /shown -->

<!-- shown: writing a persistent struct -->
```rust
let mut state = KeptSettings::load_with(&store)?;

state.port = 9090;
state.save()?;

state.mutate(|d| {
    d.host = "0.0.0.0".to_string();
    d.port = 443;
})?;
```
<!-- /shown -->

`save_lazy` and `mutate_lazy` are the same two writes with the flush left to
the debouncer.

## What else there is

- **Interceptors** - a callback that sees a write before it lands and may
  rewrite or refuse it: [Subscriptions](/amethystate/concepts/subscriptions/).
- **Tracing** - structured events, each write tagged with the struct that made
  it: [Observability](/amethystate/concepts/observability/).
- **Framework integrations** - Tauri with TypeScript bindings, Leptos, Dioxus,
  Yew, GPUI, windows-reactor:
  [Integrations](/amethystate/integrations/overview/).
