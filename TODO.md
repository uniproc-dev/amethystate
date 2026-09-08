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

A `ReactiveMap` is built through its own factory and carries neither
`on_unreadable` nor `on_delete`.

### Left on this: absence is two things and only one of them is damage

A prefix that was never written is a first launch: seed the defaults and say
nothing. A prefix that *has* been written and is missing one of its declared
paths is damage - a key somebody deleted, an external edit, a migration that did
not finish. Refusing the first would refuse every first launch; shrugging at the
second is how a setting disappears without a word.

Both discriminators are already on the disk:

| initialisation marker | in the recorded schema | path | outcome |
| --- | --- | --- | --- |
| absent | - | absent | `Seeded` - a first launch |
| present | **no** | absent | `Seeded` - the field is new in this build |
| present | yes | absent | **`Missing`** - it was written and is gone |

The middle row needs the schema snapshot, because the marker alone cannot tell a
deleted key from a field this version of the program has only just declared. It
is also exactly where *two defaults - one for a new install and one for an
existing one* belongs: a field absent because it is new, on a store that is not,
takes the for-existing value, and the commonest schema change of all stops
needing a migration step.

So the outcome is five, not four, and strictness is stated precisely rather than
"except for absence": `Missing` refuses, `Seeded` does not.

```rust
Outcome::Read
Outcome::Undecodable   // the bytes are there and will not read
Outcome::Refused       // read fine; the check said no
Outcome::Seeded        // was not there and was not meant to be
Outcome::Missing       // was declared and written, and is not there now
```

**And no `is_corrupted()` convenience over it.** Four of the five are different
decisions, and a word that collapses them collapses them in the reader's head
too - which happened twice while this entry was being written.

*A map entry is data, not a declared path.* One bad entry out of a thousand is
no reason to withhold the struct. Declared fields are strict; map entries are
dropped and reported.

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
| valid document, rubbish after it | refused | **no** |
| another format's content | refused, naming the format expected | **no** |
| root is a scalar | refused | `a_scalar_root_is_refused` |
| unreadable - permissions, a directory in the way | reported, not a panic | `a_path_that_cannot_be_written_is_reported` |
| metadata gone, data present | defaults must not come back over removals | `tamper_meta`, **ignored - open** |

**A declared path.** Whoever called `new_with` is the one told.

| what is wrong | should be | today |
| --- | --- | --- |
| prefix never written | every field `Seeded`, silent | yes |
| prefix written, a declared key deleted | `Missing`, refuses | **no** - reads as absent, seeds silently |
| prefix written, field new in this build | `Seeded`, silent | **no** - indistinguishable from the row above |
| value out of range | `Refused`, takes the default, in the report | `field_check`, `struct_check` |
| a leaf became a branch | refused | `a_leaf_that_became_a_branch_is_reported` |

**A map entry.** Nobody is told by refusing - these are data, and the struct is
still built.

| what is wrong | should be | today |
| --- | --- | --- |
| one key will not parse as `K` | dropped, name in `unreadable_keys()` | **no** - `continue`, silent |
| one value will not decode | dropped, key in `dropped()` | **no** - silent |
| *every* key fails | one line of drift at open: the key type changed | **no** - reads as an empty map |
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
| an edit adds an unparseable map key | appears in `unreadable_keys()` | **no** |
| a broken edit is not overwritten by us | left alone | `a_broken_external_edit_is_not_silently_overwritten`, **ignored - open** |

### Left on this: the repairing form, and the load that cannot report

**A struct's check refuses; it does not correct.** The repairing shape is
`Fn(&mut Schema, &Provided) -> Policy`, and what it needs is the reason it is
not built: a generated typed projection per struct, reachable by name -
`s.font_size()`, `s.net().host()` - with the shape of each accessor decided by
the `Role` the macro already reads off the type, built during the load, handed
to the check and then **kept by the instance** so `ui.schema()` afterwards is
the same object rather than a second type. That is a feature the size of this
one. `_Data` is not a substitute: it cannot say *which* path, cannot say what
happened to it - read, undecodable, absent-and-seeded - and collapses a map into
a `HashMap` where a dropped entry has nowhere to be mentioned.

