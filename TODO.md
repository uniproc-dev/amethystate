# TODO

What is left, and nothing else. A question that has been answered leaves here
and lives where it is acted on - the commit that settled it, the doc that states
it, the test that holds it.

Sizing note, because it has skewed judgement before: this is a store for
persistent reactive state, not a settings file. Settings are its smallest case -
tens of keys, written by hand. A cache persisted between runs is just as much
the target, and that means thousands of keys, written in bursts, read in scans.
Costs dismissed as trivial at ten keys are not trivial at ten thousand, and the
entries below are sized for the larger case.

## What a value coming in from the disk is checked against

### Absence is one thing

A declared path that is not there takes its default and says nothing, whether it
was never written or was written and removed. **Decided: the two are not told
apart.** Telling them apart is buildable - the initialisation marker and the
recorded schema are both on the disk, and together they say which - but no case
was found where the caller would do something different, and an outcome nobody
branches on is a word rather than a decision.

What that also settles: no `Missing`, no fifth outcome, and no *two defaults -
one for a new install and one for an existing one*, which only had somewhere to
sit if the two absences were distinguishable.

*A map entry is data, not a declared path.* One bad entry out of a thousand is
no reason to withhold the struct - though the default today is `Refuse`, which
withholds it, and `Skip` is what drops and reports.

**No aggregate, because there is nothing to aggregate.** The store never builds
structs; the application does, by name, one call site at a time. Three prefixes
refusing is three ordinary `?` in the caller's own control flow, and there is no
moment at which the store could decide to give up - it was never the one asking.

### The list this has to be checked against

Every way a store can be wrong, what should be observed, and whether anything
observes it today. Written before the code so the code can be held to it - and
because half of these are already covered by the `tamper_*` suite, which is
where the rest belong too.

**The file.** Whoever called `build` is the one told.

| what is wrong | should be | today |
| --- | --- | --- |
| no file | seeded, nothing said | yes |
| zero bytes | refused | `an_empty_file_is_refused` |
| truncated mid-document | refused, file untouched | `a_truncated_file_is_refused_and_left_alone` |
| valid document, rubbish after it | refused | `a_whole_document_with_bytes_left_after_its_end_is_refused` |
| another format's content | refused, naming the format expected | **no** |
| root is a scalar | refused | `a_scalar_root_is_refused` |
| unreadable - permissions, a directory in the way | reported, not a panic | `a_path_that_cannot_be_written_is_reported` |
| metadata gone, data present | defaults must not come back over removals | `tamper_meta`, for a declared level |

**A declared path.** Whoever called `new_with` is the one told.

| what is wrong | should be | today |
| --- | --- | --- |
| prefix never written | every field seeded, silent | yes |
| a declared key absent, however it got that way | seeded, silent | yes, and that is the answer |
| value out of range | `Refused`, takes the default, in the report | `field_check`, `struct_check` |
| a leaf became a branch | refused | `a_leaf_that_became_a_branch_will_not_read_as_the_field` |

**A map entry.** Nobody is told by refusing - these are data, and the struct is
still built.

| what is wrong | should be | today |
| --- | --- | --- |
| one key will not read as `K` | dropped, name in `unreadable_keys()` | **no** - the async path `continue`s, silent |
| one value will not decode | dropped, key in `dropped()` | **no** accessor; under `Skip` it is logged by name |
| *every* key fails | one line of drift at open: the key type changed | **no** - under `Refuse` the map refuses, under `Skip` it reads empty |
| `clear()` while an unreadable entry is there | everything goes, and it is said | **no** - goes silently |
| the map's path holds a scalar | refused | `a_section_that_holds_a_scalar...` |

**While the store is open.** And this is the row that does not fit the rest:
**strictness has no runtime form.** A struct that already exists cannot be
un-built, so an external edit that breaks a field cannot refuse anything - it can
only fall back and report. So the policy differs by moment, and that difference
has to be documented rather than discovered.

| what is wrong | should be | today |
| --- | --- | --- |
| an edit deletes a declared key | falls back to the default, reports | `field_delete` |
| an edit adds a map key that will not read | appears in `unreadable_keys()` | **no** |
| a broken edit is not overwritten by us | left alone | `WhenItWillNotRead`, `tamper_live.rs` |

### Left on this: the repairing form, and the load that cannot report

**A struct's check refuses; it does not correct.** The repairing shape is
`Fn(&mut Schema, &Provided) -> Policy`, and what it needs is the reason it is
not built: a generated typed projection per struct, reachable by name -
`s.font_size()`, `s.net().host()` - with the shape of each accessor decided by
the `Role` the macro already reads off the type, built during the load, handed
to the check and then **kept by the instance** so `ui.schema()` afterwards is
the same object rather than a second type. That is a feature the size of this
one. `_Data` is not a substitute: it cannot say *which* path, nor what happened
to it - read, undecodable, absent-and-seeded - and a dropped map entry has
nowhere in it to be mentioned.

When it is built, the corrected value does not go back to the disk. Writing it
back silently rewrites somebody's edit, which is what `WhenItWillNotRead`
refuses to do by default. Hold the corrected value in memory and let the next
ordinary write settle the file.

**And the one place the design does not close.** Under `mode = "persistent"`
there is no `Field`, so there is no `try_get`: a refused value under
`UseDefault` takes the declared default and the log is the only place it is
said. `Refuse` - the default - fails the load instead, and is the answer to
reach for when a loaded struct has to be trustworthy.

**Decided: a second constructor rather than a second return type.** `load_with`
keeps its shape, and a caller who wants to know what was wrong asks by name -
the same split `build` and `build_with_migration` already have. Nobody who does
not care pays for it, and the one who does gets the values and the complaints
together rather than a log line.

A map's entries are still out of `check`: they are data rather than declared
paths. The refusal is not written, though - `said_of_the_wrong_kind` covers
`on_unreadable` and `unreadable_entries` only, and `check` on a map field is
ignored rather than refused. What a map does have is
`on_unreadable`, which leaves out an entry that will not read - and the entries
it left out are named in the log and nowhere a caller can ask, which is the
`unreadable_keys()` row of the table above.

