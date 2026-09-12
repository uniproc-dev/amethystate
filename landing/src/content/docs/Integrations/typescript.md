---
title: Typescript
---

`amethystate` provides a TypeScript package for Tauri apps with a plain TypeScript or JavaScript frontend. The package ships `ReactiveField<T>`, `ReadonlyReactiveField<T>`, and `ReactiveMap<K, V>` — the primitive classes that generated bindings are built on top of — plus the `MapChange<K, V>` union that map subscriptions deliver.

## Installation

```sh
npm install amethystate
```

`@tauri-apps/api` is a peer dependency and must already be present in your project.

## Codegen

Generated bindings are a single TypeScript file that imports from `amethystate` and exposes typed classes for each of your state slices.

**1. Add the binary target and dependency to your Tauri crate:**

```toml
# src-tauri/Cargo.toml
[[bin]]
name = "codegen"
path = "src/bin/codegen.rs"

[dependencies]
amethystate-codegen = { version = "0.20" }
```

**2. Create `src/bin/codegen.rs`:**

```rust
#[allow(unused_imports)]
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    ts_out = "../src/bindings/amethystate.ts"
);
```

**3. Run:**

```sh
cargo run --bin codegen
```

## Using generated bindings

Each root struct becomes a class with a static `load()` method. Call it once on startup before rendering your UI.

```ts
import { AppSettings } from "./bindings/amethystate";

const settings = await AppSettings.load();
```

`load()` bulk-reads all keys under the slice's prefix over a single IPC call and wires up subscriptions so the local cache stays in sync with the backend.

A nested struct becomes its own class holding that struct's fields, reached by property access:

```ts
settings.theme.mode.value = "dark";
```

## Reading and writing fields

A plain field is a `ReactiveField<T>` instance with two access patterns:

```ts
// synchronous — reads from the local in-memory cache
const name = settings.username.value;

// optimistic write — updates cache immediately, persists asynchronously
settings.username.value = "Alice";

// async — reads directly from the persistent store (transaction-safe)
const storedName = await settings.username.get();

// async write — queues a write to the store
await settings.username.set("Alice");
```

`value` getter/setter is the typical choice for UI bindings. Use the async methods when you need a guarantee that the value is consistent with the backend, or want explicit control over when the write is queued.

The `value` getter is typed `T | null`. It reads `null` until the first value arrives, which for a key present in the store is already done by the time `load()` resolves.

`ReadonlyReactiveField<T>` is the same class without the `value` setter and without `set()`.

## Subscriptions

```ts
const unsubscribe = settings.username.subscribe((val) => {
    console.log("username changed:", val);
});

// later
unsubscribe();
```

The returned function may itself return a promise. Await it when you need the backend unsubscribe to have completed before continuing.

## Flushing to disk

Writes are debounced in the background. To guarantee immediate persistence — for example before the app closes — call `save()` on the slice:

```ts
await settings.save();
```

## ReactiveMap

A map field is a `ReactiveMap<K, V>` instance. `K` is constrained to `string`. It exposes synchronous and async access, deletion, and subscriptions per-key or for the entire map:

```ts
// async
await settings.env.set("HTTP_PROXY", "http://localhost:8080");
const proxy = await settings.env.get("HTTP_PROXY");

// synchronous (in-memory cache)
settings.env.setSync("HTTP_PROXY", "http://localhost:8080");
const cachedProxy = settings.env.getSync("HTTP_PROXY");
const hasProxy = settings.env.hasSync("HTTP_PROXY");

// iterate current entries
for (const [key, val] of settings.env.entries) {
    console.log(key, val);
}

// async delete — resolves once the backend confirms
await settings.env.remove("HTTP_PROXY");

// optimistic delete — drops the cache entry, deletes in the background
settings.env.removeSync("HTTP_PROXY");

// subscribe to any change
const unsubAny = settings.env.subscribeAny((change) => {
    if (change.type === "Insert") { /* change.key, change.value */ }
    if (change.type === "Update") { /* change.key, change.oldValue, change.newValue */ }
    if (change.type === "Remove") { /* change.key, change.oldValue */ }
    if (change.type === "Clear")  { /* no payload */ }
});

// subscribe to a specific key
const unsubKey = settings.env.subscribeKey("HTTP_PROXY", (val) => {
    console.log("proxy changed:", val);
});
```

Both `get()` and `getSync()` return `V | null` for a key the map does not hold. `entries` is a `ReadonlyMap<K, V>` over the local cache.

## Cleanup

Every field and map registers a subscription when it is constructed. Call `destroy()` on each one to unregister it:

```ts
settings.username.destroy();
settings.theme.mode.destroy();
settings.env.destroy();
```

A generated slice class exposes `load()` and `save()`, so cleanup is per field rather than per slice.

## Examples

- [`tauri-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-settings) — TypeScript frontend