When it is built, the corrected value does not go back to the disk. Writing it
back silently rewrites somebody's edit, and
`a_broken_external_edit_is_not_silently_overwritten` pins the opposite. Hold the
corrected value in memory and let the next ordinary write settle the file.

**And the one place the design does not close.** Under `mode = "persistent"`
there is no `Field`, so there is no `try_get`: a refused value under
`UseDefault` takes the declared default and the log is the only place it is
said. `Refuse` - the default - fails the load instead, and is the answer to
reach for when a loaded struct has to be trustworthy. Closing it properly means
`load_with` returning the values *and* what was wrong with them, which is a
second return type on every persistent struct.

A map's entries are still out: they are data rather than declared paths, so
`check` on a map field is a compile error until the drop-and-report policy in
the row above is built.

Two neighbours from the sector research belong with this and are not the same
thing: quarantining a file that will not parse at all, under a name that says so
rather than a silent default; and loading the fields that do read while
collecting the errors of those that do not, instead of refusing the whole
struct.

## A struct built at a runtime namespace is invisible to the schema layer

`Struct::new(store, "instances.a")` is the documented way to place a declaration
at a path decided at run time. `SchemaEntry.prefix` is `None` for such a struct,
and two mechanisms read that field and stop there.

**`Kv` writes over it.** `Kv::guard` reaches `schema_collision`, which walks
`inventory::iter::<SchemaEntry>` and skips every entry with no prefix, so
`kv.set("instances.a.port", &"oops")` is taken where the same write against a
compile-time prefix is refused - and the live field goes on reporting the `u16`
it declared. `Kv::clear` and `reset_to_defaults` decide what to keep the same
way, so they remove the struct's data while `Kv::clear`'s own doc promises the
declared paths stay. That half is silent data loss.

`Places` does know: `take` fills it at construction, per store rather than per
process. `Kv::cell` and `Kv::map` consult both; `set`, `remove`, `clear` and
`reset_to_defaults` consult only the inventory. Neither mechanism subsumes the
other, and settling this is choosing which of the two answers the question.

**The seeded defaults land in the plane and everything after in the tree.**
`new_with_id` builds every field and calls `record_schema` afterwards, so while
the defaults are being written the struct is in neither half of `Declared` -
not compiled in, since it has no prefix, and not recorded yet. `layout::levels`
therefore sends the seeding to the plane and every later write to the tree. The
file ends up holding `"instances.a.port"` beside `instances: { a: { port } }`,
one of them stale for good, and a scan lists the path twice on the document
engines and once on the flat ones.

Recording before building closes it. What that changes is when a snapshot is
written for a struct whose construction then fails - which `ensure_snapshots`
already does for every declaration in the inventory, constructed or not.

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

**Two mechanisms guard the same thing, and neither subsumes the other.**
`Places` refuses a place against what a constructor has already built;
`Kv::guard` refuses a write against what `inventory::iter::<SchemaEntry>`
declares. A declaration exists before anything is built, which is the case
`Places` cannot see; a struct hand-placed at a runtime namespace has
`SchemaEntry.prefix == None` and is skipped by `schema_collision`, which is the
case `guard` cannot see. One of them should be able to answer both.

**A map refusing a key more than one level below it** - `Level::Deeper` in
`decode_entry` and `scan_map` - stays outside the claim table on purpose. It is
the only mechanism that works against a writer no table knows about: a raw
`Store::set`, a migration, a person with a text editor. **The table prevents,
the read detects.**

## A map entry whose key will not parse as `K` disappears without a word

The scan walkers carry a malformed path up with the key attached, and
`generic_scan` logs the child it passed over at `warn`. `map_entries` does
neither. `primitives/map_ops.rs` skips an entry whose path yields no name, and
then skips one where `K::from_str` refuses the name:

    let Ok(key) = K::from_str(&key_str) else {
        continue;
    };