One neighbour from the sector research belongs with this: loading the fields
that do read while collecting the errors of those that do not, instead of
refusing the whole struct.

Setting a file aside under a name that says so is answered on the save side by
`WhenItWillNotRead::SetAside`. At open there is still no such answer - only
`WillNotOpen::StartFresh`, which takes the file away, which is what a cache
wants and what settings never do: a person who typed something into that file
has nothing left to be shown.

## Isolation: a collision is refused, confinement is not

Two goals that are easy to conflate, and they want different things:

- **collision avoidance** - nobody writes where somebody else writes. A check
  suffices, no layout changes. `Places` is that check, `tests/prefix_overlap.rs`
  states what it refuses.
- **confinement** - A cannot read B even on purpose. Needs a token, *and* needs
  the absolute-path API to go: while `Store::set(impl IntoStorePath)` is public,
  a synthetic root is a speed bump, since anyone who knows the other token can
  type it.

**A synthetic root was weighed and set aside for the declared case.** Two
readings of "the fiction need not be persistent": it never reaches disk, or it
reaches disk but is recomputed from the schema each run. The second is real
isolation but the derivation has to be stable, so a rename or a moved module
orphans the data silently - a worse failure than a loud refusal at startup, for
a library whose subject is persistence. The first cannot prevent a disk
collision at all, only attribute it, so refusal stays the outcome either way.
Both cost the flat readable layout the text engines exist for. It remains the
right shape for a genuine sandbox - a plugin, where the token is the host's own
id and nobody hand-edits the file - which is a separate opt-in mechanism, not a
default.

**Reading claims off the disk was weighed and dropped.** The recorded schema
snapshots carry the declared paths, so what a schema owns is knowable before
anything is constructed, and even for a schema whose code is no longer in the
build. What rules it out is that migrations are the one part of this library
that is not worked out, and the claim table is not the mechanism to pull them
into. So: **runtime-only, and the meta layer is not touched.**

**Two mechanisms guard the same thing.** `Places` refuses a place against what
a constructor has already built; `Kv::guard` refuses a write against what
`inventory::iter::<SchemaEntry>` declares. The inventory is the wider of the
two - a declaration is there before anything is built, and every one of them
names a prefix - so what `Places` adds is only which instance took a place, for
the report.

**Decided: one reader of what is declared.** Eight places across four modules
walk `inventory::iter` today - `migration/{builder,engine,set}.rs`,
`store/declared.rs` in three, `tauri/amethystate-codegen` - and each decides
for itself what "declared" means. That is why two guards that agree on the
question can disagree on the answer. One reader hands the declared set back as
data; `Places`, `Kv::guard` and the migration engine ask it rather than the
linker.

**A map refusing a key more than one level below it** - `Under::Deeper` in
`read_entry` and `scan_map` - stays outside the claim table on purpose. It is
the only mechanism that works against a writer no table knows about: a raw
`Store::set`, a migration, a person with a text editor. **The table prevents,
the read detects.**

## A map entry whose key will not read as `K` disappears without a word

`primitives/map_ops_async.rs` skips an entry whose name does not read as the
map's key type:

    let Some(key) = K::read(key_str.as_str()) else {
        continue;
    };

The entry is dropped from `entries`, and therefore from the projection, `len`
and `keys`, with nothing logged and nothing returned. So a hand-edited file can
make a map quietly shorter, and the shape of the failure is the one this crate
keeps finding: a `continue` where a sentence belongs.

It reaches only a map keyed by [`Id`]: a `String` key is the name, so `read`
cannot refuse one. An `Id<u16>` whose file holds `alpha` under it - after a
hand edit, or a key type that changed without a migration - is the whole of it.

**Decided: `unreadable_keys()`.** The map keeps the names that did not read and
hands them back when asked, so a caller who cares can see what was left out and
one who does not is unaffected. This is data the application may have to be told
about, not a file this library could not have written, and a log line is not
somewhere a program can look.

That settles what the answer is; what is left is that the library currently
gives three of them, and they have to become one. `load_map` refuses the whole
map with `KeyWillNotRead`, `map_entries_async` skips the entry, and `left_out`
does not admit `KeyIsNotAnEntry` to the `UnreadableEntries::Skip` path at all -
so the same file opens on a text engine and does not on redb or sqlite. The
collecting has to happen on every path, under `Skip`; `Refuse` stays the
default and still refuses the map whole.

## A renamed map is emptied rather than moved

Cleanup now takes what the declaration owned, entries and all
(`MigrationContext::drop_withdrawn`), and a rename is a drop as far as it is
concerned: the old place goes. `_Data` does carry the map - an `IndexMap<K, V>`
filled by `ctx.scan_map` - so the entries are readable on the way through, but
nothing moves them, and a step that wants them across has to rebuild them by
hand the way `tests/migration_reactive_map.rs` does.

**Decided: a rename moves the subtree.** A leaf's value is carried across a
rename, and a map is a declared place like any other - a declaration that moves
should take what it owns with it, whatever shape that is. What is left is the
work: `drop_withdrawn` has to tell a renamed place from a withdrawn one, and
the move has to happen before the drop rather than beside it.

## A flush that can never succeed is retried at the same rate as one that can

`run_with_retry` never looks at what failed. `op()` returns `Err`, the streak
starts, and the same call is made again every `retry.interval` until it lands or
the store is dropped. That is right for a full disk, which is what the loop was
written for, and wrong for anything deterministic: the same document, serialized
by the same codec, fails the same way at the same rate forever.

**Most of this class never reaches the flush**, which is worth saying before the
rest sounds worse than it is. `set_erased_inner` encodes the value where it is
written - `D::serialize_node(value, &depth)` - so a `NaN` on json is refused at
`set`, by the caller's own `?`, and never enters the buffer. What is left is the
narrower case of a document that only fails *as a whole*.

