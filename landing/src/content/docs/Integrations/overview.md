---
title: Overview
sidebar:
  order: 25
---

`amethystate` supports multiple GUI frameworks. Which mode to use — reactive or persistent-only — depends on the execution model of the framework.

## Execution models

### Retain-mode and TEA (egui, ratatui, iced, Xilem)

These frameworks have an explicit event loop. Either they redraw every frame (immediate-mode: egui, ratatui), or all state changes flow through a message → update → render cycle (TEA/MVU: iced, Xilem). In both cases the framework decides when to read state, and there is no place to attach subscriptions in a natural way.

**Persistent-only mode** is the right fit. Fields are plain Rust types, you mutate them in `update` or `show`, and call `save_lazy()` when done.

One caveat: persistent-only state does not observe external changes. If another thread, another process, or a manually edited file changes the underlying store while the app is running, the loaded struct will not update. If you need that, use reactive mode and call `.get()` at the start of each frame or update cycle — the framework loop naturally polls the latest value.

- [egui / iced / ratatui](./retain-mode)


### Property bindings (Slint, GTK 4)
Both frameworks own their UI properties. The bridge is bidirectional: subscribe to a Field<T> and push changes into the framework's property system, with optional back-propagation from UI callbacks into the field.
Reactive mode is required.

- [Slint](./slint)
- [GTK 4](./gtk4)

### Signal / hook-based (Dioxus, Leptos, Yew, windows-reactor)

These frameworks re-render from state a component subscribes to, and only when that state changes. The integration pattern is the same throughout — subscribe to a `Field<T>` and write the new value into whatever the framework re-renders from. Components read that, not the `Field<T>` directly.

What differs is where the value is kept and who owns it. Dioxus and Leptos use arena-allocated `Copy` signal handles; Yew uses RC-based ones passed by clone; windows-reactor keeps hook state on a single UI thread and hands out a marshaller for getting work onto it, which removes the channel and the background task the others need. The bridge looks different in each case, the concept does not.

**Reactive mode** is required.

- [Dioxus](./dioxus)
- [Leptos](./leptos)
- [Yew](./yew)
- [windows-reactor](./windows-reactor)

### Webview bridge (Tauri)

Tauri splits the application into a Rust backend and a frontend communicating over commands and events. `amethystate` provides a dedicated plugin that handles this boundary — state is loaded on the Rust side, and generated bindings expose it to the frontend. Both TypeScript and Rust frontend clients are supported.

- [Tauri](./tauri)

### GPUI

GPUI uses an entity model with deferred notification. Mutations are applied inside entity update closures, and the framework notifies dependents after the closure returns. This does not compose directly with synchronous `Field<T>` subscriptions, so the bridge goes through an async channel: a background task holds a `WeakEntity`, subscribes to reactive fields, and schedules updates via `AsyncApp`.

**Reactive mode** is required.

- [GPUI](./gpui)