The first is the malformed-path case again. The second is not: the path is fine
and the **key does not parse as the map's key type** - a
`ReactiveMap<u32, _>` whose file holds `alpha` under it, after a hand edit or a
key type that changed without a migration. The entry is dropped from `entries`,
and therefore from the projection, `len` and `keys`, with nothing logged and
nothing returned.

So a hand-edited file can make a map quietly shorter, and the shape of the
failure is the one this crate keeps finding: a `continue` where a sentence
belongs. What to do with it is genuinely open, because unlike a malformed path
this is data the caller may have to be told about rather than a file this
library could not have written - the same question as the read-side policy
entry above, and probably answered with it.

## A renamed map is emptied rather than moved

Cleanup now takes what the declaration owned, entries and all
(`MigrationContext::drop_withdrawn`), and a rename is a drop as far as it is
concerned: the old place goes. What carries a value across a rename is
`AmeData`, which holds the scalar fields and no map - so a renamed
`ReactiveMap` arrives empty and its entries are gone rather than left behind.

Before, they were left behind at the old prefix on redb and sqlite and taken
with the node on the text engines, so nobody could rely on either. The question
is what a rename of a map should mean: move the subtree, or refuse the rename
and make the step move it by hand. `tests/migration_reactive_map.rs` does the
second already, hand-deleting `routes.{key}` in a loop, which is now a
workaround for a fault that is fixed and could go.

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

Nothing records that set today. `set_node` bumps a `writes` counter and emits an
event, and the path is already cloned there for the event, so the set goes in
beside `writes.fetch_add` and is cleared where `save_now` moves `persisted`.
Its size is bounded by the debounce window rather than by the store.

**The same set answers two other questions**, which is the argument for building
it once. What a store still held when it died, for the closing flush that fails
where nobody is left to be told. And what a save owes the file when the file
changed underneath it: the store rewrites the document whole from memory today,
so one buffered write discards every hand edit and every commit another `Store`
made in between - `tamper_live` and finding 5 of `RFC-text-atomicity.md`. Taking
the document from disk and laying only these paths over it is that fix, and it
needs exactly this list.

## The debouncer has two states and needs four

Alive and `is_poisoned`, and the second means a panic. There is no way to say
"stop taking work, write what is left, and be done", which is what closing
wants:

- after `shutdown()` the thread is still running and can schedule another
  flush, so the store is closed in the sense that matters and open in the sense
  that shows;
- a retry streak on the way out keeps retrying into a process that is about to
  end, where one report and a stop would do;
- "stopped because it was asked to" and "stopped because it died" are the same
  observable, and only one of them is a bug.

Not a fix for the static above - that needs the call either way - but it is
what makes the call mean something definite.

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

- `reactive_map_with_path<TScope, ..>` binds `TScope: StateScope` and never uses
  it; callers turbofish four parameters for nothing.
- `Kv::keys` returns absolute paths, where `ReactiveMap::keys` returns
  `Vec<K>`. It should return the names below the namespace. (It returns
  `Vec<StorePath>` rather than `Vec<String>` now, which is the type being
  honest, not the answer being right.)
- A leaf field with no `default` panics the proc macro
  (`generate/init.rs:115`), pointing at the attribute rather than the field, so
  a struct with ten fields does not say which one. The map and nested branches
  four lines above fall back to `Default::default()`.
- `get_map_types` decides a field is a map by matching the last path segment
  against the literal string `"ReactiveMap"`, so a type alias or a renaming
  import generates a scalar field instead. It does not reach disk: the `_Data`
  struct derives `Serialize` and `Deserialize` and `ReactiveMap` implements
  neither, so it stops at a compile error - an obscure one, about a missing
  `Serialize` in generated code, naming neither the field nor the reason. Make
  the misclassification say so itself: `shape.rs` already asks the compiler what
  a type is, where this asks how it was written.
- Every prefixed struct gets a generated `new()` that calls `global_store()`,
  so the most obviously named constructor is the one that panics when there is
  no global store. There is no `try_init_global`.
- `ReactiveCell::update`/`modify` return `SourceGone` for an absent map key,
  whose message sends the reader looking for a lifetime bug they do not have.
  `KeyNotFound` is in the same enum.
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