**And there the text engines make it everyone's problem.** `persist` serializes
the entire document, so a value that cannot be rendered is not one stuck write:
every later write to that store is carried by the same flush and lands nowhere
either. The store goes on accepting writes into memory and never commits
another one, which from the outside looks like the disk went away.

`on_persist_failure` does not help here. It decides what writers are *told* from
then on - `Fail`, `Ignore`, `Poison` - and nothing anywhere removes the value
that caused it. There is no way to say "this one will never go; drop it and tell
whoever wrote it".

**The two pieces needed are both already there.** `StorageError` distinguishes
`Codec` from `Flush`, so the loop can tell a deterministic failure from a
transient one instead of treating both as weather. And a write carries
`source: Option<Uuid>` - the instance that made it - so the value can be
attributed back to a writer rather than only logged.

**Naming the culprit exactly is the wrong thing to chase.** The buffer holds a
document, not a list of writes, and a render that fails does not say which node
did it - so pinning the one bad value means either a bisect over the document or
a per-node re-render, and both are work in the path that is already failing.

Handing back **every path written since the last flush landed** costs nothing
and is enough. The writer knows what it wrote; a candidate set it can look at
beats an error that names nothing. It is also the more honest answer, since a
document can fail for a combination rather than for one node.

That half is done: `GaveUp` carries `unsaved`, filled from
`Standoff::touched`, so a callback is handed every path written since the last
flush that landed.

**Decided: the loop goes on not looking at what failed.** `StorageError` tells
`Codec` from `Flush` and a write carries the instance that made it, so it could
- but a document the codec cannot render is a case nothing in the field reaches,
and the budget already ends the retrying either way. What is kept is the half
that costs nothing: whoever gave up is handed the paths written since the last
flush that landed, which is a candidate set a writer can look at.

## Who tells the store the application is quitting

`GlobalStoreGuard::close` is the door and its `Drop` is the net, and both need
somebody to run them. The states are there already: the flush thread refuses
new work the moment it is told to stop, the pass on the way out runs once
rather than retrying into a process that is ending, and stopped is told from
poisoned. What is not settled is who says when.

**Where the trigger comes from, since the phases do not invent it.** pingora
models the same thing as an enum of service phases, and the transition into
graceful shutdown is driven by a `SIGTERM` handler the server installs: the
library holds the phases, the outside world delivers the event. A desktop
application has the same event under other names - a window closing, winit's
`LoopExiting`, Tauri's exit event - and this crate already has an integration
sitting on each of them. So `shutdown()` need not stay on the user's memory:
the integration that already knows the application is quitting can call it.

`atexit` is the other candidate and is worth less than it looks. It exists on
both platforms through the C runtime, takes an `extern "C" fn()` with no
context - which suits a static fine - and runs when `main` returns or `exit` is
called. It does not run on `abort`, `panic = "abort"`, `_exit`, a kill, or a
power cut, so it covers only the case an application can already handle with
one line, and none of the cases where data is actually lost. Other threads keep
running while its handlers do.

## Smaller, and cheap

- `crates/adapters/amethystate-reactor` is tracked in git and absent from the
  workspace `members`, so nothing builds or lints it. `observe.rs:81` declares
  `Entry<K, V, S: StoreBackend, M: AccessMode>` whose `S` appears in no field
  and no `PhantomData`, which would not compile - so it has not been built for
  a long time. Either it joins the workspace or it goes; leaving it where it is
  keeps eleven tests that have never run, which the suite section below counts.
- `reactive_map_with_path<TScope, ..>` binds `TScope: StateScope` and never uses
  it; callers turbofish three parameters for nothing.
- `Kv::keys` returns absolute paths, where `ReactiveMap::keys` returns
  `Vec<K>`. It should return the names below the namespace. (It returns
  `Vec<StorePath>` rather than `Vec<String>` now, which is the type being
  honest, not the answer being right.)
- `get_map_types` decides a field is a map by matching the last path segment
  against the literal string `"ReactiveMap"`, so a type alias or a renaming
  import generates a scalar field instead. It does not reach disk and it now
  says so itself - the macro emits a `Role` probe that names the field and the
  reason - but the misclassification is still made by reading how a type was
  written rather than asking the compiler what it is, which is what `shape.rs`
  does everywhere else.
- `ReactiveCell::update`/`modify` return `SourceGone` for an absent map key,
  whose message sends the reader looking for a lifetime bug they do not have.
  `Absent` is in the same enum.
- The README's headline example does not compile: `amethystate::Result` does not
  exist.

## Errors that reach nobody

From an audit of every bare `?` and every silent skip in `core/` and
`amethystate/src`. Ordered by what it costs.

**A failed migration reaches `StoreBuilder::build`'s caller as a log line and
nothing else.** The engine turns a failure into data -
`ComponentOutcome::Failed { error }` inside an `Ok(report)` - and `build` logs
the report rather than raising it, because its return type is the store. So a
store at v1 with a v2 step that returns `Err` still opens, holding
pre-migration data, and the application runs new code against old data unless
somebody reads the log. `MigrationReport` is not `#[must_use]` either.

**`CommitSignal` reduces a report to one bool** (`store/durable.rs:35`). Every
producer has a `Report` in hand and throws it away; `outcome` then builds a bare
`CommitFailed` from nothing. A user awaiting a durable write on a full disk gets
the same one line as one whose database was deleted. Two smaller faults in the
same struct: `last_failed` is one flag rather than per-generation, so a waiter
across two overlapping flushes reads the wrong result; and `Commit::gone` gives
the same `CommitFailed`, so "the store was dropped" and "the write did not land"
are indistinguishable.

**The migration engine does not attach what the error model documents it
will.** `store/error.rs` says the frames around a step - which prefix, which
version, which store - are put there by the engine. At `engine.rs:371`
(`step.run(&mut ctx)?`) it holds all three and attaches none. Same for every
bare `?` on the bookkeeping calls in `migrate_prefix`, where `ensure_snapshots`
in the same file attaches carefully. On sqlite, whose `run_migrations` also does
not name the store where redb's does, a failed migration yields a report with no
locating information at all.

