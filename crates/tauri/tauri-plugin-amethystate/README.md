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

`tauri-plugin-amethystate` bridges your `amethystate` state slices to the Tauri frontend. It exposes IPC commands for reading, writing, and subscribing to state, and ships a code generator that produces typed bindings — TypeScript or Rust for `wasm32` — from your Rust struct definitions.

## Installation

Add the plugin to your Tauri app's Rust crate:

```toml
# src-tauri/Cargo.toml
[dependencies]
tauri-plugin-amethystate = { version = "*", features = ["redb"] }
```

`amethystate` is re-exported as `tauri_plugin_amethystate::amethystate`, so no separate dependency is needed. The plugin's `redb`, `sqlite`, `json`, `toml` and `ron` features turn on the engine of the same name, and the store below opens only with one of them on.

Hand your store to the plugin in `main.rs`:

```rust
use tauri_plugin_amethystate::amethystate::StoreBuilder;

fn main() {
    let store = StoreBuilder::new("./app").build().unwrap();

    tauri::Builder::default()
        .plugin(tauri_plugin_amethystate::init(store))
        .run(tauri::generate_context!())
        .unwrap();
}
```

## Codegen

`amethystate-codegen`, re-exported here, walks every `#[amethystate]` struct in the process and writes typed bindings for the frontend. Collection happens inside a running process, so it runs from a binary in the crate that declares the structs, not from `build.rs`:

```rust
use your_crate_with_amethystate_types as _;

amethystate_codegen::amethystate_codegen_main!(
    rs_out = "../src/bindings/amethystate.rs",
    framework = leptos
);
```

`framework` is `leptos`, `yew`, `dioxus` or `vanilla`; the generated Rust compiles on `wasm32` only. A TypeScript frontend takes `ts_out` instead and gets its value types from `ts-rs` — see [TypeScript](https://uniproc-dev.github.io/amethystate/integrations/typescript/) in the book.

Complete apps: [`examples/tauri-typescript`](../../../examples/tauri-typescript), [`examples/tauri-leptos`](../../../examples/tauri-leptos), [`examples/tauri-yew`](../../../examples/tauri-yew).

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
| `amethystate:allow-amethystate-delete-prefix` | Delete every key under a prefix |
| `amethystate:allow-amethystate-scan-keys`   | List the keys under a prefix      |
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