**Every engine discards its last flush on drop.** `let _ = self.close()` in
`redb/mod.rs:147`, `sqlite/mod.rs:516`, and `let _ = self.save_now()` in
`text/store.rs:178`. `close` is the only thing that commits the write buffer at
shutdown. redb's `close` even attaches "flushing the buffer before close", and
the attachment goes on the floor. `Drop` cannot return, but it can log.

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

**`entry_cell` turns a read failure into "the key is empty"**
(`reactive/entry_cell.rs:61`), which is the vocabulary the cell reserves for a
removed key. The real defect is the signature: `entry_cell` returns
`ReactiveCell<V>` with nowhere to put an error.

**Poisoned-lock fallbacks that silently disable a subsystem.**
`ReactiveMapCore::notify` fails open on a poisoned lock while the same file
uses `.lock().unwrap()` elsewhere - so a poisoned mutex makes `subscribe_any`
panic while `notify` quietly delivers to nobody, permanently. `Signal::emit`
answers the same question the other way, and only the map's side has a test
(`reentrancy.rs`). Neither policy is wrong; having both is.

The registry in `observability` has no lock to poison and so is not part of
this. What it has instead is one reader, `resolve_field`, called from a test and
nowhere else, over a map that only ever grows.

**`Kv::keys` breaks the `Kv` error type** (`store/kv.rs:204`): it returns
`StorageResult` where every other method returns `WriteResult`, so a caller
using `get` and `keys` in one function needs two error types.

## The text engines replace two files with no barrier between them

`RFC-text-atomicity.md` is the campaign that went looking for what that costs,
and holds what is still open with a test for each.

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

**An existing `.bak` is overwritten by an open that reads.** A copy is taken
once both files have read, so an open that is refused leaves nothing of its own
- but where the previous run left a good copy and the file it describes now
parses to a stump, the stump is copied over it. That is finding 2 in
`RFC-text-atomicity.md`, and it is the same missing idea as finding 3: nothing
compares the two copies before acting on them.

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
2238, 2248 and 2252 tests across what used to be three legs. Clippy restricts
per engine now, without `--all-targets`; a test run cannot.

`amethystate-reactor` is outside the workspace, depends on `amethystate` from
crates.io at `0.10.0`, and patches `windows-reactor` to a path outside the
repository - 11 tests, including the whole UI-thread marshalling contract, that
have never run here. `amethystate-gpui` is excluded from clippy, test and doc,
so the only thing its exclusion hides is whether it compiles. No leg passes
`--target wasm32-unknown-unknown`, which leaves the async arena, five
optimistic-rollback blocks in leptos, and `preload_slices!` never type-checked.

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

### The store's own defects, worst first

Not codec limits and not policy - logic in the store, and mostly small. Two of
these are silent data destruction through the public API.

| what | where | confirmed |
| --- | --- | --- |
| a path that computed to nothing is the root, and a struct written there replaces the whole document | shared `generic_set` | `tests/empty_path_is_the_root.rs` |
| reordering two same-typed struct fields silently swaps their values | the binary codec writes structs positionally | `tests/field_order_is_load_bearing.rs` |
| every enum loses its variant name on ron, so an app with an enum anywhere cannot start | `ron_doc.rs` reparses through `ron::value::Value` | measured |
| a scalar at a path 82 levels deep makes the toml file unopenable, and nothing reports it until the next start | path levels bypass `serialize_node` | measured |
| `rmp_serde` has no depth limit at all: a write commits and every later process aborts on a stack overflow | redb | measured, depth 4406 |

The empty-path one has a second half worth keeping in view. The same question is
answered four ways in one file:

```rust
generic_get([])            -> Some(root)         // read the whole store
generic_set([], node)      -> *root = node       // replace the whole store
generic_delete([])         -> Ok(None)           // do nothing
generic_delete_subtree([]) -> *root = empty_map  // erase the whole store
```

`generic_delete` is the only one that treats an empty path as not naming
anything, and it is the only one that is right. The fix is a
`StorePathError::EmptyPath` at construction, which settles all four at once and
leaves `StorePath::root()` as the way to say it on purpose.

