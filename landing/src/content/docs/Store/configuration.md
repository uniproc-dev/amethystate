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

`Disk::debounce` is how long after the first unsaved write the flush comes.
Writes made in the meantime join that flush instead of putting it off, so a
stream that never pauses is still written once per window. Raising it batches
more writes into one commit; lowering it narrows the window
a crash can take. Reads are unaffected either way - a buffered write is visible
at once.

What a flush takes with it is the engine's answer rather than this setting's:
`redb` and `sqlite` commit what was asked for, a text engine rewrites the whole
file and so commits everything waiting. [Durability](/amethystate/concepts/durability/)
has the consequences.

`Disk::watch_every` is the other direction: how long the file has to sit still
before a change made outside the process is read back. Nothing polls - the
watcher is event-driven, on inotify, `ReadDirectoryChangesW` or FSEvents
depending on the platform - and this is the quiet period after the last event,
so an editor saving in several bursts costs one re-read instead of several.

`watch_every` is for the text engines only, the ones a person might edit by
hand. `debounce` applies to every engine.

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
        d.retry_every(Duration::from_secs(2))
            .give_up_after(Duration::from_secs(30))
            .on_failure(|gave_up| match gave_up.why.current_context() {
                StorageError::Flush => AfterGivingUp::Ignore,
                StorageError::Codec => AfterGivingUp::Poison,
                _ => AfterGivingUp::Fail,
            })
    })
    .build()?;
```
<!-- /shown -->

Three cases, named rather than lumped together. `StorageError::Flush` is the
disk: full, read-only, taken away, held by something else. `StorageError::Codec`
is the document not rendering. Everything else is a failure this callback has no
opinion about, and falls back to `Fail`, which is what an unconfigured store
does.

Nothing there stops anything. The retry loop is unconditional and runs until the
flush lands or the store is dropped, so a full disk is answered with `Ignore`:
writers see nothing, the buffer keeps everything, and freeing space heals the
store minutes or hours later with no restart. `give_up_after` is how long the
failing goes on *quietly*, never how long it goes on.

A store nobody configured answers `Ignore` for the same reason: a full disk is
an ordinary Tuesday on plenty of machines, and a state library is not the right
thing to bring an application down over it. The streak is still reported at
`error` under the `amethystate` target whatever the answer, so nothing is
hidden - it is only kept out of the write path. Write the callback when you want
otherwise.

`Disk::retry_every` is the gap between attempts. `Disk::give_up_after` is how
long a failing streak may run before `Disk::on_failure` is asked what writers
should be told from then on:

| answer | what a writer sees |
| --- | --- |
| `AfterGivingUp::Ignore` | nothing. Writes carry on landing in the buffer. The default when no callback is set |
| `AfterGivingUp::Fail` | an error each, naming the reason, until a flush lands |
| `AfterGivingUp::Poison` | a panic |

The enum is `non_exhaustive`: what a store can usefully do about a disk that is
not taking writes is not a settled list, and a `match` on it needs a `_` arm.

The callback is handed the failure, so the answer can depend on it. It is also handed
`gave_up.unsaved` - every path written since the last flush that landed, which
is what the store was carrying when it gave up. Candidates rather than
culprits: a document is rendered whole, and a render that fails names no node.

`Poison` is for the other kind - a failure that will not heal on its own. A
document the codec cannot render is in the same state on the hundredth attempt
as on the first, and nothing outside the program is going to change that.

It arrives as a `Report<StorageError>` rather than as one of the
[error sets](/amethystate/concepts/errors/), and deliberately: this runs on the debouncer's
thread with nobody's call to answer, so it is the engine's report rather than a
boundary's. Nothing here is being handed back to a caller.

Reads carry on in every case, and what is buffered stays buffered - which is
also the catch. `on_failure` answers who is told and never removes the cause.

Most codec failures never get this far. A value is encoded where it is written,
so one the format cannot hold is refused by `set` itself and never enters the
buffer.

### Watching without deciding

`on_failure` is one answer, given when the store is built. Hearing about a
failure is a different job, and any number of places can do it on a store that
is already open - a status bar, a telemetry sink, a test:

<!-- shown: watching a store's saving from anywhere -->
```rust
let watch = store.on_persist_failure(|event| match event {
    PersistEvent::GaveUp { failure, decision } => {
        eprintln!("not saved ({decision:?}): {:#}", failure.why);
    }
    PersistEvent::Recovered => eprintln!("saved again"),
    _ => {}
});

if let Some(why) = store.persist_failure() {
    eprintln!("the last save that gave up: {why:#}");
}
```
<!-- /shown -->

An observer hears every streak that outlived `give_up_after`, together with what
`on_failure` decided about it, and then the save that lands after it. It decides
nothing. It stays attached while its watch is kept, and dropping the watch
detaches it. Like the callback, it runs on the debouncer's thread, where closing
the store is refused.

`Store::persist_failure` is the same news for a caller that would rather ask
than listen: the failure the last streak gave up with, whatever was decided,
until a save lands again. Writers are told only under `Fail`.

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

The redb engine is the one that reads this setting. SQLite and the text engines
keep their reads on the calling thread, so asking them for it is accepted and
changes nothing.

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
