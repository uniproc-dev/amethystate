<div align="center">

<img src="https://raw.githubusercontent.com/uniproc-dev/amethystate/master/logo.svg" alt="amethystate" width="384" />

# amethystate

[![Crates.io](https://img.shields.io/crates/v/amethystate.svg)](https://crates.io/crates/amethystate)
[![Docs.rs](https://docs.rs/amethystate/badge.svg)](https://docs.rs/amethystate)
[![CI](https://github.com/uniproc-dev/amethystate/actions/workflows/ci.yml/badge.svg)](https://github.com/uniproc-dev/amethystate/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![MSRV](https://img.shields.io/badge/MSRV-1.90-orange.svg)](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/)

*A state manager for Rust applications.*

</div>

`amethystate` is a state manager for Rust applications: state is declared as an ordinary struct, its fields are
reactive, and they outlive the program.

### Features

- **Struct-defined state** — one attribute turns a struct's fields into persisted reactive ones, with defaults, subscriptions, and interceptors that can refuse a write
- **Runtime-defined keys** — a map entry or a `Kv` path gets the same subscriptions, interceptors and durability as a declared field
- **Read and write every frame** — writes are buffered and batched, reads answer from memory
- **Durable when it matters** — `durable()` on a field, a map or a `Kv` path returns only once the value is on disk, for the writes that must not sit in a buffer
- **Behaviour you choose** — which engine holds the state, when a write reaches the disk, what a field does with a value that will not read, what a new version does to an old file
- **Migrations** — explicit versions, run at startup; drift is logged
- **Engines** — `redb`, `sqlite`, and text as `json`/`toml`/`ron` on disk; text files reload on external edits
- **In a browser** — `localStorage`, no Tauri needed; a page hears the writes another tab makes
- **[Integrations](https://uniproc-dev.github.io/amethystate/integrations/overview)** — Tauri (+TS bindings), Leptos, Dioxus, Yew, GPUI, egui/iced/ratatui
- **Tracing** — structured events, each write tagged with its source struct

> [!WARNING]
> **The framework adapters and about half of the engines are not used in any real application.**
> They run in CI and were smoke-tested, and they were built as a proof of concept for the
> architecture of the store: that one store can sit behind very different engines and GUI models.
> Nobody depends on them day to day, so expect rough edges there, and report what you hit.

<!-- shown: the readme's first example -->
```rust
use amethystate::{StoreBuilder, amethystate};

#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = StoreBuilder::new("./app").build()?;
    let state = NetworkState::new_with(&store)?;

    let _sub = state.port().subscribe(|port| println!("port → {port}"));

    state.port().set(9090)?;

    Ok(())
}
```
<!-- /shown -->

---

See the **[book](https://uniproc-dev.github.io/amethystate/introduction)** for full documentation — concepts, migrations, and per-framework integration guides.

### Compatibility
The minimum supported Rust version (MSRV) for `amethystate` is **1.90**.
