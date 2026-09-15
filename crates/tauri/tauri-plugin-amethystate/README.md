<div align="center">

# tauri-plugin-amethystate

[![Crates.io](https://img.shields.io/crates/v/tauri-plugin-amethystate.svg)](https://crates.io/crates/tauri-plugin-amethystate)
[![Docs.rs](https://docs.rs/tauri-plugin-amethystate/badge.svg)](https://docs.rs/tauri-plugin-amethystate)
[![CI](https://github.com/uniproc-dev/amethystate/actions/workflows/ci.yml/badge.svg)](https://github.com/uniproc-dev/amethystate/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

*Tauri v2 plugin that exposes [amethystate](https://github.com/uniproc-dev/amethystate) reactive persistent state to the frontend, with TypeScript codegen.*

</div>

> **⚠️ Experimental**  
> This plugin is provided as-is. The API may change without notice and has not been hardened for production use. Feedback and contributions are welcome.

## Overview

`tauri-plugin-amethystate` bridges your `amethystate` state slices to the Tauri frontend. It exposes six IPC commands for reading, writing, and subscribing to state, and ships a runtime code generator that produces fully-typed TypeScript bindings from your Rust struct definitions — no manual type duplication.

## Installation

Add the plugin to your Tauri app's Rust crate:

```toml
# src-tauri/Cargo.toml
[dependencies]
tauri-plugin-amethystate = { version = "*", features = ["redb"] }
```

`amethystate` is re-exported as `tauri_plugin_amethystate::amethystate`, so no separate dependency is needed. The plugin's `redb`, `sqlite`, `json`, `toml` and `ron` features turn on the engine of the same name, and the store below opens only with one of them on.

Register the plugin and your store in `main.rs`:

```rust
use std::sync::Arc;
use tauri_plugin_amethystate::amethystate::StoreBuilder;

fn main() {
    let store = Arc::new(
        StoreBuilder::new("./app").build().unwrap()
    );

    tauri::Builder::default()
        .manage(store)
        .plugin(tauri_plugin_amethystate::init())
        .run(tauri::generate_context!())
        .unwrap();
}
```

## TypeScript codegen

The plugin ships `tauri_plugin_amethystate::backend::codegen::CodegenRegistry`, which walks all `#[amethystate]` structs registered via `inventory` and writes a fully-typed `.ts` file. Because collection happens inside a running process, it cannot be called from `build.rs` — use a test instead:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn export_bindings() {
        use tauri_plugin_amethystate::backend::codegen::CodegenRegistry;
        let reg = CodegenRegistry::new();

        reg.export_ts("../src/bindings/amethystate.ts")
            .expect("TS codegen failed");
    }
}
```

For a complete usage example see [`examples/tauri-settings`](../../../examples/tauri-settings).


## Rust codegen (WASM client)

For frontends written in Rust targeting `wasm32` (e.g. Leptos, Yew, Dioxus), the registry can emit typed Rust bindings instead of TypeScript:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn export_bindings() {
        use tauri_plugin_amethystate::backend::codegen::CodegenRegistry;
        let reg = CodegenRegistry::new();

        reg.export_rust("../src/bindings/amethystate.rs")
            .expect("Rust codegen failed");
    }
}
```

The generated file compiles on `wasm32` only. For a complete usage example see [`examples/tauri-leptos`](../../../examples/tauri-leptos).

## Mental model

There are three layers of state, each with its own representation:

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (TypeScript or Rust/WASM)                         │
│  in-memory snapshot populated by load() and kept in         │
│  Field objects / structs                                    │
└────────────────────────┬────────────────────────────────────┘
                         │ IPC (Tauri commands)
┌────────────────────────▼────────────────────────────────────┐
│  amethystate (Rust)                                             │
│  in-memory write buffer, reactive subscriptions             │
└────────────────────────┬────────────────────────────────────┘
                         │ debounced flush / explicit save
┌────────────────────────▼────────────────────────────────────┐
│  Disk (redb / json)                                         │
└─────────────────────────────────────────────────────────────┘
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

| Identifier                          | Description                       |
|-------------------------------------|-----------------------------------|
| `amethystate:allow-amethystate-get`         | Read a single key                 |
| `amethystate:allow-amethystate-set`         | Write a single key                |
| `amethystate:allow-amethystate-delete`      | Delete a single key               |
| `amethystate:allow-amethystate-subscribe`   | Subscribe to key changes          |
| `amethystate:allow-amethystate-unsubscribe` | Unsubscribe from a key            |
| `amethystate:allow-amethystate-get-prefix`  | Bulk-read all keys under a prefix |
| `amethystate:allow-amethystate-flush`       | Flush pending writes to disk      |

Every permission has a corresponding `deny-*` variant that takes priority over `allow-*`.

## Requirements

- Tauri **v2**
- amethystate **v0.x** (see [Cargo.toml](../../../Cargo.toml) for the exact workspace version)

## License

MIT — see [LICENSE](../../../LICENSE).