**Every interceptor rejection reports the same thing, including the one that is
not a rejection.** `run_interceptors` distinguishes three outcomes; all five
call sites collapse them to `Intercepted`. The damaging one is depth
exhaustion - nothing rejected anything, the guard refused to run because the
write is ten levels deep in interceptor-triggered recursion, which is a bug in
the caller's own code reported as a validation refusal.

**The file watcher can go deaf without saying so.** `text/store.rs:320` -
`let Ok(event) = res else { return };`. `notify` delivers its own failures
through that channel: a dropped watch, a lost handle, queue overflow. After one,
the store may stop seeing external edits entirely, and the only symptom is that
they stop arriving.

**`open` claims a restore happened whether or not it did.** A failed
`restore_from_backup` now logs, but the attachment on the refusal - "the files
were restored from their backups" - is written without asking, so a reader who
believes it will not check the file.

**`entry_cell` returns `ReactiveCell<V>` with nowhere to put an error.** It
seeds from the map's projection, which is an infallible cache read, so there is
no read failure left to swallow - but the signature still cannot carry one if
the shape ever changes.

**Poisoned-lock fallbacks that silently disable a subsystem.**
`ReactiveMapCore::notify` fails open on a poisoned lock while the same file
uses `.lock().unwrap()` elsewhere - so a poisoned mutex makes `subscribe_any`
panic while `notify` quietly delivers to nobody, permanently. `Signal::emit`
answers the same question the other way, and only the map's side has a test
(`reentrancy.rs`). Neither policy is wrong; having both is.

The registry in `observability` has no lock to poison and so is not part of
this. What it has instead is one reader, `resolve_field`, called from a test and
nowhere else, over a map that only ever grows.

## The text engines replace two files with no barrier between them

**`StoreFiles::persist` is two atomic replaces, not one operation.** Each half
is `persist_atomic` - temp file in the same directory, `sync_all`, rename - so
neither file is torn on its own. Nothing joins them.

The order is the metadata first, and it is chosen so the disagreement is the
visible one: the metadata over-claims rather than under-claims, and `held`
catches an over-claim - the last save left keys here and the file now holds
none. Under-claiming would be silent.

**No order is safe for an operation that is not idempotent**, which a migration
step is not: metadata-first can leave a step counted that did not run, data-first
can run it twice. That wants an intent record - the metadata says a prefix is
being taken to v2, the step writes, the metadata says it arrived - so a crash in
between is a named refusal rather than either kind of silence. Two extra saves
per migration and none per ordinary write. `AppliedStep` is where the outcome
would go, and adding a field to it is what the additivity rule on
`SchemaSnapshot` is for.

**A kill inside `persist` can leave a copy that is older than the data it
describes.** The copy is taken immediately before `run_migrations`, and both of
that pass's outcomes take it away again - `clean_backups` on success,
`restore_from_backup` on a rollback. What is left is the window between the
migration succeeding and `persist` finishing: die in there with the data file
already replaced, and the `.bak` holds the pre-migration document while the
metadata records the migration as applied. The next open recovers onto it if
the data will not read, which puts a document of the old shape under
bookkeeping of the new one.

## The three text engines diverge in what a node can hold

The traversal is unified: `Navigable` abstracts a node - `get_child`,
`insert_child`, `is_map`, `scan_children` - and all three text engines walk it
through the same `generic_get` / `generic_set` / `generic_scan`.

The divergence is one level down, in what `Navigable` is implemented **for**:

```rust
impl Navigable for serde_json::Value    // json_doc.rs:16
impl Navigable for ::ron::value::Value  // ron_doc.rs:16
impl Navigable for toml_edit::Item      // toml_doc.rs:16
```

The abstraction says how to walk a tree and nothing about what a node can hold.
Every representational finding sits exactly there: ron loses variants because
`ron::value::Value` has no variant case; toml answers a parent path with its
only child's value because that is `toml_edit::Item`'s behaviour; the depth
limits differ because `parse` and `serialize` are the foreign libraries.

So the change is `Navigable` for **one owned node type**, with a reader and a
writer per format into it. `generic_*` does not change at all. That is smaller
than a new document layer, and it is the same per-format work any alternative
would need.

It also explains why the root defect and the leaf-scan defect are identical on
all three: they are in the shared half, and one edit fixes three engines.

**json has it; toml is the one to do next.** `Node` answers `known_same` from a
subtree hash, and every engine that cannot answer pays the whole diff instead -
which for toml is our own cost rather than the format's, because
`toml_edit::Item` carries the trivia a person left in the file and comparing two
items by value is a question it has no answer to. ron is a different entry: its
node loses variants, which is upstream's defect and not one an owned node here
should paper over.

### And the owned node should be immutable, which is how a snapshot gets cheap

A persistent tree - the one `ReactiveMapCore` already keeps its cache in -
where a write yields a version sharing every node it did not touch. This is the
representation the owned node is kept in rather than a second change: what it
buys is a before-image and a diff, both of which the code takes the expensive
way today.

- `lay_over_the_file` clones the whole document on every save that has to ask
  the file, and `look` clones it on every outside edit taken. Both become an
  `Arc` bump.
- The migration provider keeps `backup_data` and `backup_meta` as deep copies
  for rollback. That is a version, made the expensive way.

And it is what gives a flush that can never land a third answer. Today it
retries forever or refuses writers forever; with versions it can go back to the
last one that reached the disk, drop what came after, and say whose paths those
were - `GaveUp::unsaved` names them already. Finding the culprit becomes
rendering the last good version plus one change at a time, which is bounded by
the debounce window and paid only on a path that has already failed.

That third one carries the least weight of the three: a value that encodes on
`set` and then will not render as part of the document has not been seen outside
the tests that construct it, and those constructions are not what an application
does.

What it costs is the formatting and comments `toml_edit` preserves, which an
owned node does not carry. Accepted: nobody here edits a store by hand, and if
that is ever wanted it is what tree-sitter is uniquely good at - a CST with the
trivia attached - which is the one argument for it that the rest of its costs
did not answer.

