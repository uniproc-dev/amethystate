---
title: Observability
sidebar:
  order: 19
---

`amethystate` emits structured trace events via the [`tracing`](https://docs.rs/tracing) crate. All events use the target `amethystate`, so you can filter them independently of the rest of your application.

## Enabling trace output

Trace events are at the `TRACE` level. To see them, configure a `tracing` subscriber with the `amethystate` target enabled:

```rust
tracing_subscriber::fmt()
    .with_env_filter("amethystate=trace")
    .init();
```

Or via the `RUST_LOG` environment variable:

```sh
RUST_LOG=amethystate=trace cargo run
```

## What gets logged

### Field writes

Every `field.set()` emits one event:

```
TRACE amethystate: field write path=network.port source=NetworkState
TRACE amethystate: field write path=network.port source=external
```

| Field | Value |
|-------|-------|
| `path` | Store key of the field, e.g. `network.port` |
| `source` | Short struct name of the slice that called `set()`, or `external` if the write came from outside the process (file watcher, another process, migration) |

### Subscription fires

Every time a subscription callback is invoked, one event is emitted:

```
TRACE amethystate: signal emit → subscription fire subscription_id=0 name=PortWatcher location=src/main.rs:42
TRACE amethystate: map signal emit → any subscription fire subscription_id=1 name=None location=src/server.rs:88
```

| Field | Value |
|-------|-------|
| `subscription_id` | Monotonic integer identifying the subscription within its signal |
| `name` | Label set via `.named()`, or `None` |
| `location` | `file:line` of the `.subscribe()` call site |

## Labeling subscriptions

Subscriptions capture their call site automatically via `#[track_caller]`. Add a human-readable label with `.named()` for subscriptions created in generic or framework code where the file/line isn't meaningful on its own:

```rust
let _sub = state.port()
    .subscribe(|p| do_something(p))
    .named("PortWatcher");

// ReactiveMap works the same way
let _sub = state.limits()
    .subscribe_any(|change| handle(change))
    .named("LimitsWatcher");
```

The label appears in every trace event fired for that subscription.

## Reading the output

A write followed by two subscribers firing looks like this:

```
TRACE amethystate: field write path=network.port source=NetworkState
TRACE amethystate: signal emit → subscription fire subscription_id=0 name=PortWatcher location=src/ui.rs:55
TRACE amethystate: signal emit → subscription fire subscription_id=1 name=None location=src/logger.rs:12
```

`source=external` means the change arrived from outside — a file watcher detected that the store file was modified, or a migration step wrote the value on startup.
