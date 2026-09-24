---
title: egui / iced / ratatui
---

These frameworks have an explicit event loop — either redrawing every frame or routing all changes through a message cycle. There is no natural place for subscriptions, so `amethystate` is used in persistent-only mode: load state on startup, read and mutate fields directly, flush when needed.

## Pattern

```rust
#[amethystate(prefix = "app", mode = "persistent")]
pub struct AppState {
    #[amestate(default = 800u32)]
    pub window_width: u32,

    #[amestate(default = "dark".to_string())]
    pub theme: String,
}
```

```rust
let mut state = AppState::load()?;

// read directly
println!("{}", state.theme);

// mutate and flush
state.mutate_lazy(|s| {
    s.theme = "light".to_string();
})?;
```

External changes — another process writing to the same store, a file edited manually — will not be reflected in the loaded struct. If you need that, use reactive mode and call `.get()` at the start of each frame to poll the latest value.

## Examples

- [`egui`](https://github.com/uniproc-dev/amethystate/tree/master/examples/egui)
- [`iced`](https://github.com/uniproc-dev/amethystate/tree/master/examples/iced)
- [`ratatui`](https://github.com/uniproc-dev/amethystate/tree/master/examples/ratatui)