### Where the diff stands, and what is left of it

The owned node is built for json - `tree.rs` and `json/json_tree.rs`, with
`JsonDocument` untouched beside it as the reference both are held to. What it
bought, at a hundred thousand keys on a declared tree:

| changed | reference | owned node |
| --- | --- | --- |
| nothing | 244 ms | 4.0 ms |
| 1 key | 287 ms | 13.3 ms |
| 10 keys | 287 ms | 13.1 ms |
| 25% | 290 ms | 82 ms |
| 50% | 300 ms | 148 ms |
| nothing in common | 456 ms | 514 ms |

**What those numbers measure, because the first reading of them was wrong.** A
level answers `known_same` from a hash it works out on the first ask and keeps,
so a bench handed one pair of documents hashes them once and compares for free
ever after - which reported 24 ns for "nothing changed" and was measuring two
`OnceLock` loads. The column above parses the incoming document per iteration
(`iter_batched`, so the parse itself is outside the measurement): the reading
the store holds stays warm across ticks because nothing writes to it, and the
one off the disk is new every time, so its hash is part of the tick and belongs
in the number.

So "nothing changed" is 4.0 ms, and it is one pass over a document that was
just parsed. It is also **not the common case**: a watcher wakes on the file
being *touched*, and `look` compares the bytes it read against the bytes this
store last wrote before parsing anything - 147 µs, no parse, no diff. The 4 ms
is what a file whose bytes really differ but whose document does not costs: a
reformat, or somebody writing the same values back.

What is left is the floor: one key costs 13.3 ms, of which about 4 is the hash
and the rest is a hundred thousand name comparisons - what it takes to find out
*which* name moved. Two readings of a file share no memory, so nothing cheaper
exists without a level that can answer for a **part** of itself: chunk hashes
inside a wide level, measured and set aside once because the sort and the
copying dominated it. They are gone now, so the arithmetic has changed and the
question is open again.

Where it is paid: `save` runs the diff only when `Standoff::holding` says the
file moved, and the watcher runs it on each settled outside edit. A save that
nobody raced does not pay it.

### What is left, and what it is for

**The scan still builds a `StorePath` per key per pass.** `push_shared` is a
list, an allocation and an `Arc` per key, for paths that do not change between
two readings of the file. The answer is the same one as everywhere else here:
the document keeps what it has already worked out, next to the levels it worked
it out from. That is what an immutable node makes safe rather than merely
possible - the worked-out part belongs to a version, so a version you still hold
is consistent by construction and there is no cache to keep coherent.

**A path is still two allocations to build.** `Levels` is a `Vec<SmolStr>` grown
and then shrunk into a `Box<[SmolStr]>`, inside an `Arc`. Measured, at three
levels: 254 ns to build against 12.6 ns to copy, so it is the building that
costs and the refcount that does not. An inline buffer takes about 30% of the
building and was set aside: a registry would take all of it, and the two do not
compose.

**A registry was weighed and refused, except the one already there.** Interning
paths at run time turns a per-scan allocation into permanent residency the size
of the store, which for a tool that opens a store and exits is a loss. What is
kept is compile-time: the macro emits `const { StorePath::from_static(..) }`, so
a declared path is `Held::Written` - a copy is a pointer and there is no count
to touch. Nothing else gets interned.

**`visit_prefix` on a flat engine builds a path per key again.** It used to
borrow one out of the string key with `PathRef::parse`, which a byte key has no
equivalent for. A borrowed path over a key is possible - the levels are
contiguous runs, and only a name holding `0x00` or `0x01` is not - but it is a
second representation for `PathRef` rather than a small change.

**A map's entries should live in a space of their own.** A declared map's
entries in a text engine already do - a declared prefix is a `Root::Tree` and
nests, so the file holds `cpu`, not `ui.layout.levels.cpu`. Repeating the prefix
on every key is a flat-engine property, and what it costs is now only the bytes
on disk per entry: the comparison against the prefix per key went with the
joined key, since a subtree is a byte range the engine walks and nothing is
filtered afterwards. redb has named tables for exactly this; sqlite gets the
same from a small integer beside the entry name. Shortening the prefix to
`a.b.c` is a workaround for not having the projection - with it the prefix is
not shorter, it is gone, and an entry name on its own nearly always fits
`SmolStr` inline.

Decided: this is the direction. What is left is the design, not the question.

It does not touch `StorePath` at all - it is a storage layout, not a path
representation. What it needs designing for is the boundary: a `delete_prefix`
above a map has to reach into its space, a scan spanning a map and its
neighbours has to merge two sources into one order, and existing stores keep
their entries in the shared space, so it is a migration rather than a flag.

### The ron node, worked out and not yet built

Every enum loses its variant name on ron, so an application with an enum
anywhere in its state cannot start. The cause is two lines in
`ron_doc.rs::serialize_node`:

```rust
let s = ron::ser::to_string(value)?;              // "On", "Level(3)", "(a: 1)"
let node: ron::value::Value = ron::from_str(&s)?; // Unit, Seq([3]), Map{..}
```

`ron::value::Value` is `Bool | Char | Map | Number | Option | String | Bytes |
Seq | Unit` - no variant among them. The reparse *succeeds*, which is why
nothing errors, and the name is gone. `EmptyStruct {}` dies the same way: ron
writes `()`, which reparses as `Unit`.

`ron::value::RawValue` (present in 0.12.1) is a `#[repr(transparent)]` wrapper
over `str`. `RawValue::from_rust(value)` renders and stops; `into_rust::<T>()`
parses back. Measured: `Level(3)` is kept as `"Level(3)"` and reads back as
`Mode::Level(3)`. That fixes the class rather than a list of cases - the node
stops being a model of ron and becomes ron.

**But `Node` has to be `Navigable`**, and raw text cannot be walked into. So:

```rust
enum RonNode {
    Branch(BTreeMap<String, RonNode>),  // a level the store made
    Leaf(Box<RawValue>),                // a value the application wrote
}
```

