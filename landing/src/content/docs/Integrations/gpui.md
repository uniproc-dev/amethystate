---
title: GPUI
---


GPUI uses an entity model with deferred notification — mutations happen inside entity update closures, and the framework notifies dependents after the closure returns. Synchronous `Field<T>` subscriptions don't compose with this model directly, so the integration bridges them through an async channel.

## How it works

`amethystate-gpui` provides `RpView<T>` — a wrapper that holds a state slice and a `ReactiveScope`. On construction it subscribes to all external changes on the slice and sends a unit message over an unbounded channel. A background task inside the entity drains that channel and calls `entity_cx.notify()`, which triggers a GPUI re-render.

This means GPUI reads state synchronously during `render` via `.get()`, while change detection happens asynchronously in the background.

## Setup

```toml
[dependencies]
amethystate-gpui = "*"
```

Initialize the store before opening any windows:

```rust
StoreBuilder::new("./app.redb").init_global();
```

## Defining state

```rust
#[amethystate(prefix = "counter")]
pub struct CounterState {
    #[amestate(default = 0)]
    pub count: i32,
}
```

## Creating an entity

Use `cx.new_amethystate()` instead of `cx.new()` to wrap a state slice in an `RpEntity`:

```rust
struct CounterView {
    state: RpEntity<CounterState>,
}

impl CounterView {
    fn new(cx: &mut Context<Self>) -> Self {
        let state = cx.new_amethystate(CounterState::new).unwrap();
        Self { state }
    }
}
```

`RpEntity<T>` is an alias for `Entity<RpView<T, Store>>`. `RpView` derefs to `T`, so state fields are accessed directly through the entity.

## Reading state in render

```rust
impl Render for CounterView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_count = self.state.read(cx).count().get();

        div().child(format!("Count: {}", current_count))
    }
}
```

## Writing state

Writes can happen from anywhere — the entity's `on_click` handler, a background thread, another part of the app. Any external write triggers a `notify()` and a re-render:

```rust
// from a click handler inside render
let state = self.state.clone();
Button::new("Increment")
    .on_click(move |_, _, cx| {
        state.read(cx).count().update(|v| v + 1).ok();
    })

// from a background thread via fork
let forked = state.read(cx).fork();
std::thread::spawn(move || {
    loop {
        std::thread::sleep(Duration::from_secs(2));
        forked.count().update(|v| v + 1).ok();
    }
});
```

Note that writes from the same instance (non-forked) do not trigger an `external` subscription and therefore do not notify the entity. Use `.fork()` when writing from a background thread if you want the UI to react.

## Using with a custom GPUI version

If your project depends on a git version of GPUI, add a `[patch]` to your workspace `Cargo.toml` to ensure a single copy of the crate is used:

```toml
[patch.crates-io]
gpui = { git = "https://github.com/zed-industries/zed", rev = "abc123" }
```

Without this, Cargo will treat the crates.io and git versions as separate crates and you'll get type mismatch errors at compile time.

## Examples

- [`gpui-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/gpui-settings)