---
title: windows-reactor
---

`amethystate-reactor` reads reactive state into [windows-reactor](https://github.com/microsoft/windows-rs)
components. A hook returns a plain value — `u16`, `String`, `Vec<(K, V)>` — and the component
re-renders when the store changes. Subscriptions, thread hops and cleanup do not appear in your code.

Reactor renders on a single UI thread, while amethystate delivers changes from whichever thread wrote
them — including a file watcher, for edits made outside the process. The adapter crosses that boundary
with the host's own `UiMarshaller`, the same one `use_async_state` uses.

**Reactive mode** is required.

## Setup

```toml
[dependencies]
amethystate-reactor = { git = "https://github.com/uniproc-dev/amethystate" }
windows-reactor = "0.0.0"

[patch.crates-io]
windows-reactor = { path = "../windows-rs/crates/libs/reactor" }
```

`windows-reactor` is a placeholder on crates.io — an empty crate holding the name — so point it at a
`windows-rs` checkout. The patch has to live in your own workspace root, because patches are not
inherited from dependencies.

Neither crate is published while reactor itself is unreleased, so this adapter is a moving target.

## Loading state

```rust
use amethystate_reactor::AmeCx;

fn app(cx: &mut RenderCx) -> Element {
    let state: NetworkState = cx.use_ame_state();
    ...
}
```

`use_ame_state` reads the global store, so nothing has to be threaded through the component tree —
there is no provider. Call `init_global` once at startup. To use a store you hold yourself:

```rust
let state: NetworkState = cx.use_ame_state_in(&store);
```

Either way the slice is loaded once per component, not once per render. A slice that will not load
panics: a schema that disagrees with what is on disk is a startup fault, not something a render can
recover from.

## Reading

```rust
let port  = cx.use_ame(&state.port());
let theme = cx.use_ame(&state.theme());
```

`use_ame` takes anything that implements `Observe` — a `Field`, a `ReactiveCell`, or a `ReactiveMap`.
The value is read from the store once, when the component mounts; later renders read nothing. The
subscription is dropped when the component unmounts.

## Writing

There is no setter to unpack. The component already holds the handle, so write through it:

```rust
let bump = state.port();

button("+1").on_click(move || {
    let _ = bump.set(port + 1);
});
```

The write comes back as a change like any other, and the next render sees it.

## Maps

A `ReactiveMap` reads as a `Vec<(K, V)>`, sorted by key:

```rust
for (name, width) in cx.use_ame(&state.widths()) {
    ...
}
```

For one key, `use_ame_entry` — an absent key reads as `None`:

```rust
let cpu = cx.use_ame_entry(&state.widths(), "cpu".to_string());
```

It resubscribes when the key changes. Prefer it to reading the whole map when you only care about one
entry: `use_ame` on a map re-reads every value whenever any of them changes.

## Sources that change across renders

`use_ame` binds to the source it was given on the first render. When the source itself depends on
something — a selected item, a route — use `use_ame_keyed` and pass what it depends on:

```rust
let value = cx.use_ame_keyed(&source, selected_id);
```

The new value arrives on the render *after* the deps changed, so one render still shows the previous
source. That is the same rule React follows, and it is why `use_ame_entry` takes the key rather than
letting you rebuild the source yourself.

## What crosses the thread boundary

A change is delivered on whichever thread wrote it. The adapter marshals it to the UI thread, writes
the hook slot, and requests a re-render — so your callback code never runs off-thread and nothing you
write has to be `Send`. A value that did not actually change is dropped at that point and costs no
render.

This is also why nothing here asks you to drive a loop of your own: the marshaller already delivers
onto the UI thread, and the framework schedules the render.

## Examples

- [`reactor-settings`](https://github.com/uniproc-dev/amethystate/tree/master/examples/reactor-settings)