**Telling one from the other on parse.** Measured: the serializer already
distinguishes them - a struct renders `(x:1)` and a map renders `{"a":1}` - and
`RawValue` keeps that, while parsing into `Value` collapses both to `Map`. So
the split can be syntactic, with no schema involved: deserialize one level as
`BTreeMap<String, Box<RawValue>>`, look at each value's first character, `{`
means recurse and anything else is a leaf.

It cannot be a `Deserialize` impl on `RonNode`: serde's model erases the
distinction, `visit_map` fires for both, and the recursion has to work on the
text.

**The case it does not fix**, also measured: a user's own `BTreeMap<String, _>`
renders `{"a":1}`, identical to a branch, so a map-valued leaf still reads back
as a level. That is exactly today's behaviour, so nothing regresses - and the
complete answer is the schema, which records `Role::{Field, Map, Node}` per path
and is the discriminator this lacks.

**The part that needs designing before it is written.** `with_bytes_de` carries
this note today:

> deserialize from the node, not from its rendered text: a `Value` map renders
> as `{..}`, which a struct deserializer will not accept

Reading a *branch* as a struct works only because `Value` presents itself to
serde as a map and serde decides whether the target is a struct or a
`HashMap`. A `Branch` has to do the same, which means either a `Deserializer`
impl for `RonNode` or converting a branch to `Value` for that one operation -
and the conversion loses variants again for leaves *inside* the branch being
read. Narrow, but it is the thing to settle first rather than discover halfway
through.

## What the suite does not test

Audited by asking of each test whether it would say so if the behaviour under it
were broken, and naming the one-line mutation it survives where it would not.
`macrotest` is gone and the trybuild inventory is complete, so what is left is
tests that never run.

**Nothing runs this crate's tests against fewer than five engines.**
`Cargo.toml`'s dev-dependency on the crate itself names all of them, and cargo
unifies dev-dependency features into the package when building test targets, so
`--no-default-features --features json` builds a lib with all five. Measured:
2238, 2248 and 2252 tests across what used to be three legs. CI's clippy legs
still restrict per engine; `ci.ps1` passes `--all-targets` and so does not, and
a test run cannot either way.

**Decided: the integrations are a proof of concept and stay one.** wasm, the
reactor, gpui - nobody is using them, so what they cost has to be nothing, and
the only requirement on them is that they do not get in the core's way. What
that rules out is a fix that reaches back into the core to suit an integration;
what it rules in is leaving them where they are.

So these are recorded rather than open: `generate/wasm.rs` writes against a
client signature that moved to `StorePath`, glues a map entry's name on with no
escape, and spells the root `"."`, and none of it is compiled because nothing in
the workspace writes `target = "wasm"`. `amethystate-reactor` sits outside the
workspace on `amethystate` 0.10.0 from crates.io with 11 tests that have never
run. `amethystate-gpui` is excluded from clippy, test and doc. No leg passes
`--target wasm32-unknown-unknown`.

`crates/tauri/amethystate-codegen` formats paths too and is **not** part of
this: there the string is the output, a name written into a `.ts` file, not a
way of addressing the store.

## What five engines did with the same values, measured

One probe per engine, run against the category below rather than reasoned about:
`tests/probe_json.rs`, `probe_toml.rs`, `probe_ron.rs`, `probe_redb.rs`,
`probe_sqlite.rs`. They print rather than assert, so they pass whatever they
find - they are raw material, not a suite, and most of them should be thrown
away once the handful worth keeping have been rewritten as tests that fail.

Every store in them names its backend, because the default is redb and a text
probe that does not name one measures redb instead. That mistake is already
recorded further down; it cost eight files once.

Read the tables in the probe files for the full detail. What follows is what
changes a decision.

### What redb keeps that the text engines lose

The negative result, and it is large enough to bound the category. Confirmed
through a reopen: non-finite floats bit-for-bit including the sign of `NaN` and
a `NaN` payload; `-0.0` keeps its sign; `Option::None` survives; `u128`/`i128`
exact, and narrowing is refused rather than wrapped; key encoding injective
across seven near-collisions; prefix scans stop at the level boundary in both
directions, including the siblings sqlite leaks on; no residue after a
write-then-delete; non-string map keys, which no text engine can hold.

So the representational half of the category belongs to the document formats.
The two structural defects redb had of its own - positional structs and no
depth limit - are answered: the codec writes structs as maps, and the ceiling
is a chosen 512 refused at the write.

### Depth, all five measured

| engine | limit | what it counts |
| --- | --- | --- |
| ron | 64 | path + value |
| toml | ~81 path, ~80 value | separately; they do not combine |
| json | 127 | path + value |
| sqlite | 254 | value only; the path is a `TEXT` key and costs nothing |
| redb | none | the stack ends around 3,200 on the read side |

These are the numbers `Backend::depth_ceiling` holds, and the path is counted
against them now rather than met by the parser at the next open. What is left of
the entry is the measurement itself: sqlite's 254 was folded to json's 127 on
purpose, and redb's ceiling is a chosen 512 rather than anything the engine
imposes.

## Accepted on the way in, refused or altered on the way out

A category, not a bug. A write returns `Ok`, and the read of the same path does
not give back what was written - because the codec, the document or the key
encoding will take something on the way in that it will not return on the way
out. Every instance below was found separately and filed separately, and they
are one shape:

| what | write says | read gives | where |
| --- | --- | --- | --- |
Every one of them is now refused at the write - nesting past a codec's ceiling
and the same value at a deeper path by `Limits`, `f64::NAN` and the rest by
`screening`, a key with escapes by the `0x00`-terminated encoding, and clearing
a map takes the map's own level with it. What the category is for is the shape,
not the instances.

Three severities, and the middle one is the worst to live with:

- **The file will not open.** Total, immediate, and at least it is loud.
- **The value comes back different or not at all.** Silent. Nothing in the
  application ever learns, and the wrong value is now the stored one.
- **Residue.** A path nobody wrote is readable, which is only visible to a scan.

What made it a category worth naming is that the fixes do not compose. Each
instance has a cheap local fix - count the path's depth, refuse `NaN`, escape
keys differently - and the next instance is not covered by any of them. Reading
back what was just written would address the class as a class.