### What redb keeps that the text engines lose

The negative result, and it is large enough to bound the category. Confirmed
through a reopen: non-finite floats bit-for-bit including the sign of `NaN` and
a `NaN` payload; `-0.0` keeps its sign; `Option::None` survives; `u128`/`i128`
exact, and narrowing is refused rather than wrapped; key encoding injective
across seven near-collisions; prefix scans stop at the level boundary in both
directions, including the siblings sqlite leaks on; no residue after a
write-then-delete; non-string map keys, which no text engine can hold.

So the representational half of the category belongs to the document formats.
redb's own two defects are structural instead - positional structs and no depth
limit - and neither is a codec limit.

### Depth, all five measured

| engine | limit | what it counts |
| --- | --- | --- |
| ron | 64 | path + value |
| toml | ~81 path, ~80 value | separately; they do not combine |
| json | 127 | path + value |
| sqlite | 254 | value only; the path is a `TEXT` key and costs nothing |
| redb | none | the stack ends around 3,200 on the read side |

Two of the three text engines already pay for a check by accident: ron refuses a
value past 64 at the write, and toml reparses the node in `serialize_node`. What
none of them check is the **path**, whose levels are built straight into the live
document and are met by the parser only at the next open.

## Accepted on the way in, refused or altered on the way out

A category, not a bug. A write returns `Ok`, and the read of the same path does
not give back what was written - because the codec, the document or the key
encoding will take something on the way in that it will not return on the way
out. Every instance below was found separately and filed separately, and they
are one shape:

| what | write says | read gives | where |
| --- | --- | --- | --- |
| nesting past `serde_json`'s 128 | `Ok` | the file does not open at all | `serializer_damage.rs` |
| the same value at a deeper path | `Ok` | the file does not open at all | `serializer_damage.rs` |
| `f64::NAN` on json, and on sqlite because it stores json | `Ok` | nothing - written as `null` | `non_finite_float.rs` |
| `Option::None` on toml | `Ok` | the node is not there | fixed; was a panic on `unwrap` |
| a key with escapes in it | `Ok` | a different path, or a residue node | `backend_conformance.rs`, 2 failing |
| clearing a map on a text engine | `Ok` | a node left behind | its own entry below |

Three severities, and the middle one is the worst to live with:

- **The file will not open.** Total, immediate, and at least it is loud.
- **The value comes back different or not at all.** Silent. Nothing in the
  application ever learns, and the wrong value is now the stored one.
- **Residue.** A path nobody wrote is readable, which is only visible to a scan.

What makes it a category worth naming is that the fixes do not compose. Each
instance has a cheap local fix - count the path's depth, refuse `NaN`, escape
keys differently - and the next instance is not covered by any of them. Only
reading back what was just written addresses the class as a class.

That was the argument for the round-trip flag, and probing all five engines
weakened it: most of what the category holds turned out to be defects in the
store rather than limits of a codec, and what remains of the representational
half is enumerable - non-finite floats, nested `Option`, depth. Those are worth
refusing by name at the write. The flag stays as an option for what nobody has
enumerated; the decisions above have the shape.

It also says where these belong as tests: the general form is a property -
what a store returns for a path equals what was written to it - and
`backend_conformance.rs` is already the place that generates values and paths
and checks exactly that. Two of its failures are members of this category and
are not currently read as such. Instances found elsewhere should end up there as
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

`a_value_the_writer_accepts_can_always_be_read_back` is `#[ignore]` with that as
its finding. The nesting is the instance; the class is any asymmetry between
what a codec will write and what it will read, and every text engine has its own
version of it.

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

So the candidates, in the order they should be considered:

- Weigh the path with the value: depth already spent by the path plus depth the
  value adds, against the reader's limit. The store knows the path at `set`, the
  arithmetic is free, and it is the only cheap check that is also correct.
- Read the whole document back after writing it. Catches this and every other
  asymmetry a codec might have, and doubles the cost of every flush.
- Bound depth at the encoder with a constant. Cheapest, and wrong in the same
  way as checking the value alone unless the path is counted.

