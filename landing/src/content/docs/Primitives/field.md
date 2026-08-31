---
title: Fields
sidebar:
  order: 8
---

A field declared with `#[amethystate]` is a handle onto one path in the store.
Reading answers from memory, writing lands in memory at once and on disk on the
debounce.

<!-- shown: reading and writing a field -->
```rust
let port = state.port().get();

state.port().set(9090)?;

let raised = state.port().update(|port| port + 1)?;

state.port().modify(|port| *port += 1)?;
```
<!-- /shown -->

`get` returns the value itself. A field always has one: a path holding nothing
answers with the default its declaration gave it.

```rust
fn get(&self) -> T
fn try_get(&self) -> ReactiveFieldResult<T>
fn set(&self, value: T) -> ReactiveFieldResult<()>
fn update<F: FnOnce(T) -> T>(&self, f: F) -> ReactiveFieldResult<T>
fn modify<F: FnOnce(&mut T)>(&self, f: F) -> ReactiveFieldResult<()>
```

## Reading when the answer might not be the store's

`get` is the tolerant read: it always hands back something a render function
can draw. `try_get` is the same read with the doubt kept.

It answers `Err` when a change arrived that would not decode into this field's
type - a file edited outside the process, a migration that left something
behind, a codec that accepted a value it cannot read back.

The field goes on reporting the last value the store agreed with, and nothing
is delivered to subscribers. What is on screen was true a moment ago; the
declared default is a compile-time guess and the least likely thing the person
was looking at. So `get` keeps drawing, and `try_get` is where a caller that
cares finds out the store has stopped agreeing.

Nothing fails at the moment of asking. What failed happened earlier, and this
reports it - and answers `Ok` again as soon as a change decodes, so it holds
for exactly as long as it is true.

A value that was already unreadable when the struct opened is a different
moment, and one the declaration decides:
[what a value going wrong does](/amethystate/state/defining-structs/#what-a-value-going-wrong-does).

## Writing

`update` and `modify` are read-modify-write and are **not atomic**. Two of them
racing on the same field can lose one of the two results, the same way two
`get`-then-`set` pairs would. Where that matters, the store's own write is the
one to reach for.

`update` returning the stored value is the difference worth knowing: it saves
the `get` you would otherwise write on the next line, and it is the value that
actually landed rather than the one you computed.

## What a write costs

Nothing waits for the disk. A write reaches the buffer, the subscribers hear
about it, and the flush happens on the debounce - so a field written every
frame costs a buffer write per frame and one commit per debounce interval.

To wait for the disk instead: [Durability](/amethystate/concepts/durability/).

## What a write is refused for

Three things refuse a write: a value the running engine's codec cannot encode, a
path deeper than the store allows, and an interceptor that says no.

All three answer at the `set` that made them. The value is encoded where it is
written, so the refusal arrives in the caller's own control flow and the value
never reaches the buffer.

## Where to go next

- Hearing about a change, on your own thread or somebody else's:
  [Subscriptions](/amethystate/concepts/subscriptions/).
- Collections whose keys are decided at run time:
  [ReactiveMap](/amethystate/primitives/reactive-map/).
