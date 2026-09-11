---
title: Opening a store
sidebar:
  order: 12
---

A store is the handle everything is read from and written through. It is built
from a `StoreBuilder`, which decides where the files go and which engine holds
them, and it is `Clone` - the clone is another handle onto the same open store,
not a second one.

An application usually opens one and keeps it for the life of the process, but
nothing enforces that: several stores can be open at once, over different files
or different engines.

## At a path you name

<!-- shown: opening a store at a path you name -->
```rust
let store = StoreBuilder::new(settings).build()?;
```
<!-- /shown -->

`build` returns a `Store`. Hold it, pass it to `new_with`, and let it drop when
the process ends - see [Closing](#closing) for what that costs you if you never
look at the result.

## Or once, for the whole process

Passing a store into everything that needs one is explicit and tedious. The
process-wide store is installed once and reached from anywhere:

<!-- shown: opening the process-wide store -->
```rust
let _ame = "./app.redb".init_global();

let state = NetworkState::new()?;
```
<!-- /shown -->

`NetworkState::new()` is `new_with` with the global store filled in, so a
struct declared anywhere can build itself with nothing passed to it. The store
itself is reachable too:

<!-- shown: reaching the global store from anywhere -->
```rust
let store = global_store();
store.kv().set("theme", &"dark".to_string())?;
```
<!-- /shown -->

**The guard is the store.** `init_global` returns one and it is `#[must_use]`
for a blunt reason: dropping it closes the store, so `"./app.redb".init_global();`
on its own line opens the store and closes it on the same line. Bind it in
`main` - `let _ame = ...` - and the last writes are flushed when `main` returns.

Installing twice panics. It is a `OnceLock`, and a second `init_global` is a
bug rather than a re-configuration.

The same split as `build` and `build_with_migration` applies here:

<!-- shown: opening it with the migration pass -->
```rust
let (report, _ame) = StoreBuilder::new("./app.redb").init_global_with_migration();

if report.has_failures() {
    eprintln!("a migration step failed; the data was put back");
}
if report.has_drift() {
    eprintln!("a struct changed without a version bump");
}
```
<!-- /shown -->

## Letting the platform say where

Spelling a path yourself is right for a test or a tool pointed at a file. An
application that installs on someone's machine wants the place that platform
keeps for it, and `located` works one out:

<!-- shown: letting the platform say where the file goes -->
```rust
let config = StoreBuilder::located(|at| at.app("my-app", "settings"))?;

let named = StoreBuilder::located(|at| at.app_under(Layout::App, "my-app", "settings"))?;

let portable = StoreBuilder::located(|at| at.beside_the_executable("settings"))?;
```
<!-- /shown -->

Three places are on offer:

| how | where it lands |
| --- | --- |
| `at.app("my-app", "settings")` | the configuration directory this platform keeps for the application |
| `at.app_under(Layout::App, ..)` | the same, under the convention you name |
| `at.beside_the_executable("settings")` | next to the running binary, for an install somebody unpacked |

### Which convention `app` follows

There is more than one answer to "where does an application keep its files",
and `Layout` names them:

| layout | where it puts things |
| --- | --- |
| `Layout::App` | the XDG layout, on every platform including Windows and macOS |
| `Layout::Native` | wherever the host system says application files go, which differs from XDG off Linux |
| `Layout::ProjectDirs` | what the `directories` crate produces - XDG on Linux and Windows, `Library/Application Support` on macOS |

`app` picks `Layout::App` for you.

`Layout::Native` is what a desktop application usually wants, since it is where
the rest of the platform's software puts things - and `app` never chooses it.
Ask for it by name. `Layout::ProjectDirs` is for an application whose files are
already where the `directories` crate puts them, so that crate's reading of the
directory is the one the store has to match.

**Name the layout once you have shipped.** The conventions disagree about
enough of the tree that a store written under one is not found under the other,
so `app_under` is what pins where someone's settings live across an upgrade.

### Moving a store that is already somewhere else

The closure is called once and its answer is the path. What it does before
answering is yours - `located` hands over a `Location` and takes back a
`PathBuf`, and everything in between is ordinary code.

Moving needs more than one path, though. A path is what a store is *opened* at;
the files it is *made of* are the engine's - an extension it added, a sidecar
beside it, a copy kept while a rewrite is in flight. A layout names all of them,
and a move is a walk down one:

<!-- shown: moving every file a store is made of -->
```rust
fn relocate(from: &StoreLayout, to: &StoreLayout) -> std::io::Result<()> {
    for (old, new) in from.names().iter().zip(to.names()) {
        if old.exists() {
            std::fs::rename(old, new)?;
        }
    }

    Ok(())
}
```
<!-- /shown -->

**`names` is every name and not every file.** A rewrite copy is written when a
rewrite starts and removed when it finishes, so a store sitting still has none -
which is why the move above skips what is absent rather than failing on it. Two
layouts of the same engine pair up name by name, so the data file lands as the
data file and the sidecar as the sidecar; `present` is the same list with the
missing ones dropped, for asking what is actually there.

The layout itself comes from `at.files_at`, worked out from a path with nothing
opened - which is the whole difficulty with the old place, since there is
nothing there to ask:

<!-- shown: a store found where an older release left it -->
```rust
let store = StoreBuilder::located(|at| {
    let now = at.app(&app, "settings")?;
    let was = at.files_at(at.app(&app, "settings-legacy")?, backend);

    relocate(&was, &at.files_at(&now, backend)).ok();

    Ok(now)
})?
.backend(backend)
.build()?;
```
<!-- /shown -->

That is the answer to shipping under one layout and wanting another. Nothing
about it is special: move the files, copy them, ask the person first, or open
the old place and leave it there - the library does none of it and refuses none
of it. It is handed one path and works the rest out from that.

Whether the move converges is yours too. A closure that keeps answering with
the old path keeps both places alive forever, which is a decision rather than
an oversight - it is written where somebody can read it.

`beside_the_executable` is for a portable install, and not for an installed
one: `Program Files` and `/usr/bin` are not writable by the person running the
program, and on macOS the binary sits inside a signed bundle. Any of those
shows up as a failure to open the store - at startup, which is the good time
for it.

## Which file the name becomes

The engine names the file when the path you give has no extension:

<!-- shown: letting the engine name the file -->
```rust
let store = StoreBuilder::new(config_dir.join("settings"))
    .backend(Backend::Json)
    .build()?;
```
<!-- /shown -->

That writes `settings.json`. Name the extension yourself and it is left alone:

<!-- shown: naming the file yourself -->
```rust
let store = StoreBuilder::new(config_dir.join("settings.conf"))
    .backend(Backend::Json)
    .build()?;
```
<!-- /shown -->

That writes `settings.conf`, still in JSON. The rule is about ownership rather
than format: a name you spelled is yours - a `.conf` some other tool already
watches keeps working - and a name the library invented belongs to whichever
engine actually runs. It matters most when `backend` is called after the path
is set, since the extension is re-derived at that point.

Which engines exist and what each writes:
[Installation](/amethystate/getting-started/installation/).

## What to do when a value will not read, or its key is gone

Two decisions, and there is no opening without them. What is on disk does not
parse into the type it was declared as — fail, or take the declared default?
The key under a field was removed — go on reporting what was last there, or the
declared default again?

Left alone, the open refuses and names the path, and a field whose key was
removed goes on reporting the value it held before. To say otherwise:

```rust
use amethystate::store::OnUnreadable;

let store = StoreBuilder::new(path)
    .rules(|r| r.on_unreadable(OnUnreadable::UseDefault))
    .build()?;
```

The same rule can be written in three places: here, on the store; as an
attribute on a struct; and as an attribute on one field. Whichever sits closest
to the data wins, which makes what is said here the weakest of the three.
`Refuse` written on a struct stays `Refuse` whatever the store was told: nothing
said out here overrules somebody else's promise, it only supplies one to whoever
made none.

That is what makes this safe to reach for. Say an application keeps generated
thumbnails in the store and a licence beside them. Nothing about the thumbnails
is worth failing to start over — but the licence is:

<!-- shown: one process, two answers about the same store -->
```rust
#[amethystate(prefix = "thumbnails")]
pub struct Thumbnails {
    #[amestate(default = 0u32)]
    pub generated: u32,
}

#[amethystate(prefix = "licence", on_unreadable = Refuse)]
pub struct Licence {
    #[amestate(default = "".to_string())]
    pub holder: String,
}
```
<!-- /shown -->

Opened with `UseDefault`, a thumbnail count that will not read is replaced by
its declared default and the application starts. A licence that will not read
still stops it, because `Licence` said so for itself. One store, one process,
and the struct that cared is the one that decides.

Left alone, a value that will not decode refuses the open, and a field whose key
was removed goes on reporting what it last held. Which is which, and why:
[Defining structs](/amethystate/state/defining-structs/).

## Which migrations run

`build` runs the steps handed to the builder and no others.
`build_with_migration` also collects every `#[migrate]` step in the binary, and
returns what the pass did alongside the store.

So a store opened with `build` in a binary full of `#[migrate]` steps migrated
nothing, and said nothing about it. Reach for `build_with_migration` whenever the
macro is in play. [Migrations](/amethystate/migrations/overview/).

## Writing the buffer out

Dropping the store flushes what it holds, so an ordinary exit loses nothing.
Dropping says nothing about whether the flush landed, though, and a disk that
was full at that moment is the case worth hearing about:

<!-- shown: writing the buffer out and hearing whether it landed -->
```rust
store.kv().set("port", &8080u16)?;

if let Err(report) = store.save_now() {
    eprintln!("settings were not saved: {report:?}");
}
```
<!-- /shown -->

`save_now` is the same flush with the failure handed back. The store goes on
working: the later drop flushes again, and reports what it finds to the log
rather than to you. That is the whole difference - the drop is the one nobody
hears.

## Closing

`save_now` leaves the store holding its file. `close` gives it up: it writes
what is buffered, stops the background thread, and lets go.

<!-- shown: closing the store and letting go of the file -->
```rust
if let Err(report) = store.close() {
    eprintln!("the last writes were not saved: {report:?}");
}
```
<!-- /shown -->

Afterwards every read, write, scan and flush on that store answers `Closed` -
`ReadValue::Closed`, `WriteValue::Closed`, `ScanKeys::Closed`, `Flush::Closed`,
each naming the place it was asked about - and so does every clone of it, since
there is one file between them. Calling it twice is fine, and the drop that
follows does nothing.

A field is the exception, and on purpose: it goes on answering `get` from
memory, and says so through `try_get`, whose `Reason::Closed` means *this is the
last thing I heard* rather than *this failed*.

This is what to call when something else needs the file: another process, a
backup, a rename. What that buys differs by engine, and each engine gives up
what it was holding:

| engine | what a live store claims | what closing releases |
| --- | --- | --- |
| sqlite | the file, against the whole machine | renaming or deleting it starts working |
| redb | the right to open it | a second store can open the same path |
| json, toml, ron | nothing between flushes | the background thread only |

A field goes on answering `get` from memory, so a screen drawn from the last
values keeps drawing them. [`try_get`](/amethystate/concepts/errors/) is where
that shows: it reports that the store was closed and what the field holds is the
last thing it was told.

**The global store closes the same way.** Dropping the guard closes it, and a
failure there goes to `tracing::error!` under the `amethystate` target - nowhere
a caller can act on it. `amethystate::shutdown()` hands the failure back
instead, and it is called before the guard goes out of scope. A static is never
dropped, so without one of the two the last debounce interval is lost on a clean
return.

What that buys is exactly one thing: you find out, and can tell the person. The
writes cannot be rescued at that point - the store is closed, and everything
but a `get` from memory answers `Closed`. Calling `close` again will not help
either: it sees that closing has already begun and answers `Ok` without
flushing anything. The place for a second attempt is `save_now`, while the
store is still open.

For waiting on the disk for one write rather than all of them:
[Durability](/amethystate/concepts/durability/).