The two are not alternatives and should not be one setting. The first is
arithmetic on a depth the store already knows, costs nothing measurable, and can
simply always run - a write it refuses was going to make the file unreadable.
The second parses the document it just rendered, on every flush, and is a real
price: it is the one that belongs behind a flag, off by default, for an
application that would rather spend the time than ever meet a file it cannot
open.

That flag goes where the rest of this is going. `StoreConfig` grew
`file_write: FileWritePolicy` for the retry budgets, and this is the same kind
of question about the same operation:

```rust
StoreBuilder::new(path)
    .file_write(|w| w.verifying(Verify::ByReadingItBack))
```

with `Verify::ByArithmeticOnly` the default. Naming not settled; what is settled
is that the cheap check is not a setting and the expensive one is.

The limits of the other four engines are not measured - only `serde_json`'s 128
is - so the arithmetic needs a per-codec number before it can be written, and
the codec is the right place to hold it.

Separately, `tests/atomic_write.rs` has an `#[ignore]` where the backup *is*
load-bearing - during an open - and is overwritten by the broken file it exists
to replace. That one is a defect in the scope described here, not an argument
for widening it.

## The book documents a library that is no longer there

Found by reading it end to end against the sources. Not a list of typos - these
are things a reader following the book cannot make work:

- `StoreBuilder::collect_migrations` and `amethystate::Result` do not exist.
- The migration pages destructure a report out of `build()`, which returns a
  store. `Migrations/overview.md` also documents a `~` row - `field 'port':
  u16 -> u32` - in the drift output, which `log_to_tracing` cannot print:
  `SchemaDiff` is `added` and `removed` only, and a type change under one name
  nags with no field named at all. That is deliberate; only the page disagrees.
- The dioxus and leptos pages name the provider component `amethystateProvider`;
  it is `AmeStateProvider`. The dioxus page uses both.

Rustdoc has its own: the macro says `default` is required on leaf fields, where
the code falls back to `Default::default()`.

What is left of the dependency ordering in `Migrations/overview.md` still has to
be revisited once the graph is demand-driven.

## What tampering with a text document does, found by doing it

`tests/tamper_*.rs` write a store, edit the file the way a person or another
tool would, and reopen. Every failing test asserts the behaviour that would be
right, so its failure message is the finding. Worst first; what is left here
loses data with no error at all.

The suite is ordinary tests now: what still fails carries an `#[ignore]` naming
the finding, and everything else is green. Every file but
`tamper_engine_contrast.rs` is gated on a text feature, and that one is the
control - on redb and sqlite it passes, which is the point of it.

**A section standing where a declared leaf is has no path that reaches it.**
`[cfg.width]\npx = 800` where `cfg.width` is a declared `u16`: the shape is
reported, and `Cfg::new_with` refuses, which is the half of
`tamper_shapes::a_leaf_that_became_a_branch_is_reported` that holds. The other
half asks `get(["cfg","width","px"])` for `Some(800)`, and no flat engine could
ever answer it: a path inside a declared value is not in the tree, so it is a
plane key on all five, and `"cfg.width.px"` is not the section the file holds.
Reachable only by deciding that a document engine may look inside a value its
schema says is a leaf, which is the identity between the two families given
away. Left parked on that question, not on a defect.

**Losing the metadata file, for a namespace nothing declares.** It is a plane of
whole keys with no level of its own, so emptying it leaves nothing to read and
the `__init` marker in the lost file is all there was - a removed default comes
back. A declared level answers for itself, which is the rest of `tamper_meta.rs`
and holds; this case has nothing to answer with.
`losing_the_metadata_file_does_not_resurrect_removed_defaults` stays parked on
it.

**An unrelated pending write rolls back a concurrent external edit.**
`pull_external_changes` hands off to `watching::take_outside_edit`, which holds
the edit while `writes != persisted` - `Standing::Unsaved` - and a persist
writes the whole document from memory, so one buffered write anywhere discards
every hand edit, including to untouched keys. It says so at `warn`, and that is
the whole of what anyone is told. `tamper_live.rs`.

