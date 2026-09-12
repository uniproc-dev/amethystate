---
title: "Durability: availability over consistency"
sidebar:
  label: Durability
  order: 18
---

Most storage documentation opens with what it guarantees. This page opens with what it gives up, because that is the part you need in order to decide whether `amethystate` suits what you are building.

The parallel with CAP is loose but honest: faced with the same kind of choice, this library gives consistency away. Durability is what drags asynchrony — or blocking — into the paths that touch state, and a user interface wants neither. Nobody wants to await a read, and few want to await a write. Consistency here means the agreement between what you read and what is durably stored, and that agreement is what pays for keeping both paths plain synchronous calls. What you read is always the truth about your application's state. It is not always the truth about what is on disk.

Everything below is the shape of that decision: what it buys, what it costs, and where you can buy the guarantee back when you need it.

## How it works

Writes do not reach disk when you make them. `field.set()` puts the value in an in-memory buffer, notifies subscribers, and returns. A debounce timer flushes the buffer to storage a little later, and a burst of writes to one path costs one flush rather than one each.

**An identical write stops at the comparison.** The bytes go against what the store holds, buffered or committed, and a match ends it there: nothing buffered, no subscriber called, no flush scheduled. A slider that rounds to the step it was already on, a form saving on blur without an edit, a cache revalidated on a timer — each costs one memcmp. The comparison is on bytes, so a `f64` holding `NaN` deduplicates against itself.

**Subscribers hear about changes.** Writing the same value again is silent, so anything that needs to say *checked, still valid* needs its own place to say it.

Reads are cheap for the same reason. `get()` looks in that buffer first and answers from it, so a value you have been writing every frame is read back from memory, not from storage.

## What you get

**Reads see your own writes.** A value you just wrote is immediately visible through `get()`, even before it reaches disk. The buffer is consulted first.

<!-- shown: a write you can read and the disk cannot -->
```rust
state.port().set(9090)?;

let reads_back = state.port().get();
```
<!-- /shown -->

`reads_back` is `9090`, and the document on disk still holds the value from before.

**Flushes are atomic.** Everything buffered goes to storage in a single transaction. Storage never holds a half-written batch.

**Clean shutdown loses nothing.** Dropping the store flushes it. A process that exits normally has everything on disk.

## What you lose

**A crash loses the buffer.** A process killed by a signal, aborting on panic, or cut off by power loss loses everything written since the last flush. Destructors do not run in those cases.

**The window is the debounce interval** — 300 ms by default:

<!-- shown: narrowing the window -->
```rust
let store = StoreBuilder::new(path.path())
    .disk(|d| d.debounce(Duration::from_millis(50)))
    .build()?;
```
<!-- /shown -->

A smaller value narrows the window and flushes more often. A larger one widens it and flushes less. This is the only knob, and it controls exactly this trade.

**A notification does not mean the value is stored.** Subscribers are called during `set()`, before the flush. A subscriber can observe a value that a later crash erases. If your callback does something irreversible outside the process — sends a request, writes another file — do not treat the event as proof the value survived.

## Waiting for the disk

`save_now()` pushes the whole buffer out and returns once the store has
committed it.

<!-- shown: forcing everything out -->
```rust
state.port().set(9090)?;
store.save_now()?;
```
<!-- /shown -->

Fields, maps, cells and `Kv` each offer a `durable()` view: the same writes, every one of them returning only once the change is on disk. That keeps the guarantee to a single call, with no window between writing and committing for you to be preempted in — or to forget:

<!-- shown: a write that waits for the disk -->
```rust
state.port().durable().set(9090)?;
```
<!-- /shown -->

Off the UI thread there is `set_async`, lazy like any future, so nothing happens — the write included — until it is awaited:

```rust
state.port().durable().set_async(9090).await?;
```

Reach for these at points where losing the last few hundred milliseconds actually matters — before launching an external process, after a step the user cannot repeat. Calling them on every write gives back the cheap writes you came for.

### A durable write commits its neighbours

`port` below is written durably, and `host` — buffered a moment earlier and never asked to be durable — is on disk when the call returns:

<!-- shown: what else a durable write commits -->
```rust
state.host().set("10.0.0.1".to_string())?;

state.port().durable().set(9090)?;
```
<!-- /shown -->

How wide that goes depends on the engine. `redb` and `sqlite` commit everything buffered under the same prefix in one transaction; the text engines rewrite the whole document, so one durable write makes the entire store durable.

Two consequences worth holding on to. The cost of a durable write is not the cost of your value — it is the cost of whatever else is waiting under that prefix, which you did not choose and cannot see. And a value you deliberately left buffered can reach disk because something beside it was committed, so "not durable yet" is never a guarantee about where a value *is not*.

## Everything follows these rules

There is no separate path with immediate durability. Every write lives by the same terms, including the bookkeeping `amethystate` does for itself, such as marking a namespace as initialized — that goes into the same buffer as a value does. The uniformity is deliberate: it is what lets a value and the metadata describing it land in the same transaction, so a crash can never leave one without the other.
