---
title: Tauri
---

`tauri-plugin-amethystate` is a Tauri v2 plugin that bridges your state slices to the Tauri frontend over IPC. It exposes commands for reading, writing, and subscribing to state, and ships a code generator that produces typed bindings for both TypeScript and Rust WASM frontends.

## Mental model

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (TypeScript or Rust/WASM)                         │
│  in-memory snapshot populated by load() and kept in         │
│  Field objects / structs                                    │
└────────────────────────┬────────────────────────────────────┘
                         │ IPC (Tauri commands)
┌────────────────────────▼────────────────────────────────────┐
│  amethystate (Rust)                                         │
│  in-memory write buffer, reactive subscriptions             │
└────────────────────────┬────────────────────────────────────┘
                         │ debounced flush / explicit save
┌────────────────────────▼────────────────────────────────────┐
│  Disk                                                       │
└─────────────────────────────────────────────────────────────┘
```

## Optimistic updates

The frontend API is synchronous by design — reads and writes return immediately without waiting for IPC confirmation. This means the frontend applies updates optimistically: the local value is updated first, and the IPC call to the backend follows asynchronously.

If the backend returns an error, the frontend value is reset to the last confirmed state. This tradeoff keeps the UI responsive but means a failed write will visibly revert.

## Installation

Add the plugin to your Tauri app's Rust crate:

```toml
# src-tauri/Cargo.toml
[dependencies]
tauri-plugin-amethystate = "0.20"
```

`amethystate` is re-exported as `tauri_plugin_amethystate::amethystate`, so no separate dependency is needed.

Register the plugin and your store in `main.rs`:

```rust
use tauri_plugin_amethystate::amethystate::StoreBuilder;

fn main() {
    let store = StoreBuilder::new("./app").build().unwrap();

    tauri::Builder::default()
        .manage(store)
        .plugin(tauri_plugin_amethystate::init())
        .run(tauri::generate_context!())
        .unwrap();
}
```

## Permissions

Add the default permission set to `src-tauri/capabilities/default.json`:

```json
{
  "permissions": [
    "amethystate:default"
  ]
}
```

`amethystate:default` includes the following permissions:

| Identifier | Description |
|------------|-------------|
| `amethystate:allow-amethystate-get` | Read a single key |
| `amethystate:allow-amethystate-set` | Write a single key |
| `amethystate:allow-amethystate-delete` | Delete a single key |
| `amethystate:allow-amethystate-subscribe` | Subscribe to key changes |
| `amethystate:allow-amethystate-unsubscribe` | Unsubscribe from a key |
| `amethystate:allow-amethystate-get-prefix` | Bulk-read all keys under a prefix |
| `amethystate:allow-amethystate-flush` | Flush pending writes to disk |

Every permission has a corresponding `deny-*` variant that takes priority over `allow-*`.

## Codegen

`amethystate-codegen` generates typed frontend bindings from your `#[amethystate]` structs. The binary must live in the same crate where your types are declared.

**1. Add the binary target and dependency:**

```toml
[[bin]]
name = "codegen"
path = "src/bin/codegen.rs"

[dependencies]
amethystate-codegen = { version = "0.20" }
```

For Rust WASM frontends, add the appropriate feature flag:

| Feature | Framework |
|---------|-----------|
| `leptos` | Leptos |
| `dioxus` | Dioxus |
| `yew` | Yew |
| *(none)* | TypeScript or vanilla WASM |

**2. Create `src/bin/codegen.rs`:**

For a TypeScript frontend:

```rust
#[allow(unused_imports)]
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    ts_out = "../src/bindings/amethystate.ts",
);
```

For a Rust WASM frontend:

```rust
#[allow(unused_imports)]
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    rs_out = "../src/bindings/amethystate.rs",
    framework = leptos
);
```

**3. Run:**

```sh
cargo run --bin codegen
```

## Examples

- [`tauri-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-settings) — TypeScript frontend
- [`tauri-leptos`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-leptos) — Leptos WASM frontend
- [`tauri-yew`](https://github.com/uniproc-dev/amethystate/tree/master/examples/tauri-yew) — Yew WASM frontend