**Decided: no round-trip flag.** Probing all five engines emptied the class it
was for. Most of what the category held turned out to be defects in the store
rather than limits of a codec, and those are fixed; the representational
remainder is enumerable - non-finite floats, nested `Option`, depth - and
screening refuses each by name at the write. Paying an encode and a decode on
every write to guard what is left is not worth it.

It also says where these belong as tests: the general form is a property -
what a store returns for a path equals what was written to it - and
`backend_conformance.rs` is already the place that generates values and paths
and checks exactly that. Instances found elsewhere should end up there as
generated cases rather than as one hand-written test each.

The five engines will not agree on the answer and are not supposed to: what a
format can hold is a property of the format. What they can agree on is that the
disagreement is reported rather than discovered later, which is the same
decision *What the conformance suite says the engines disagree about* records:
one contract, with what a document cannot honour recorded per engine.

## A `Serialize` that never failed can still write a file that cannot be read

An instance of the category above, kept separate because it is the one with a
measurement behind it.

`tests/serializer_damage.rs`. The store refuses a value whose serializer errors,
and refuses it where it is written: `set` returns a report naming the path and
carrying what the serializer said, the file does not move a byte, and the next
flush is unaffected. Declaring a path whose *default* cannot be written fails at
the declaration, so a type that refuses everything never gets as far as a file.
Three tests pin that, and they pass.

What is not covered by refusing is a serializer that **succeeds** and still
writes a document the reader will not accept. `serde_json` bounds recursion at
128 on the way in and not on the way out, so a value nesting deeper than that is
taken without complaint and the file it lands in cannot be opened again:

```
JSON codec error: recursion limit exceeded at line 128 column 255
```

That one is refused at the write now, and
`a_value_the_writer_accepts_can_always_be_read_back` runs. The nesting is the
instance; the class is any asymmetry between what a codec will write and what it
will read, and every text engine has its own version of it.

Backups do not cover it, and are not supposed to. They are taken on open and
`clean_backups` removes them once it succeeds, which is the right scope: a
migration transforms data the store did not write, and failing part of the way
through leaves a document of neither shape. That is rare, bounded, and worth
copying a whole file for. An ordinary flush is none of those, and a copy before
each replacement would double the I/O of every write for a store that already
cannot lose the previous file to a torn one.

A copy would not help here anyway. It would be taken while the file was still
good, and the damage arrives with the write - so what would be needed is a kept
generation, not a backup, and that is a different feature with a different
price. `the_backup_covers_the_open_and_ends_with_it` states the scope so it is
written down somewhere other than the position of a `clean_backups` call.

Prevention is possible at all only because the asymmetry is one-sided in a
useful direction. The limit lives in `serde_json`'s `Deserializer` - the thing
that reads text - and neither `Value` nor the `Serializer` has one. So a
document of any depth is legal in memory and legal to write, and the only way
to learn that it cannot be read is to run the reader. Which can be done at the
write, where the caller is still standing.

What must not be done is check the value on its own. The budget is spent by the
whole document, and the levels the store nests a value under to spell its path
come out of the same allowance:

```
segments  2, value depth 120 -> opens
segments  2, value depth 126 -> does not
segments 10, value depth 120 -> does not
segments 40, value depth 120 -> does not
```

`where_a_value_is_written_decides_whether_it_may_be_written` stands on that
contrast: the same `Deep(120)` is taken at a two-level path and refused at a
ten-level one, and the refusal names what the path itself cost. A check that
round-trips the value in isolation passes it in both places and is therefore
wrong, which is worth writing down because it is the cheap implementation and
the obvious one to reach for.

**The cheap check is built and always runs.** `Backend::depth_ceiling` holds a
number per engine - redb 512, json and sqlite 127, toml 80, ron 64 - `Limits`
lowers it to the shallowest engine a store names, and `Noticed` counts what the
path spent before the value is encoded. A write past it is refused where it is
made.

**What is left is the expensive one**, and it belongs behind a flag rather than
always on: read the whole document back after writing it, which catches this and
every other asymmetry a codec might have and doubles the cost of every flush.
`StoreConfig` grew `file_write: FileWritePolicy` for the retry budgets, and this
is the same kind of question about the same operation:

```rust
StoreBuilder::new(path)
    .file_write(|w| w.verifying(Verify::ByReadingItBack))
```

off by default, for an application that would rather spend the time than ever
meet a file it cannot open.

## The book documents a library that is no longer there

Found by reading it end to end against the sources. Not a list of typos - these
are things a reader following the book cannot make work:

- `StoreBuilder::collect_migrations` does not exist.
- The migration pages destructure a report out of `build()`, which returns a
  store. `Migrations/overview.md` also documents a `~` row - `field 'port':
  u16 -> u32` - in the drift output, which `log_to_tracing` cannot print:
  `SchemaDiff` is `added` and `removed` only, and a type change under one name
  nags with no field named at all. That is deliberate; only the page disagrees.

What is left of the dependency ordering in `Migrations/overview.md` still has to
be revisited once the graph is demand-driven.

## What tampering with a text document does, found by doing it

`tests/tamper_*.rs` write a store, edit the file the way a person or another
tool would, and reopen. Every failing test asserts the behaviour that would be
right, so its failure message is the finding. Worst first; what is left here
loses data with no error at all.

Two of those `#[ignore]`s were not findings and have gone. A test that pins
something the design cannot do reads like a defect nobody has got round to, and
costs more than it says:

- **A leaf that became a branch** asked that `cfg.width.px` read back after a
  person nested a section under a declared leaf. It cannot, on any of the five:
  `cfg.width` is declared a leaf, so `cfg.width.px` is a whole key of the plane
  and the file's section is not that key. The half of it that *was* a finding -
  that the field must not read back as a number - now runs, as
  `a_leaf_that_became_a_branch_will_not_read_as_the_field`.
