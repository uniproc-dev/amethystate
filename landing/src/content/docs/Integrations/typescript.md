---
title: Typescript
---

`amethystate` ships an npm package for Tauri apps whose frontend is TypeScript or JavaScript. Generated bindings are built on it: a class per state struct, holding a `Field<T>` per value and a `ReactiveMap<V>` per map.

## Installation

```sh
npm install amethystate
```

`@tauri-apps/api` is a peer dependency and must already be present in your project.

## Codegen

The classes come from the Rust declarations. The types of the values they hold come from [`ts-rs`](https://github.com/Aleph-Alpha/ts-rs): derive `TS` on the types a field or a map holds that are not primitives. Only those types reach the frontend, so only they need it.

**1. Add the binary target and the dependencies to your Tauri crate:**

```toml
# src-tauri/Cargo.toml
[[bin]]
name = "codegen"
path = "bin/codegen.rs"

[dependencies]
amethystate-codegen = "0.22"
ts-rs = "12"
```

**2. Derive `TS` on the value types:**

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct Todo {
    pub title: String,
    pub done: bool,
}
```

**3. Create `bin/codegen.rs`:**

```rust
use your_crate::Todo;
use ts_rs::{Config, TS};

fn main() {
    let beside_the_bindings = Config::new().with_out_dir("../src/bindings");
    Todo::export_all(&beside_the_bindings).expect("Todo would not export");

    amethystate_codegen::amethystate_codegen!(ts_out = "../src/bindings/amethystate.ts");
}
```

The bindings import each value type from a file of its own name beside them, which is where `ts-rs` writes it when it is given the same directory.

**4. Run:**

```sh
cargo run --bin codegen
```

## Defaults come from the backend

The frontend reads what the store holds and does not know the defaults declared in Rust. Build the struct on the Rust side before the frontend loads it: `Todos::new_with(&store)` writes the defaults of every field the store does not hold yet. `load()` refuses a field the store does not hold, and names its path.

## Loading

```ts
import { Todos } from "./bindings/amethystate";

const todos = await Todos.load();
```

`load()` reads everything under the slice's prefix in one IPC call. A nested struct becomes a class of its own, reached by property access. `dispose()` on the slice stops every watch its fields and maps took.

## Fields

```ts
todos.hideDone.get();
await todos.hideDone.set(true);
await todos.nextId.update((id) => id + 1);
```

A write is taken at once: `get()` and every subscriber see the new value before the store answers. A write the store refuses is taken back, the subscribers hear the old value again, and the promise rejects.

## Subscriptions

```ts
const stop = todos.hideDone.subscribe((hide) => render(hide));

stop();
```

A subscriber is called with the current value straight away, and again after every change. `get()` hands back the same value until something changes, and `subscribe` returns the function that stops it: the shape React's `useSyncExternalStore` and Svelte's store contract ask for.

## Maps

A map's keys are strings, and a key may hold any character: a dot in a key is part of one level, the way the store spells it.

```ts
todos.items.get("3");
todos.items.has("3");
todos.items.entries();

await todos.items.insert("3", { title: "milk", done: false });
await todos.items.update("3", { title: "milk", done: true });
await todos.items.remove("3");
await todos.items.clear();

const stopAll = todos.items.subscribe((entries) => render(entries));
const stopOne = todos.items.subscribeKey("3", (todo) => renderRow(todo));
const stopRaw = todos.items.onChange((change) => log(change));
```

`entries()` comes sorted by key, and is the same array until something changes. `insert` puts a value whether or not the key was there; `update` refuses a key the map does not hold. A key's subscriber is told `undefined` once the key is gone, whoever removed it. `onChange` hands over each `MapChange` as it happens, and nothing when it is subscribed.

Writes to a map are taken at once and taken back when refused, the way field writes are. A change the map made itself is not heard a second time when the store announces it.

## Flushing to disk

Writes are debounced in the backend. To have them on disk now, for example before the app closes, call `save()` on the slice:

```ts
await todos.save();
```

## Without Tauri

`load()` takes the store it talks to as an argument, and `tauri()` is the default. Anything implementing `Transport` can stand in for it, which is how the package's own tests run without a Tauri app.

## Examples

- [`tauri-typescript`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-typescript) — the todo app over the package