**A broken external edit is dropped without a word and then overwritten.**
`D::parse` fails, the look answers `Taken::Unreadable`, nothing reaches the
caller, and the next save replaces the half-written file. The log is again the
only place it is said.

## What the conformance suite does not ask

`tests/backend_conformance.rs` states twenty-nine properties and runs each
against every engine compiled in; all of them hold on all five. Uncovered:
concurrency between two handles, the async surface, `is_initialized` across a
failed flush, and value shapes past `u32`/`String` - nested structs, enums and
sequences are where the three text formats differ most from each other and from
msgpack.

## Documentation

**`Watch::stream` has no doctest.** `reactive/watch.rs` carries three, and all
of them sit above `register_with_source`; `stream` is the last public method in
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
| `entry_cell` doctest | a write to a removed key recreates it | the `ReactiveCell` rework |
| `ReactiveCell` methods | `get` returns `T`, never absence | the same |
| `ReactiveMap::len`, `entries` | the cost is a scan, and `take(1)` saves nothing | reads moving to the projection |
| `Field::durable` | what each engine family commits, and that a text backend is accidentally stronger | `flush_prefix` becoming per-write |
| `Store::decode` | corrupt bytes yield `Default` with a warning | settling the split against `get`, which errors |
| `Field::set`, `ReactiveMap::insert` | every write reaches the store | value dedupe |
| `Kv::cell` | a path's type is remembered for this run only, and a second type is refused | `check_type` becoming persistent, which also puts an `AmeType` bound on the method |
| `Kv::set`, `Kv::get` | any type at any path, unchecked | the same |

Sorting is documented on `keys` and pointed at from `entries`: the order is the
store's, over the key's string form, so numeric keys come back `10, 100, 9`.
That one is not expected to change.

**The migration context needs a written-up page of its own.** Its methods carry
doc comments and `StoreBuilder::provide` has a runnable example, but there is
nowhere that explains the shape of a migration as a whole - and it is the part
of the library a person meets exactly once, under pressure, with data they
cannot afford to lose. What it should cover:

- what a step is: a bare `fn` collected at link time, capturing nothing, which
  is why anything from the application arrives through `provide`/`require`
  rather than a closure;
- the difference between `build` and `build_with_migration` - only the second
  collects the steps `#[migrate]` generated, which is its own entry above and
  is the first thing that bites;
- reading old data (`AmeData`), the scoped forms (`nested`, `scoped`), and
  which of `get`/`global_get` addresses what;
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

*The page shows what runs, not what came out.* The measured table is printed by
the run and the page still carries a copy of it written by hand. That is the
same drift `//@act` closed for code, left open for output: nothing checks that
the table on the page is the table the test produced.

*Generation reads source, never a run.* `cargo xtask docs` parses text. It will
happily publish a region guarded by `#[cfg(feature = "toml")]` from a checkout
where toml is off and the test has never executed. So a page can assert
something no run verified, and `--check` will call it up to date. Closing this
means generating from a test run - captured output keyed by test name - rather
than from a file, and it is the one that turns the pipeline from a formatter
into infrastructure.

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

## A `close` whose flush failed answers `Ok` the second time

Every backend opens `close` the same way:

```rust
if !self.debouncer.stop_accepting() {
    return Ok(());
}
```

`stop_accepting` is `!self.stopped.swap(true, ..)` - a one-way latch carrying
one bit. The first `close` takes it, and if the flush that follows fails, the
error is returned and the latch stays set. A second `close` finds it already
set and returns `Ok(())` without flushing anything: success reported for data
that is not on the disk.

That is what makes a retry impossible after the failure a caller was told
about, and it is why `Store/opening.md` and `Store`'s doc comment used to offer
"offer to retry, save elsewhere, or not exit yet" - none of which a caller can
do. Those sentences are corrected; the behaviour is not.

The one bit is the whole problem. Three backends layer three meanings on it -
closed, closing, mid-flush - and there is no state for "closing was attempted
and did not finish". Four phases would carry it: open, draining, detached,
closed-and-drained, with a failed drain landing somewhere a retry can act on.