- **Losing the metadata over an undeclared map** asked that entries a user
  removed stay removed when the marker file goes missing. An undeclared map is a
  plane of whole keys with no level of its own, so emptying it leaves the data
  file holding nothing the marker could be recovered from - there is no evidence
  left to read. The declared case, where the level stands and can be read, is
  pinned and green in
  `a_declared_map_emptied_by_hand_stays_empty_when_the_metadata_is_lost`.

The rest are ordinary tests, and all of them are green - there is no `#[ignore]`
left anywhere in the suite. Every file but `tamper_engine_contrast.rs` is gated
on a text feature, and that one is the control - on redb and sqlite it passes,
which is the point of it.

**A section standing where a declared leaf is has no path that reaches it, and
that is the answer rather than a gap.** `[cfg.width]\npx = 800` where
`cfg.width` is a declared `u16`: the shape is reported and `Cfg::new_with`
refuses, which is what
`tamper_shapes::a_leaf_that_became_a_branch_will_not_read_as_the_field` pins.

Asking `get(["cfg","width","px"])` for `Some(800)` is asking a document engine
to look inside a value its schema calls a leaf. **Decided: it may not.** A flat
engine holds that value as opaque bytes under one key and could never answer,
so answering on the text engines alone would mean the same program reads
differently depending on which engine is under it - and that the two families
are interchangeable is what the whole library rests on.

**Losing the metadata file, for a namespace nothing declares.** It is a plane of
whole keys with no level of its own, so emptying it leaves nothing to read and
the `__init` marker in the lost file is all there was - a removed default comes
back. A declared level answers for itself, which is the rest of `tamper_meta.rs`
and holds; this case has nothing to answer with, which is why the test that
asked it was deleted rather than parked.

## What the conformance suite does not ask

`tests/backend_conformance.rs` states twenty-nine properties and runs each
against every engine compiled in; all of them hold on all five. Uncovered:
concurrency between two handles, the async surface, `is_initialized` across a
failed flush, and value shapes past `u32`/`String` - nested structs, enums and
sequences are where the three text formats differ most from each other and from
msgpack.

## Documentation

**`Watch::stream` has no doctest.** `reactive/watch.rs` carries two, and both
sit above `register_with_source`; `stream` is the last public method in
the file and has prose only. It is also the one that most needs an example,
because it is the only exit from the builder that is not a callback: what it
returns has to be polled, the loop shape is the thing a reader is looking for,
and "dropping the stream ends the subscription" is a lifetime rule that a
worked example states better than a sentence. `Concepts/subscriptions.md` shows
a loop over it, and that block is hand-written for the same reason - the page
has no test behind it either, so the two gaps are one gap seen twice.

**Several of these document today's behaviour, and today's behaviour is on this
list.** They are written to fail rather than quietly go stale, but they will
need rewriting as the entries above land:

| doc | what it records | changes with |
| --- | --- | --- |
| `Field::durable` | what each engine family commits, and that a text backend is accidentally stronger | `flush_prefix` becoming per-write |
| `Field::set`, `ReactiveMap::insert` | every write reaches the store | value dedupe |

Sorting is documented on `keys` and pointed at from `entries`: the order is the
store's, by the name each key borrows. That one is not expected to change.

**`Migrations/manual.md` is the migration context's page, and three things are
missing from it:**

- `provide`/`require`, which appear nowhere in the book - and they are the
  answer to why a step, being a bare `fn` that captures nothing, can still see
  anything from the application;
- that `scan_map` reads a map the step will write back whole, so an entry it
  cannot read is an error rather than a skip;
- what a failing step leaves behind, once migration atomicity above is
  settled - this one has to wait for that answer rather than describe the
  current behaviour, which is on this list.

When the list is empty, turn on `#![deny(missing_docs)]` for the documented
modules so the next undocumented public item cannot land quietly.

### The policies are not in the book at all

`FileWritePolicy`, `RetryPolicy` and `FlushPolicy` are configurable and the word
"policy" does not appear on any page. Two of them behave in a way a reader would
guess wrong: `FileWritePolicy` splits a write into two steps with unrelated
budgets, and `RetryPolicy`'s `budget` is how long the store stays quiet about a
failing flush, not how long it keeps trying - it keeps trying until it lands or
it is dropped. Configuring it as a give-up time gets the opposite of what was
meant.

`tests/atomic_write.rs` already exercises `FileWritePolicy` on both ends of its
budget, so the page can be sourced from it rather than written out. Where the
pages go depends on the shape settling above.

### A test that measures a format writes the page about it

`cargo xtask docs` turns a test into a page under `Limitations/`. A file
publishes only if it marks a region with `//@act` / `//@end`; the preamble
becomes the prose, each marked region becomes a block of code under it. Nothing
is keyed off a file name, so a file opts in by marking itself and opts out by
not. `--check` fails a run whose pages are behind their tests, which is what
keeps prose and code from drifting.

`absent_or_null` is the shape the rest should take: one question, every engine
answering it in one run. It used to pick a single engine through
`text_backend()` and carry the three-engine table as prose - the features are
additive, so the limit was a choice in the helper rather than anything cargo
imposed.

**Four things this deliberately does not do yet, and the order they will
probably be wanted in.**

*The page cannot say where the code goes.* Regions are appended in file order
under one heading. A page wanting prose, code, prose, code needs regions to
have names and the preamble to have holes to drop them into - `//@act name`
and a `{{name}}` in the prose. Everything else below assumes this exists.

*The section is hardcoded, and should stay flat rather than become a tree.*
Everything lands in `Limitations/`, which is the wrong name for what is
accumulating: `absent_or_null` is not a defect of this library, it is what a
person choosing between five engines needs to know before choosing.

A tree of sections is the obvious next step and the wrong one. `absent_or_null`
belongs to toml, to `Option`, and to choosing an engine all at once, so a tree
makes it pick one home and raises "where does this go" on the second page
rather than the fiftieth. Flat pages that declare what they are about, and
indexes built from those declarations, never ask it: a new way of slicing adds
an index instead of moving files. Renaming the section later moves every
published URL, so the name is worth settling before there are many.


