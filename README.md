<div align="center">

<img src="https://raw.githubusercontent.com/uniproc-dev/amethystate/master/logo.svg" alt="amethystate" width="384" />

# amethystate

[![Crates.io](https://img.shields.io/crates/v/amethystate.svg)](https://crates.io/crates/amethystate)
[![Docs.rs](https://docs.rs/amethystate/badge.svg)](https://docs.rs/amethystate)
[![CI](https://github.com/uniproc-dev/amethystate/actions/workflows/ci.yml/badge.svg)](https://github.com/uniproc-dev/amethystate/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![MSRV](https://img.shields.io/badge/MSRV-1.90-orange.svg)](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/)

*Persistent reactive state for Rust GUI apps.*

</div>

Every Rust GUI project builds the same persistence layer from scratch. It starts with a struct, `serde`, and `confy` —
or just the same boilerplate written by hand. Then the app grows: schema changes get mixed into validation logic,
a file watcher gets bolted on so settings reload without a restart, versioning becomes a fragile enum that guesses at
the data's shape.

`amethystate` is that layer, built once. Fields persist automatically, fire subscriptions on change, and flush to disk
in the background. Schema versions are explicit, migrations run on startup, and drift is detected and logged.

### Features

- **Struct-defined state** — one attribute turns a struct's fields into persisted reactive ones, with defaults, subscriptions, and interceptors that can refuse a write
- **Runtime-defined keys** — a map entry or a `Kv` path gets the same subscriptions, interceptors and durability as a declared field
- **Read and write every frame** — writes are buffered and batched, reads answer from memory
- **Migrations** — explicit versions, run at startup; drift is logged
- **Three backends, five formats** — `redb`, `sqlite`, and text as `json`/`toml`/`ron`; text files reload on external edits
- **[Integrations](https://uniproc-dev.github.io/amethystate/integrations/overview)** — Tauri (+TS bindings), Leptos, Dioxus, Yew, GPUI, windows-reactor, egui/iced/ratatui
- **Tracing** — structured events, each write tagged with its source struct

```rust
#[amethystate(prefix = "network")]
pub struct NetworkState {
    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,

    #[amestate(default = 8080)]
    pub port: u16,
}

fn main() -> amethystate::Result<()> {
    let store = StoreBuilder::new("./app").build()?;
    let state = NetworkState::new_with(&store)?;

    let _sub = state.port().subscribe(|p| println!("port → {p}"));
    
    state.port().set(9090)?;

    Ok(())
}
```

---

See the **[book](https://uniproc-dev.github.io/amethystate/introduction)** for full documentation — concepts, migrations, and per-framework integration guides.

### Compatibility
The minimum supported Rust version (MSRV) for `amethystate` is **1.90**.
