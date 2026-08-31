---
title: Configuring a store
sidebar:
  order: 13
---

Everything here is set on the `StoreBuilder` before `build`, and applies to the
whole store. Every one has a default that is meant to be left alone; this page
is for the cases where it is not.

## Timing

<!-- shown: how long a write waits, and how long an outside edit settles -->
```rust
let store = StoreBuilder::new(settings)
    .disk(|d| {
        d.debounce(Duration::from_millis(500))
            .watch_every(Duration::from_secs(2))
    })
    .build()?;
```
<!-- /shown -->

Everything about when this store touches its file lives under
`StoreBuilder::disk`, which hands the defaults in and takes back what you
changed - so only the settings you name are written, and forgetting one cannot
zero the rest.

`Disk::debounce` is how long a write sits in the buffer before the flush.
Raising it batches more writes into one commit; lowering it narrows the window
a crash can take. Reads are unaffected either way - a buffered write is visible
at once.

`Disk::watch_every` is the other direction: how long the file has to sit still
before a change made outside the process is read back. Nothing polls - the
watcher is event-driven, on inotify, `ReadDirectoryChangesW` or FSEvents
depending on the platform - and this is the quiet period after the last event,
so an editor saving in several bursts costs one re-read instead of several.

It applies to the text engines, which are the ones a person might edit by hand.

## A flush that keeps failing

A failing flush is not a lost write. The same buffered changes are retried, and
they keep being retried until they land or the store is dropped - a full disk
that someone frees up heals the store with no restart.

What is configurable is how long that goes on quietly, and who is told when it
does not stop:

<!-- shown: how long a failing flush stays quiet -->
```rust
let store = StoreBuilder::new(settings)
    .disk(|d| {
        d.retry_every(Duration::from_millis(200))
            .give_up_after(Duration::from_secs(10))
            .on_failure(|failure| match failure.current_context() {
                StorageError::Codec => AfterGivingUp::Poison,
                _ => AfterGivingUp::Ignore,
            })
    })
    .build()?;
```
<!-- /shown -->

`Disk::retry_every` is the gap between attempts. `Disk::give_up_after` is how
long a failing streak may run before `Disk::on_failure` is asked what writers
should be told from then on:

| answer | what a writer sees |
| --- | --- |
| `AfterGivingUp::Fail` | an error each, naming the reason, until a flush lands. The default when no callback is set |
| `AfterGivingUp::Ignore` | nothing. Writes carry on landing in the buffer |
| `AfterGivingUp::Poison` | a panic |

The callback is handed the failure, so the answer can depend on it. The split
above is the useful one: a full disk is usually someone about to delete
something, and waiting it out is right, while a document the codec cannot
render is in the same state on every attempt.

Reads carry on in every case, and what is buffered stays buffered - which is
also the catch. Retrying is unconditional: the same flush is attempted every
`Disk::retry_every` until it lands or the store is dropped, whatever failed.
This
answers who is told, and never removes the cause.

Most codec failures never get this far. A value is encoded where it is written,
so one the format cannot hold is refused by `set` itself and never enters the
buffer.

## One write to one file

Below the retry budget, and not beside it: this is what happens *inside* a
single attempt, and only once it runs out does a flush count as having failed
at all.

<!-- shown: how hard one write to one file fights -->
```rust
let store = StoreBuilder::new(settings)
    .file_write(|w| {
        w.writing(WriteAttempts::times(3).apart(Duration::from_millis(50)))
            .replacing(WriteAttempts::times(20).apart(Duration::from_millis(250)))
    })
    .build()?;
```
<!-- /shown -->

It applies to the text engines, which write a whole document to a file of its
own and then replace the target with it. Those two steps fail for unrelated
reasons and take separate budgets: `writing` is ordinary I/O, where a full disk
stays full and a few quick attempts are all it is worth, while `replacing` is
an antivirus or a cloud client holding the file, which lets go on its own - so
its budget is the longer one, and raising it costs nothing until something
really is stuck.

redb and SQLite hold their own handle and write through it, so there is no
replacement to block.

## What the store refuses to hold

Set through `limits`, and described on its own page:
[What a store refuses to hold](/amethystate/store/limits/).

## Reading a large collection

<!-- shown: letting a large read use more than one core -->
```rust
let store = StoreBuilder::new(settings).parallel_reads(true).build()?;
```
<!-- /shown -->

Parsing every stored key and decoding every value is around four hundred
milliseconds of a million-entry open; splitting the work takes that to about
eighty. Off by default, because this is a thread pool inside a state library
and an application that already has one should say whether it wants a second.
While it is off nothing is spawned - the pool is built on first use.

Below roughly a thousand entries the handing out costs more than the work, and
the split does not happen.

## What is not a setting

`migrations`, `provide` and `context` sit on the same builder and are not
configuration. They are **inputs**: the steps to run, the values those steps
are handed, and the values the declared checks are handed.
[Migrations](/amethystate/migrations/overview/) covers the first two, and
[Defining structs](/amethystate/state/defining-structs/) the third.

The two that hand over a value are separate because they are read from
different places. A migration step runs once, inside `build`, on the thread
that called it, so `provide` takes anything at all - an `Rc`, a handle its
toolkit refuses to move. A check runs every time a value arrives, including
from the thread watching the file, so `context` asks for `Send + Sync`.
