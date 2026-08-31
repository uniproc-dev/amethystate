# RFC: `ReactiveTable`

A table is the one shape this library does not cover. `Field` holds a value,
`ReactiveMap` holds a keyed set, `ReactiveCell` and `entry_cell` are views onto
one of those, and `Kv` addresses any of them by path. None of them has an order,
and a table is an order over rows before it is anything else: a GUI renders row
7 through row 34, sorts by a column the user clicked, hides the rows a filter
rejects, and needs to be told which few positions moved rather than that
something changed.

This proposes the smallest primitive that answers that, and spends most of its
length on the two parts that are expensive to change afterwards - what is
written to disk, and what a subscriber is written against. Everything else is
deliberately left thin, with the seam named where a richer version would attach.

## What a user does today

A `ReactiveMap<String, Task>` and an order kept beside it by hand. That works,
and it is worth being precise about how far, because the proposal is only worth
its weight past that line.

`ReactiveMap` is resident: `reactive_map_with_path_only` scans the prefix once
at construction and decodes every entry into `ReactiveMapCore::cache`, and every
read afterwards answers from there. So `get`, `len` and `contains_key` are
already frame-rate reads - `map_len_vs_size` measures 680 ns flat from 10 to
10 000 entries. Nothing about residency is the problem.

Three things are.

**`entries()` clones and sorts the whole map per call.** It collects
`(String, K, V)` for every entry, sorts with `cmp_names`, and hands back an
iterator. A render function that wants rows 7 to 34 pays for all of them, every
frame, including the value clones. There is no way to ask for a range.

**The only order is the key's stored form.** `entries` and `keys` sort by
`cmp_names`, which is the order the store's own scan produces. That is the right
default and it is documented, but it is the only one: sorting by a field of the
value means the caller rebuilds a `Vec` and sorts it, which is the previous
problem again, now with a comparator.

**`MapChange` carries no position.** A subscriber is told
`Insert { key, value }` and has to work out where that key belongs in whatever
order it is maintaining - which means it needs the order in hand, which means it
kept its own `Vec`, which means the map's projection and the caller's vector are
two copies of one fact with nothing keeping them in step. That is the same shape
as the defect recorded in `TODO.md` under "The map already projects itself into
memory, then ignores it", moved out of the library and into every application
that needs a list.

The hand-rolled version therefore works up to the point where the list is short
enough to redraw whole. Past a few hundred rows it stops, and it stops by
getting slower rather than by failing, which is the worst way for a limit to
arrive.

## Non-goals

These are structural, not caveats. Each one is a thing the design is built so as
not to need, and where a later version might want it, the section that would
have to change is named.

**No query language.** Nothing parses a string. Filters and comparators are Rust
closures over `&R`, checked by the compiler, and they run in this process
against decoded values. A query language needs a parser, a name resolver, a type
for expressions, and an answer to what happens when a query names a column the
struct no longer has - four problems this library has no reason to own. The
closure has the opposite property: a column that is gone is a compile error at
the closure.

**No query planner, and no joins.** There is one access path - the row id - and
one order per view, chosen by the caller. Nothing chooses between paths, so
there is nothing to plan. A join across two tables is the caller writing a loop
over one view and looking rows up in the other table by id, which is `get` on a
resident projection and costs a hash lookup. Making that implicit would require
cost estimates, statistics, and a notion of which of two tables is smaller -
none of which exist here and none of which a GUI list needs.

**No aggregates, no group-by.** Counting, summing and grouping over a resident
projection is `view.iter().filter(..).count()` and the caller can write it.
Pushing it down would require the engines to compute, and three of the five are
a text document.

**No transactions beyond what the store already offers.** A write reaches the
buffer and subscribers immediately, and the debouncer commits it later; a write
through `durable()` returns once it is on disk. There is no multi-row atomic
write and none is proposed. `TODO.md` records that a durable write already
commits a different amount on each engine - `flush_prefix` commits a prefix on
redb and sqlite and the whole document on json, toml and ron - so the granularity
the library can promise today is one write, and building a table-level
transaction on top of that would be promising something the layer underneath
does not have.

**No schema of columns at run time.** A row is a Rust struct. There is no
`add_column`, no column type registry, no dynamic row. What the disk holds is
described by the four-layer record `TODO.md` settles on - path, role, shape,
version - and the table adds nothing to it.

**No pagination cursors, no server semantics, no offset stability across
writes.** A view's indices describe the view as it is now; a write that changes
the order changes them, and the change notification says exactly how. There is
no snapshot isolation and no stable cursor across concurrent writes, because
there is one process and the writes are its own.

**No unique constraints, no foreign keys, no cascades.** The store has one
uniqueness rule, which is that a path holds one value, and the row id is that
path. Anything else is the caller's invariant, and the caller already has the
place to enforce it: an interceptor can refuse a write, and refuses it before
the value reaches the buffer.

**No reactive virtualised list over a table too large to hold.** The size
envelope this library is aimed at is a million rows at the ceiling and a hundred
thousand where it is already expected to be uncomfortable, and the section below
works out what the resident design costs at both. What is not proposed is the
combination that would reach past it: a list addressed by scroll offset, kept
live over rows it does not hold, ordered by an arbitrary column. That needs a
secondary index, a maintained total, and fallible reads on the one path that
must not have them. The non-resident work that *is* proposed is a different
shape - a cursor and the windows it loads - and it is designed from the jobs it
actually serves rather than from the list.

**No non-resident form on the text engines.** A json, toml or ron document is
one file parsed whole into memory before anything in it is addressed, so it is
resident by construction and nobody puts a hundred thousand rows in one. A
streaming API over an in-memory tree would be a promise the engine does not
keep. This is a scoping decision, and what a table on a text engine is instead
is stated plainly below.

## Shape

Two types, and the split between them is the whole design.

**`ReactiveTable<R, M: AccessMode = ReadOnlyMode>`** is the stored thing: rows
keyed by id, resident, subscribable, interceptable, writable when `M` is
`WritableMode`. It has no order beyond the store's own, and no filter.

**`TableView<R>`** is one order over that table, with one filter. It holds no
rows of its own - only a `Vec<R::Id>` - and it maintains that vector
incrementally from the table's changes. A table may have any number of views; a
sorted column header and a search box are two views or one rebuilt, and either
is cheap.

The split exists because ordering is a property of a screen, not of the data,
and because it puts the only expensive thing - a sort - in a place the caller
creates and drops deliberately.

Both are resident, both are reactive, and neither may touch the disk.

Two more types serve the work that is not on the drawing thread.
**`TableCursor<R>`** is a loader: it holds nothing, reads in key order, may
block, and is fallible per row. **`RowWindow<R>`** is what it hands back - a
resident view over one key range, infallible to read, which is the only thing
the drawing thread ever sees. The cursor touches the disk so that nothing in a
render function has to.

| | touches disk | may block | reads | live over | writable |
| --- | --- | --- | --- | --- | --- |
| `ReactiveTable`, `TableView` | at open only | no | infallible | the whole prefix | yes |
| `TableCursor` | every call | yes | `Result` per row | nothing | no |
| `RowWindow` | at load only | no | infallible | its key range | no |
| `Kv::namespace` (exists) | every call | yes | `Result` | one path, via `cell` | yes |

The next three sections are why the split falls there, why the cursor is a
cursor rather than an index-backed window, and how a window stays live without
having to know anything about the rows outside it.

## Residency is a property of the backing, not of the primitive

Residency is not something a table would introduce. `ReactiveMap` holds
`cache: Arc<DashMap<K, V>>`, filled by one scan at construction and kept current
by the store subscription, and every read answers from it - the same mechanism a
table needs, with the same limits, over the same key layout. So the question of
what to do about a collection too large to hold belongs to the backing that both
share, and any answer shaped as a trait inside the table would have to be
dragged out to the map the first time the map met the same size.

**The seam that is real is on disk, and it already has three occupants.**
`ReactiveMap` at `<path>.<name>`, a `ReactiveTable` at `<path>.<id>`, and
`Kv::namespace(name)` addressing the same level by path - all three write one
segment per entry under one prefix, sorted by `cmp_names`, seeded through
`is_initialized`. That is the contract worth writing down, and writing it down
records a fact rather than adding a mechanism. It is what makes a map
exchangeable for a table with no migration, and it is why the contract should be
stated now even though nothing new implements it.

**The non-resident keyed collection already exists, and it is `Kv`.**
`Kv::namespace("columns")` gives `get`, `set`, `remove` and `keys` over exactly
the same level a map occupies, reading through to the store every time and
returning `WriteResult` because it genuinely touches disk; `Kv::cell` adds a
subscription over one path. What it lacks against a map is the collection-level
answers - a length, an ordering, one notification for the collection rather than
per path - and those are precisely the things residency buys. So "a windowed
`ReactiveMap`" is not an unbuilt type. It is `Kv` with `MapChange` bolted on,
and the honest way to want it is as an addition to `Kv`.

**The rule underneath residency is that the drawing thread must not touch the
disk.** That is what the projection buys, and it is why the library is shaped
the way it is elsewhere: reads are infallible because they cannot fail if they
never leave memory, writes are buffered rather than committed, and
`RedbStoreInner::db` answers a read with an error while the handle is being
traded rather than blocking on it - `TODO.md` says so in as many words, that a
read or a scan "sees the `None` and is told, rather than blocking a UI thread on
a file operation".

So the two sides differ in **blocking discipline** before they differ in
anything else, and that is the strongest argument against one type
parameterised by a backing: a single type would have to impose the union of both
sets of constraints on both. The resident side may not block, so the parameterised
type may not; the non-resident side may block and must be allowed to, so the
parameterised type's contract cannot say it does not. There is no honest
combined answer.

The return type is the visible half of the same thing. The six reads that just
stopped returning `Result` - `get`, `contains_key`, `entries`, `keys`, `len` and
`is_empty` - were `Ok(..)` around an infallible projection lookup, and the cost
of the wrapper was an `.unwrap()` on the line a GUI writes most often. A
`Collection<K, V, S: Backing>` either returns `Result` on every read, paying
that cost back on the resident path for a capability it does not use, or hides
the difference behind an associated `type Read<T>` that is `T` in one case and
`StorageResult<T>` in the other - which compiles, and makes every error message
and every doctest a two-headed thing.

And the method sets differ, not merely their wrappers. `remove` now takes
`&Q where K: Borrow<Q>` and lifts the owned `K` out of the projection, so a
removal costs no allocation and the change still carries a real key; without a
projection there is nothing to lift it out of. A change carrying `old_value`
is free from a projection and a read from disk. An interceptor that checks the
collection for a duplicate can do so resident and cannot otherwise. A single
type would have half its surface conditional on a parameter.

**What `len` means is not part of this, and the distinction matters.**
`ReactiveMap::len` is documented as how many entries the map holds, answered
from the projection and counting buffered writes that have not landed - a
statement about the view, not an inventory of the store. A resident view over a
subset holds fewer and says so, and that is the same statement told truthfully.
Nothing about a smaller view makes `len` or `keys` misleading, and no part of
this design is arranged around a hazard there. What a subset genuinely cannot
answer is a **total** - how many rows exist altogether, which a scrollbar and a
"showing 1-50 of N" both need - and `len` never promised it. That is an
affordance the windowed form has to add, not a defect it has to work around, and
it is taken up under [the total](#the-total-and-what-it-costs).

## Row identity

**A row id is exactly one `StorePath` segment.** Not a path: `StorePath` builds
from segments and a separator inside one is part of the name, so an id may
contain a dot and stays one level. It may not be empty, which is the one thing
`StorePath::try_segment` refuses.

**The library assigns ids, and the caller may supply its own.** `insert(&row)`
generates one and hands it back; `insert_with(id, &row)` takes the caller's.
Both are needed: a row created by a button press has nothing to be named after,
and a row mirroring something that already has an identity - a file path, a
device serial - should keep it.

**A generated id is `Uuid::now_v7()` in its hyphenated form.** The first 48 bits
are a big-endian millisecond timestamp and the rendering is fixed-width
lowercase hex, so lexicographic order over the string is time order, and the
store's own key order is therefore insertion order. That matters more than it
looks: the engines can order by key and by nothing else, so the one order
available for free should be the one a list wants by default. Within a single
millisecond the remainder is random, so two rows created in the same millisecond
have an arbitrary but stable order between them.

`uuid` is already a workspace dependency, already re-exported from the facade,
and already the source of `instance_id`; this needs its `v7` feature added
beside the `v4`, `js` and `serde` already enabled.

**This one is hard to reverse.** The id format is written into every key, so
changing the generator later does not break reading - nothing parses an id - but
it silently scrambles the default order across the boundary, because old ids and
new ids no longer interleave by time. A ULID would be ten characters shorter per
key and costs a dependency; a monotonic counter would be shorter still and
cannot be made safe with two processes on one file, since the next number is
derived from what is already there and two writers derive the same one. Pick
`now_v7` and do not revisit it.

**Order over caller-supplied ids is the store's, not the id type's.** Escaping
does not preserve order - `cmp_names` documents why: `.` and `\` both become a
pair beginning with `\`, so `a.b` sorts after `a1b` though the bare names sort
the other way. A view with no comparator orders by `cmp_names` over the id,
which is exactly what a scan produces, so a listing and a view agree. An id
type that is a number therefore lists as text - `10, 100, 9` - which is already
true of `ReactiveMap::keys` and is documented there. A caller who wants numeric
order zero-pads, or supplies a comparator.

**The id is the key and only the key. It is never inside the row.** Two copies
of one fact can disagree and there is nothing to resolve it with; the store
already owns the key, so the value should not repeat it. Every read hands back
`(R::Id, R)` rather than `R`, so a render function has the id without the row
carrying it.

**The id type comes from the row trait, not from a parameter on the table.**

```rust
pub trait TableRow:
    Serialize + DeserializeOwned + Clone + Default + Send + Sync + 'static
{
    /// What names a row. One `StorePath` segment.
    type Id: RowKey;
}

pub trait RowKey: FromStr + Display + Clone + Hash + Eq + Send + Sync + 'static {}
impl<T: FromStr + Display + Clone + Hash + Eq + Send + Sync + 'static> RowKey for T {}
```

`RowKey` is `ReactiveMapKey` under another name, blanket-implemented the same
way, so `String`, `u64` and `Uuid` are all ids without an impl. It is a separate
trait rather than a re-export so the two can diverge later without a breaking
change.

Putting `Id` on the row trait rather than writing `ReactiveTable<R, I, M>` is
the point. A third parameter cannot be added later without breaking every
`ReactiveTable<Task, WritableMode>` already written, since `M` would move; an
associated type is there from the start at no cost to the common case. Rust has
no stable associated type defaults, so every row spells `type Id = RowId;`,
where `RowId` is the newtype around a generated `Uuid`. That is one line of
ceremony bought against a parameter list that can never change.

## The on-disk layout

This is the part that has to keep loading data written by an earlier version, so
it gets the most care.

### What is written

One key per row, under the table's path, named by the id:

```
tasks.items.0199a0c3-4d8b-7f21-9a10-6c4b2f8e1d33   -> the whole row, encoded
tasks.items.0199a0c3-4d8c-7a44-b3e2-11f9c7a05b62   -> the whole row, encoded
```

That is byte for byte the layout `ReactiveMap` already writes. It is the same
layout deliberately, and the consequences are all good ones: an existing
`ReactiveMap<String, Task>` field becomes a `ReactiveTable<Task>` with no
migration and no rewrite, a table that turns out not to need an order goes back
the same way, and `Role::Map` in `FieldDescriptor` already describes it
correctly - "a map's entries live one level under here; this path itself holds
nothing" is the exact rule a reader needs. No new role is required, and the
"structure where a schema declares it, flat keys everywhere else" decision in
`TODO.md` covers a table without amendment: the reader descends one level and
takes each entry whole.

**Nothing else is written in the data space.** No order vector, no index, no row
count, no sidecar of any kind beside the rows.

### The reserved space, named

A table will eventually want to persist something that is not a row - a
maintained sort order, a count, an index. That must not go in the data space,
because every segment under the table's path is a legal row id and there is
nothing to reserve: a row could be created with any name a sidecar might use.

**A table's bookkeeping goes in the metadata space, keyed by the table's path**,
beside `PrefixMeta`, `SchemaSnapshot` and the migration log, which are already
per-prefix records at `meta.<prefix>` and are already flat by decision. A row id
cannot collide with it, because the two spaces are different tables on redb and
sqlite and a different file on json, toml and ron.

That placement carries one rule, and the rule is what makes it safe:

> **Anything a table stores in the metadata space must be reconstructible from
> the rows alone.**

`TODO.md` records that the metadata file on a text engine is a second file with
nothing binding it to the data, that losing it replays migrations, and that the
fix under consideration is a checksum that reads a crash between the two writes
as a divergence. An index that is the truth would turn that divergence into
data loss. An index that is a cache turns it into a rebuild on the next open,
which is a startup cost and nothing else. So the rule is not tidiness; it is
what lets the sidecar live in the one place where a row id cannot reach it.

Version one writes nothing there. Reserving it costs nothing and is the seam
every deferred feature below attaches to.

### How it behaves on each engine family

| | redb, sqlite | json, toml, ron |
| --- | --- | --- |
| a row | one key, stored whole | one node one level under the table |
| listing the rows | `range(prefix..)` to the subtree bound, whole subtree | the document walk, direct children only |
| row order in a listing | the joined key, byte by byte | the same, after the walk sorts |
| a row id holding `.` or `\` | escaped into the one key, survives | one level, escaped on the way out |
| deleting the table | `delete_prefix` over the range | the node goes, and its subtree with it |
| what is left after `clear` | nothing | an empty node at the table's path |

The text engines' one-level scan depth, which `TODO.md` records as a real
divergence (`scan_prefix_impl` sets `target_depth = parts.len() + 1`), is
harmless here for exactly the reason it is harmless for a map: a table's rows
are always precisely one level below its path, so the depth a scan reaches never
matters. This is the single strongest argument for row-as-one-value over a key
per column, and it is worth stating as such - a layout of
`tasks.items.<id>.<column>` puts the values two levels down, where a scan of the
table on json, toml and ron cannot see them at all.

**What the text engines cannot do here, plainly.** They cannot hold a large
table. The whole document is parsed into memory, and every write rewrites the
whole file: `flush_prefix` on a text engine ignores its argument and calls
`save_now`. So one row changing costs a serialisation and an atomic replace of
every row. That is fine for a table of settings-sized data a person might edit
by hand, which is what the text engines are for, and it is wrong for the ten
thousand rows the flat engines handle without complaint. The library should not
refuse it - a text store with a big table works, it is merely slow in a way the
user can measure - but the documentation has to say it in one sentence: a table
on a text engine costs a full file rewrite per flush, and its size should be the
size of something a person would open in an editor.

The second thing they cannot do is leave nothing behind. `clear()` on a text
engine leaves an empty node at the table's path, which a scan then reports as a
stored key. `load_map` already skips a scanned key equal to the map's own path
for this reason, and a table's loader must do the same. `TODO.md` explains why
the node is not litter: it is the one bit that distinguishes a table that was
emptied from one that never existed, and the initialization markers depend on
it.

### How the layout grows

The row type is a Rust struct that a codec serialises, and the five engines do
not agree about what that means. Only redb encodes structs positionally;
`RedbStore` builds `Serializer::new(..).with_bytes(BytesMode::ForceAll)` and
does not call `with_struct_map`, so `struct Task { title: String, done: bool }`
is a two-element msgpack array with no names in it. sqlite encodes with
`sonic_rs` JSON, and json, toml and ron are name-keyed documents. So four
engines out of five key a row's columns by name and one does not - and the one
that does not is the default.

What follows was read out of the encoders rather than assumed.

| change to the row struct | redb | sqlite, json, toml, ron |
| --- | --- | --- |
| a column appended, with `#[serde(default)]` | old rows decode, the new column takes its default | the same |
| a column appended, without a default | every old row fails with `invalid_length` | old rows decode, the column takes `Default` only if asked |
| a column inserted anywhere but the end | every later column is read from the wrong position, no error | no effect |
| a column removed | **every old row fails to decode** | old rows decode, the column is ignored |
| a column renamed in Rust, with `#[serde(rename)]` unset | free, the position is unchanged | every old row loses that column's value |
| two columns exchanging types | read with the decoders swapped, no error | no effect |

The two rows that matter are the two in bold type in practice. An appended
column with `#[serde(default)]` is free on every engine, because rmp-serde's
`SeqAccess::next_element_seed` answers `Ok(None)` once the array is exhausted
and serde's derived `visit_seq` fills the field from `Default::default()` -
`expr_is_missing_seq` in `serde_derive` emits exactly that when the field has a
default and `Error::invalid_length` when it does not. A removed column is a hard
failure on redb and cannot be softened by any serde attribute, because
`any_inner` checks the remaining element count after the visitor returns and
answers `Error::LengthMismatch` when the array had elements nobody consumed.

**This is the most expensive decision in the RFC and it is not a decision the
table can take on its own.** Two ways out:

- **Give redb `with_struct_map`.** Rows become name-keyed like every other
  engine, and appending, removing, reordering and renaming all behave the way
  the other four already do. The cost is size - every row carries its column
  names - and the cost lands hardest exactly where it is felt, on a table with
  many rows. It also changes redb's encoding for everything, not only tables, so
  every existing store is rewritten once. `TODO.md` weighs the same trade under
  the schema hash entry and records the cost as real on both sides.
- **Write down the rule that columns may only be appended, and only with a
  default.** No format change, no rewrite, and the rule holds until somebody
  forgets it, at which point the failure is either a decode error naming the
  table or - for an inserted column - values read from the wrong fields with
  nothing said.

The table is the strongest argument yet for the first, because a table is the
one primitive whose value type is expected to change shape repeatedly over an
application's life. And the moment to take it is before any table data exists,
since `TODO.md` has already accepted that changing the format now costs one
run of drift on the author's own machine and nothing else. That window closes
the first time anyone stores a table.

A narrower version was considered and is worse: choose the encoding per path,
`with_struct_map` under a `Role::Map` and positional elsewhere. It needs the
role threaded into `set_erased`, which takes a path and an erased value and
knows nothing about schemas, and it leaves one store holding two encodings that
nothing on disk distinguishes.

### What the migration engine can and cannot carry

| the change | needs a step? | what the step does |
| --- | --- | --- |
| a column appended with a default | no | nothing; old rows decode |
| a column removed, on redb | yes | read every row, write every row back |
| a column's meaning changed | yes | read every row, transform, write back |
| the row id format changed | yes | read every row, write under the new key, delete the old |
| the table renamed or moved | yes | the same, and the old subtree has to go |
| a table field dropped from the schema | **yes, and cleanup is broken today** | see below |
| a secondary index added | no | the first open builds it |

`MigrationContext::scan_map::<K, V>` reads a whole map at a key as a
`HashMap<K, V>`, and its doc already says why it is strict: a step reads,
changes and writes the collection whole, so an entry that cannot be read is an
entry the migration would delete. That is exactly the operation a table
migration needs and it already exists.

Its limit is that it is whole-table and in memory. A step over a hundred
thousand rows decodes all of them into a `HashMap`, in one transaction, before
anything is written, and at the stated ceiling of a million that is the open
cost worked out below plus the whole table held twice. A chunked form is the
same walk the cursor makes, over the same `scan_range`, so the two wants are one
addition rather than two - and a bulk migration is one of the scenarios that
motivates the cursor in the first place.

Two known defects bite a table harder than they bite a map, and both are already
written up in `TODO.md`:

**Cleanup deletes one key.** `ctx.delete(field.name)` removes a single key,
while a map or table field's data lives at `prefix.field.<id>`. So dropping a
table field from the schema leaves every row on disk, on redb and sqlite; the
text engines delete the node and take the subtree. For a map of a dozen settings
that is a leak; for a table it is the bulk of the store, kept forever, invisible
to everything. Renaming such a field is worse - the rows are written at the new
path and the old subtree stays, so there are two live copies.

**A step is invisible through `StoreBuilder::build`.** `build` does not call
`collect_codegen`, so an application that opens with it runs no generated
migrations and reports success. A table is the primitive where running new code
against old rows is least likely to be noticed, because one undecodable row is
enough to fail the open and the message will be about a codec.

Neither is caused by this proposal and neither should be fixed by it, but a
table should not ship before the first is fixed.

### Drift

The schema hash does not see a row's shape change, and will not be made to.
`FieldDescriptor::type_hash` folds with XOR, so two columns exchanging types
cancels, and `TODO.md`'s "Decided: the library guarantees paths, and says
nothing about types" removes the type layer entirely: what is recorded is the
path, the role, the shape read off the disk, and the author's version number.
For a table that is the right set. The path set catches a table being renamed or
dropped, the role says its children are entries, the shape says what an engine
actually stored, and nothing pretends to describe the Rust struct.

What catches a column change is the read, on open, and that is where the table
has a genuine fork of its own.

## Strict open, and the fork under it

`load_map` is strict: "a key under this path that cannot be read back is an
error rather than an absence", because answering short means the caller acts on
a collection that is missing something the store holds. For a map of ten
settings that is plainly right. For a table of ten thousand rows it means one
hand-edited row, or one row written before a column was removed, fails the whole
construction of the state struct and the application does not start.

Version one is strict, matching the map. Two reasons, and the second is what
makes the first comfortable.

The loosening is forward-compatible and the tightening is not. A later version
that quarantines a bad row, keeps it on disk untouched, excludes it from every
view and reports it through `table.rejected()` turns a hard failure into a soft
one, and nothing that worked before stops working. Going the other way would
break applications that had learned to start.

And the tolerant reader already exists as a different type. `TableCursor` yields
`StorageResult<(R::Id, R)>` per row, so a pass over a table with three bad rows
returns three `Err`s among the `Ok`s and keeps going - which is exactly what a
repair or an integrity check wants, and exactly what an all-or-nothing open
cannot give. So strictness on the resident path is not the library refusing to
cope with a damaged table; it is the resident path declining to guess, with the
loose reader one type away.

## The change model

Two types, because two different things happen and only one of them has a
position.

```rust
#[non_exhaustive]
pub enum RowChange<R: TableRow> {
    #[non_exhaustive] Insert { id: R::Id, row: R, source: Option<Uuid> },
    #[non_exhaustive] Update { id: R::Id, old: R, new: R, source: Option<Uuid> },
    #[non_exhaustive] Remove { id: R::Id, row: R, source: Option<Uuid> },
    #[non_exhaustive] Clear  { source: Option<Uuid> },
}
```

`RowChange` is what happened to the table. It is what an interceptor sees, and
what a subscription on the table itself delivers. It carries no index, because
a write has no position: there may be any number of views over the table with
different orders, and before the write lands there is no order it belongs to.
The shape is `MapChange`'s, deliberately, so the two read alike.

```rust
#[non_exhaustive]
pub enum ViewChange<R: TableRow> {
    #[non_exhaustive] Inserted { id: R::Id, row: R, at: usize, source: Option<Uuid> },
    #[non_exhaustive] Updated  { id: R::Id, old: R, new: R, at: usize, source: Option<Uuid> },
    #[non_exhaustive] Removed  { id: R::Id, row: R, from: usize, source: Option<Uuid> },
    #[non_exhaustive] Moved    { id: R::Id, row: R, from: usize, to: usize, source: Option<Uuid> },
    #[non_exhaustive] Reset    { source: Option<Uuid> },
}
```

`ViewChange` is what happened to one view's order, and it is the type a
virtualised list is written against. The index semantics have to be pinned in
version one and never moved, because every subscriber is written to them:

- `Inserted.at` is the index the row now occupies, in the order after the
  insertion.
- `Removed.from` is the index the row occupied in the order before the removal.
- `Updated.at` is the row's index, which is the same before and after - an
  update that would move the row is a `Moved`, not an `Updated`.
- `Moved.from` is the index before, `Moved.to` the index after the removal at
  `from` has been accounted for. This is the convention a list widget's
  `row_moved` already takes, so a subscriber forwards it unchanged.
- `Reset` says every index is meaningless: re-read from zero. It is what
  `clear()`, a changed comparator, a changed filter, and an external edit that
  replaced the document all produce.

A filtered view reports a row that stopped matching as `Removed` although the
row is still in the table, and a row that started matching as `Inserted` though
it was already there. That is the right answer for a list - the row left the
screen - and it has to be said out loud, because `RowChange` and `ViewChange`
then disagree about the same write, on purpose.

### What a version-one subscriber still does under later versions

Both enums and every struct variant are `#[non_exhaustive]`, so a subscriber
must write a wildcard arm and must use `..` in its patterns. That is the cost -
one arm that would otherwise be a compile error when a variant is added - and it
buys the ability to add a variant or a field without breaking anything already
written.

`MapChange` is not `#[non_exhaustive]` today, and its own doctest matches all
four variants exhaustively, so the map's change model is frozen. The table
should not repeat that.

`#[non_exhaustive]` alone is not enough, because a wildcard arm is silent: a
subscriber that ignores a new variant compiles and does the wrong thing. So the
compatibility rule is stated as a constraint on what may ever be added:

> **A variant may only be added to `ViewChange` if a subscriber treating it as
> `Reset` is correct.**

Under that rule a version-one subscriber written as

```rust
_ => list.reset(),
```

is never wrong under a later version - it re-reads more than it had to, and
re-reading is always correct. Anything a subscriber *must* act on specifically
gets a new entry point rather than a new variant: coalescing a burst of writes
into one notification, for instance, should land as `.coalesced()` on the
subscription builder, not as a `Batch` variant that a version-one wildcard would
swallow. That is worth writing down now, because the tempting shape is exactly
the one that fails silently.

## API sketch

Reads answer from memory and cannot fail, so none of them returns `Result`.
Writes return `WriteResult<T>`, which is `Result<T, Report<WriteError>>` -
`ReactiveMapError` is already an alias of `WriteError` and the table adds no
error kinds of its own.

```rust
#[amethystate(prefix = "tasks", version = 1)]
pub struct TaskState {
    #[amestate(table)]
    pub items: ReactiveTable<Task>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Task {
    pub title: String,
    pub done: bool,
    pub due_at: i64,
}

impl TableRow for Task { type Id = RowId; }
```

The `#[amestate(table)]` attribute is not decoration. `get_map_types` decides a
field is a map by matching the last path segment of its type against the literal
string `"ReactiveMap"`, so a type alias or a renaming import silently generates
a scalar field - already recorded in `TODO.md`. A table should be declared by
attribute rather than by the spelling of its type, and the attribute costs the
macro one flag.

Writing:

```rust
let tasks = state.items();                            // ReactiveTable<Task, WritableMode>

let id = tasks.insert(&Task { title: "wire the flush".into(), ..default })?;
tasks.insert_with(id.clone(), &task)?;                // caller's id, create or replace
tasks.update(&id, &task)?;                            // strict: an absent id is an error
tasks.modify(&id, |t| t.done = true)?;
let gone: Option<Task> = tasks.remove(&id)?;
tasks.clear()?;

tasks.durable().insert(&task)?;                       // returns once it is on disk
tasks.durable().remove_async(&id).await?;
```

`update` is strict for the reason `ReactiveMap::update` is: subscribers receive
`Update`, which carries the previous value, and an absent row has none.

Reading, and this is the shape a render function has:

```rust
let view = tasks.view()
    .filter(|_id, t| !t.done)
    .sort_by(|a, b| a.due_at.cmp(&b.due_at))
    .build();

let total = view.len();                               // usize
for (i, (id, task)) in view.range(first..last).enumerate() {
    ui.row(first + i, id, task);                      // (R::Id, R), cloned out of the projection
}

if let Some((id, task)) = view.row_at(cursor) { .. }  // Option<(R::Id, R)>
let where_is_it: Option<usize> = view.index_of(&id);
```

`range` takes a `Range<usize>` over the view's order, clamps it to the length,
and clones out only what is asked for. `row_at` returns `Option` rather than
panicking on an out-of-range index, because a virtualised list computes indices
from a scroll offset and a length that a write may have changed between the two.

Reading a table too large to hold, which is a different piece of code in a
different place - a worker loads, the drawing thread reads what came back:

```rust
// worker
let cursor = TableCursor::<LogLine>::open(&store, ["logs", "lines"])?;
let page = cursor.window_after(last_seen.as_ref(), 200)?;   // blocks; returns resident

// and for the work that never had a screen
for row in cursor.rows() {
    let (id, line) = row?;                                   // Result per row
    writeln!(out, "{id}\t{}", line.message)?;
}

// drawing thread, holding `page: RowWindow<LogLine>`
page.len();                                                  // rows in this window
page.rows();                                                 // &[(R::Id, R)]
page.last_id();                                              // where the next window starts
page.total();                                                // Option<usize>, None unless counted
```

A `RowWindow` may hand out all of itself, unlike the table: it is bounded by
construction, so the rule against whole-collection reads does not apply to it.

Subscribing:

```rust
let _sub = view.subscription_with().external().register(move |change| match change {
    ViewChange::Inserted { at, .. }        => list.rows_inserted(*at, 1),
    ViewChange::Removed  { from, .. }      => list.rows_removed(*from, 1),
    ViewChange::Updated  { at, .. }        => list.row_changed(*at),
    ViewChange::Moved    { from, to, .. }  => list.row_moved(*from, *to),
    _                                      => list.reset(),
});
```

`subscription_with()` is the existing `Watch` builder, so `.external()`,
`.local(&mut scope)`, `.every()`, `.stream()` and `.register_with_source()` all
apply unchanged. `.external()` filters out this handle's own changes by
`instance_id`, and `Watchable::filterable` decides which kinds are filterable at
all - for a map it is `Update` only, on the grounds that a key appearing or
disappearing changes what the collection holds and goes to everyone. A view
takes the same rule with `Moved` added to the filterable set, since a row this
handle moved is this handle's own business while a row appearing or leaving the
view is not.

Interceptors, on the table rather than the view:

```rust
let guard = tasks.intercept(|change| match change {
    RowChange::Insert { row, .. } if row.title.is_empty() => None,
    other => Some(other),
});
```

Returning `None` refuses the write and the caller gets `WriteError::Intercepted`
with the refusal attached. This is where a uniqueness constraint lives, if an
application wants one - it can read the table's projection inside the
interceptor and refuse a duplicate - and it is why the library needs no
constraint machinery of its own.

Provenance is `instance_id` and works exactly as it does for a map: `clone()`
keeps the id so writes through the clone are indistinguishable, `fork()` gives a
new one. A view inherits the id of the table it was built from.

`AccessMode` splits the two as it does everywhere else: `insert`, `update`,
`modify`, `remove`, `clear`, `durable` and `intercept` live in
`impl<R> ReactiveTable<R, WritableMode>`, and everything else - including
`view()` - is on `impl<R, M: AccessMode>`. A `TableView<R>` carries no mode,
because a view is read-only by construction.

**A view owns the table it came from.** `TODO.md` records the shape of the bug
that follows from getting this wrong: `.pipe()` on a single source subscribes,
drops the source and keeps only a `keepalive()` that is `None` for `Field` and
`ReactiveMap`, so a component that pipes one field and lets go of the state
struct shows the right first value and never updates again. A view holds an
`Arc<ReactiveTable<R, M>>`, so handing a view to a widget and dropping
everything else is the ordinary case and not a trap.

## Sorting, filtering, paging: where they run and what they cost

All three run in memory, over the table's resident projection, in this process.
The engines do none of it, and the reason is not laziness: the flat engines can
order by the joined key and by nothing else, and the text engines cannot order
at all beyond what the document walk produces. Ordering by a column would
require the column's value to be in the key, which is a secondary index, which
is the section below.

**Building a view** is one pass over the projection to apply the filter, then a
sort. That is O(n log n) comparisons over n row ids, with the rows read from a
`DashMap` by id. At ten thousand rows this is a handful of milliseconds and it
happens when the user clicks a column header, not per frame.

**The comparator is completed with the id.** A caller's comparator is followed by
`cmp_names` over the id as a tie-break, so the order is total. Without that,
equal rows have an arbitrary relative order, incremental insertion has no
well-defined position, and two runs over the same data can disagree about what
the list looks like. It costs one comparison per tie.

**A row change is O(log n) comparisons plus a memmove.** The view's `Vec<R::Id>`
is sorted by the comparator, so the position of an inserted row is a
`partition_point` - about fourteen comparisons at ten thousand rows - and the
insertion itself moves at most the tail of the vector. An update re-evaluates
the filter and the comparator, and produces `Updated` when the position is
unchanged and `Moved` when it is not. A removal is a search and a memmove.

**Paging is free.** `range(a..b)` is a slice of the order vector and a clone per
row asked for. Nothing is decoded - the projection holds decoded rows - and
nothing touches disk. A virtualised list rendering thirty of ten thousand rows
clones thirty rows.

### What the envelope costs

The library is aimed at a million rows as a ceiling, with a hundred thousand
already expected to be uncomfortable. That is the number to design against, so
here is what the resident design costs at each, and whether the ceiling and
residency are in conflict.

One measurement anchors all of it, and it is the only one: `scan_prefix` on redb
takes 8.13 ms for ten thousand entries with the whole map still buffered.
`reactive_map_bench.rs` stops at ten thousand - `SIZES` is `[10, 1_000,
10_000]` - so **everything at 10^5 and 10^6 below is linear extrapolation, not
a measurement**, and the first thing worth doing before committing to any of it
is extending that bench by two decades.

The costs divide into two kinds that behave completely differently, and the
ceiling bites each in its own way.

**Paid once, at open, and movable off the render thread.** One `scan_prefix`,
plus one decode per row. Extrapolating the anchor: about 80 ms at 10^5 and about
800 ms at 10^6, before a single row is decoded. Decode is on top; the library
measures a whole buffered insert at about 5 µs and an overwrite at 2.1 µs, both
of which include far more than a decode, so a bare msgpack decode of a small row
is comfortably under a microsecond - putting the decode term somewhere around
0.1 s at 10^6 and making the scan the larger half.

Call it 100 ms at 10^5 and one to two seconds at 10^6. **80 ms is five frames**,
which is the arithmetic behind the instinct that 10^5 is the edge: it is not
where memory runs out, it is where the open stops fitting in a frame and has to
leave the thread that draws.

**Paid per frame, and not movable anywhere.** Nothing. `range(a..b)` clones the
rows asked for and no others, `len` is a counter read, `row_at` and `index_of`
are a slice index and a search. None of them scales with the table. **Once open,
a resident table is size-independent to render**, at 10^4 and at 10^6 alike, and
that is the property the whole design exists to have.

**Paid per write, and partly per frame when writes arrive during one.** The
order vector's memmove. A sorted `Vec<R::Id>` of sixteen-byte ids moves on
average half its length per insertion at a random position: 0.8 MB at 10^5,
which is on the order of 80 µs, and 8 MB at 10^6, which is on the order of
0.8 ms. One insert at 10^6 is therefore a twentieth of a frame, and a burst of a
hundred is a visible stall. **This, not memory and not rendering, is what makes
10^6 uncomfortable in steady state - and it only bites under write load.**

**Steady-state memory.** The decoded rows, a `DashMap` entry each, and a
`Vec<R::Id>` per view. A small row with two short strings, keyed by a 36-character
id, runs a few hundred bytes all in once the map's per-entry overhead and load
factor are counted - call it 250 to 400 bytes. So 10^5 is 25 to 40 MB and 10^6
is 250 to 400 MB. Large for a desktop application and not disqualifying, and
worth stating plainly because the intuition that residency fails on memory is
the wrong worry: **memory is not the binding constraint at the stated ceiling.**

### The verdict, and the work it names

**10^6 is reachable resident for reading, and not reachable for opening or for
write bursts.** Both gaps are inside the resident design, and both close with
named work rather than with a second type:

- **Take the open off the constructor.** It is the largest cost by an order of
  magnitude and the only one the user experiences as the application failing to
  start. The table opens with an empty projection, reports that it is filling,
  scans on a worker, and emits `Reset` when it is done - which is a state a
  virtualised list already knows how to draw, and which needs no API that is not
  already in this document. This is the single highest-value change here and it
  costs no new type.
- **Scan keys at open, decode rows on demand.** `scan_keys` is measured at 5.09
  ms against `scan_prefix`'s 8.13 ms at ten thousand, and it avoids the decode
  entirely, so the open becomes proportional to the key count with no row memory
  until a row is touched. It is not a free win: a view with a comparator or a
  filter needs every row decoded and so pays it all back on first build, and a
  change carrying `old_value` needs the previous row. It helps exactly the case
  that reaches 10^6 - a long log ordered by id and paged - and does nothing for
  a sorted, filtered one.
- **A better order index than a flat `Vec`.** A B-tree or gap-buffered order
  turns the insertion from bytes moved into O(log n), which is what removes the
  write-burst term at 10^6.

So the resident form reaches the ceiling for everything except the open and
write bursts, and both of those are fixable inside it. What the non-resident
side is for is therefore **not** making the resident table bigger. It is the
work that was never on the drawing thread in the first place, and that is a
different design with a different shape.

**A reactive, offset-addressed, index-backed virtualised list is not proposed
and is not planned.** It needs a secondary index before it can order by anything
but the row id, a maintained total before it can address a scroll offset at all,
and it puts fallible reads on the one path that must not have them - to reach a
size outside the stated envelope. The entry condition that would reopen it: a
real application needing an arbitrary column order over a table it cannot hold.

**Two API rules hold regardless, and neither depends on any of the above.** No
read on the table hands back every row - there is no `rows() -> Vec<R>` and no
whole-table iterator, because that is the shape `ReactiveMap::entries` already
regrets, cloning and sorting the collection on every call. And `row_at` returns
`Option`, because an index computed from a scroll offset races a write that
changed the length. Neither applies to a `RowWindow`, which is bounded by
construction and may hand out all of itself.

**The order vector has the same question one level up, and the same answer.** A
view holds one id per row that passes its filter, so at 10^6 it is 16 MB per
view and the insertion cost above. That is the third work item, not a reason for
a different kind of view.

### Two costs that scale with writes rather than with the table

`scan_prefix` folds the write buffer over the committed result, so a scan during
a large burst of buffered writes pays for the buffer rather than for the answer.
And inserting a key that is not already buffered costs about 5 µs against 2.1 µs
for overwriting one, the difference being a read transaction opened to fill
`old` in the `StoreEvent`; bulk-loading a table is a per-row transaction until
the batch write `TODO.md` proposes exists. Loading 10^6 rows at 5 µs each is
five seconds of writes, which makes the batch write a prerequisite for reaching
the ceiling by any route, resident or not.

## The non-resident side is a cursor, not a window over an index

The scenarios decide the shape, so they come first. What actually reads a large
table without a GUI in front of it:

| | wants | retains |
| --- | --- | --- |
| export, import | one pass in any order | an output handle |
| a bulk migration | one pass, rewriting as it goes | one row |
| aggregation - a count, a sum, a group | one pass | an accumulator |
| background synchronisation | one pass, comparing and writing | one row |
| an integrity check or repair | one pass, tolerating rows that will not decode | a list of the bad ones |
| a headless tool, no GUI at all | one pass | whatever it prints |

Every one is a single forward pass with nothing kept, no random access, no
order beyond the store's own, and no need for a length up front. **None of them
wants `row_at(index)`.** A window over an order vector serves random access into
a sorted view and serves all six of these badly.

Two of them are in this repository already, both doing it by materialising
everything. `MigrationContext::scan_map` reads a whole map into a `HashMap`
before a step touches anything, in one transaction, and its own doc explains that
the strictness is deliberate because the step rewrites the collection whole.
`InspectorBackend::scan_all` is `scan_prefix(&StorePath::root())` - every key and
every value in the store, copied into a `Vec`, so that the CLI can list them. So
the non-resident case is not hypothetical; it is unserved, and the two places
that need it have each built the worst version.

### The cursor

```rust
/// A table read without holding it. One pass, in key order, off the thread
/// that draws.
pub struct TableCursor<R: TableRow> { .. }

impl<R: TableRow> TableCursor<R> {
    pub fn open(store: &Store, path: impl IntoStorePath) -> StorageResult<Self>;

    /// Every row from the start, or from just after `id`.
    pub fn rows(&self) -> impl Iterator<Item = StorageResult<(R::Id, R)>> + '_;
    pub fn rows_after(&self, id: &R::Id) -> impl Iterator<Item = StorageResult<(R::Id, R)>> + '_;

    /// One pass, counting. There is no cheaper way - see below.
    pub fn count(&self) -> StorageResult<usize>;
}
```

`open` reads nothing and costs nothing; there is no projection to build. The
iterator yields in key order, in chunks, and holds one chunk at a time.

**Fallibility per row rather than per pass is the point, not a concession.** A
`Result` on every item is unremarkable in a stream - nobody writes `.unwrap()`
per frame on one - and it is what finally serves the repair case: a row that
will not decode is one `Err` among many `Ok`s, the pass continues, and the tool
reports which rows are bad. That is exactly what the resident table cannot do,
because its open is all-or-nothing, and it is why the strict open above can stay
strict: the loose reader exists, it is a different type, and it is the one a
repair tool reaches for.

**It is not reactive, and that is a decision.** A cursor has no subscription, no
interceptor chain and no `instance_id`. Delivering a change for a row the cursor
does not hold would mean reading it in order to deliver it, which is the cost the
cursor exists to avoid. A pass sees what was on disk as it went past.

**Chunked, not one long transaction.** The obvious redb implementation is one
`begin_read` held for the whole pass, and it is wrong here. `recovery.rs` exists
because a second live `Database` handle keeps the file locked and defeats the
reopen that heals a failed disk - the background flush used to hold its own
clone, and
`the_database_can_be_traded_for_a_fresh_one_under_a_live_store` was written to
fail the moment another one reappears. A cursor holding a read transaction open
across a long pass is that second handle. So a chunk is one bounded
`scan_range`, the transaction closes between chunks, and a pass tolerates the
store changing underneath it - which it must anyway, since it is not a snapshot.

### Which engines, and the one that is honestly excluded

**The non-resident form is redb and sqlite only, by design.** A text document is
one file parsed whole into memory before anything is addressed; it is resident
by construction, and nobody puts a hundred thousand rows in a json file. A
"streaming" cursor over json, toml or ron would walk a structure that is already
entirely in memory and would be a promise the engine does not keep - the shape
`TODO.md` catalogues over and over, where one API means two different things
depending on which engine is underneath.

So the honest arrangement is that correctness is universal and cost is declared:

```rust
trait StoreBackend {
    /// Whether this engine can read a bounded range without holding the whole
    /// store in memory. `false` means `scan_range` is correct and costs what
    /// `scan_prefix` costs.
    fn supports_ranged_scan(&self) -> bool { false }
}
```

redb and sqlite override it to `true` and implement `scan_range` properly; the
text engines override neither and inherit the default body built on
`scan_prefix`. A `TableCursor` therefore opens and works on all five, and a
caller who cares asks. Nothing in the design breaks on an engine that cannot do
it - which matters because `Store` is one type over every engine and
`Store::from_arc` is public, so out-of-tree backends exist by design and get the
correct-but-slow answer without doing anything.

**What a table on a text engine is, then.** Resident, reactive, and sized like
the file it lives in: a table a person could open in an editor. It has no
windowed sibling because it does not need one, and its cursor is offered for
uniformity of code rather than for size. That is a scoping decision, stated
here so it does not read as a gap.

## A window comes back resident

The bridge between the two sides, and the thing that stops them being two
worlds: **a window is asked for off the drawing thread and comes back as
something the drawing thread can read with no `Result` anywhere.** Scrolling is
loading another window, not reaching for the disk inside a render function.

```rust
// on a worker
let cursor = TableCursor::<LogLine>::open(&store, ["logs", "lines"])?;
let page: RowWindow<LogLine> = cursor.window_after(last_seen.as_ref(), 200)?;

// on the drawing thread
for (id, line) in page.rows() { ui.row(id, line); }   // infallible
page.len();                                            // rows in this window
```

**Windows are keyset-paged, not offset-paged.** `window_after(id, n)` resumes
from a key; there is no `window_at(offset, n)`. Offset addressing needs a count
of the rows skipped, which is a pass, and it is unstable under a concurrent
insert before the offset. A key is stable: a row inserted anywhere else does not
move the window, because the window's identity is a range and not a position.
This is also the second, independent reason the offset-addressed virtualised
list is not proposed - it wants the one addressing mode the store cannot give
cheaply.

### A window is a live view over a key range

`RowWindow<R>` holds decoded rows and the key range it covers, and it stays
subscribed to that range. That is what makes it a resident *representation*
rather than a photograph, and it is well defined for one reason: **a window is a
range, so membership is a comparison.** A row whose key falls inside the range
belongs in the window and a row outside does not, and the store subscription can
decide which without reading anything.

That settles the four questions:

- **A row outside the window changes.** The window is not told, and correctly
  does not care - the row is not in its range, so its contents are unaffected.
  Nothing about the window goes stale, because a window never claimed to know
  about rows outside itself. The one thing that does go stale is a total, and a
  window does not have one unless it was given one.
- **A row inside the window is deleted by someone else.** An ordinary
  `RowChange::Remove`, with provenance saying it was not this handle, and `len`
  drops by one. Identical to the map's behaviour, and truthful: the view holds
  one fewer row because there is one fewer row in its range.
- **Writing through a window.** Not allowed; `RowWindow` has no write methods.
  There is exactly one write path - a writable handle on the table - and it
  carries the provenance, the interceptor chain and the ownership guard. A
  second write path with its own rules is the shape of the `Kv::guard` hole that
  `TODO.md` records, where an alternate route reached paths the guard was
  supposed to protect. The id is in the window, so writing what you found is
  `table.update(&id, &row)` with the id the window gave you.
- **Two overlapping windows.** Supported, and they cost two windows: each is an
  independent range with its own rows and its own subscription, and the overlap
  is duplicated in memory - nothing at page size. There is deliberately no
  shared row cache between them, because a shared cache needs invalidation
  between windows and that is precisely the coherence problem that defining a
  window as a range avoids.

One consequence worth stating rather than discovering: a window subscribed to a
range **grows if rows are written into that range**. It holds what its range
holds, which is the same contract `ReactiveMap` has over its prefix, and it is
bounded by what the application actually writes there.

### The total, and what it costs

`len` answers for the view and always did. A scrollbar and a "showing 1-50 of N"
need something else - how many rows exist altogether - and that is an affordance
the windowed form adds rather than a defect it works around.

There is no cheap way to get it. sqlite answers `SELECT COUNT(*)` over the key
range from its `TEXT PRIMARY KEY` index; redb has to iterate the range, which is
the pass the cursor exists to avoid. So `cursor.count()` is honest and expensive,
and a `count_prefix` on `StoreBackend` would be a method one engine can answer
cheaply and the other cannot - the kind of asymmetry that becomes a divergence.

The alternative is a counter maintained in the metadata space, keyed by the
table's path, under the rule already established there: reconstructible from the
rows alone, so a lost or diverged metadata file costs one counting pass at the
next open rather than a wrong number. That is the shape to build if a total is
wanted, and it is not built here - `RowWindow::total()` returns
`Option<usize>`, `None` unless a count was asked for and paid for, so the
expensive thing is visible at the call site instead of hiding behind a cheap
name.

## Secondary indexes

**Not proposed, and after the sections above there is no occupant left for one.**

An index exists to avoid reading rows you do not need. Take the three forms in
turn. A resident table has already read every row before it answers anything, so
an index over it saves nothing - the order vector a view maintains *is* the
index, built in memory in O(n log n) and maintained in O(log n) per change, and
writing it to disk would save that build while leaving the open's real cost, the
scan and the per-row decode, exactly where it was. A cursor makes one forward
pass in key order and reads every row on purpose; an index would only tell it
where to start, which is what `rows_after` already does with a key. A window is
a key range, so the store's own ordering resolves it with a bounded range scan
and nothing else is consulted.

The one thing that would need an index is the form that is not proposed: an
arbitrary column order over a table too large to hold, where the order cannot be
computed because the rows are not there to compare. So the index and that form
are one feature, and neither is built.

If it ever is, three things are already decided above and should not be
re-litigated: it lives in the metadata space keyed by the table's path and never
in the data space, because every segment under the table is a legal row id; it
is reconstructible from the rows alone, so a lost or diverged metadata file
costs a rebuild rather than wrong answers; and it needs `scan_range`, because an
index whose consumer still scans the whole prefix has not removed the cost it
exists to remove.

## What has to be added to `StoreBackend`

**For version one, nothing.** The table uses `scan_prefix` at open,
`set_owned_erased` and `delete_with_source` for writes,
`delete_prefix_with_source` for `clear`, `subscribe`/`unsubscribe` for the
projection's upkeep, `flush_prefix` and `flush_async` for `durable()`, and
`is_initialized`/`set_initialized` for the seeding decision. Every one of those
is on the trait today, and a table is layered on them exactly the way
`reactive_map_with_path_only` is.

For the cursor and the window, one method and one flag:

```rust
/// The keys and values under `prefix` from `after` onwards, at most `limit` of
/// them, in the order `scan_prefix` lists in.
fn scan_range(
    &self,
    prefix: &StorePath,
    after: Option<&StorePath>,
    limit: usize,
) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
    // default: scan_prefix, skip past `after`, take `limit`
}

/// Whether `scan_range` reads a bounded range, or materialises the prefix and
/// then narrows it.
fn supports_ranged_scan(&self) -> bool { false }
```

redb ranges from the start key to `utils::subtree_bound(prefix)` and takes
`limit`, which is what `range` already does with the take missing; sqlite has
`utils::key_range` and a `TEXT PRIMARY KEY`, so it is
`WHERE key >= ? AND key < ? ORDER BY key LIMIT ?`. Both answer `true`. The text
engines take the default of both: the document is parsed whole into memory
before anything is addressed, so a bounded range costs what an unbounded one
costs, and saying `false` is the truth rather than an apology.

Three things about that addition matter more than its body.

**Both get a default implementation.** `Store::from_arc` is public and
documented as the way in for a backend implemented outside the crate, so
`StoreBackend` has implementors this repository cannot see. A method with a
default body is additive; a required method is a breaking change to a public
trait. The defaults are correct and merely slow, and `supports_ranged_scan`
defaulting to `false` is the safe answer for a backend nobody here has read.

**It has three consumers, two of which exist today.** `MigrationContext::scan_map`
builds a whole `HashMap` before a step runs, and `InspectorBackend::scan_all` is
`scan_prefix(&StorePath::root())` copied into a `Vec` so the CLI can list a
store. Both become chunked walks on top of the same method. That is a better
justification for adding it than the table is.

**It does not remove the write buffer's cost.** `scan_prefix` folds the pending
writes over the committed result, and a chunk still has to be folded against
every pending key under the prefix. So `scan_range` bounds what is read from
disk and does not bound what is merged, and a pass during a large write burst
still pays for the burst. Fixing that is a separate change to how the buffer is
indexed and should not be smuggled in here.

`count_prefix` is the candidate to leave out. sqlite answers it from its index
and redb has to iterate the range, so it would be a method one engine answers
cheaply and the other answers by doing the thing the caller was avoiding - the
asymmetry that becomes a recorded divergence. A total belongs in the metadata
space as a maintained, reconstructible counter, or it belongs to whoever paid
`cursor.count()` for it.

## Open questions, most expensive to defer first

**1. Whether redb keeps positional struct encoding.** Answering this late means
answering it with tables on disk, and the answer either way then costs a rewrite
of every row in every store. Answering it now costs one run of drift on the
author's machine, which `TODO.md` has already accepted. Everything about how a
table's columns are allowed to change follows from it. This is the one item that
should be settled before a line of the table is written.

**2. The row id format.** `Uuid::now_v7()` hyphenated is the recommendation and
the reasoning is above. It is expensive to defer because ids are keys: a store
that already holds rows under one format and starts generating another has a
default order that is time-ordered on each side of a boundary and arbitrary
across it, with nothing to indicate where the boundary is.

**3. Whether `ViewChange`'s index conventions are the ones a list widget wants.**
Every subscriber is written against them and they cannot move afterwards. The
convention proposed - `at` after the insertion, `from` before the removal,
`Moved.to` after accounting for the removal - is what `QAbstractItemModel`,
`NSTableView` and the diffable list adapters converge on, but it is worth
checking against the adapters this repository already ships (`amethystate-gpui`,
`amethystate-dioxus`, `amethystate-leptos`, `amethystate-yew`,
`amethystate-reactor`) before it is fixed, because those are the first
consumers.

**4. Whether the extrapolation from 10^4 holds.** Every number at 10^5 and 10^6
in this document is `scan_prefix`'s measured 8.13 ms at ten thousand, multiplied.
`reactive_map_bench.rs` stops there, and a range scan over a B-tree is not
exactly linear in the count. Extending `SIZES` by two decades is a morning's
work and it is what turns the sizing section from an argument into a fact -
including whether the open at 10^6 is one second or four, which is the
difference between "move it off the render thread" and "do not do this".

**5. Whether the resident open can be moved off the constructor without an API
that admits it.** A table that is filling has a state the caller can observe,
and the only two honest shapes are a constructor that blocks and one that hands
back something empty and fills it. `field_with_path` and
`reactive_map_with_path_only` both construct synchronously today, and the
generated struct's constructor does too, so this decides how the whole state
struct is built and not just the table. Expensive to defer for that reason,
though not as expensive as 1 to 3.

**6. Whether the sidecar's placement in the metadata space is safe on the text
engines.** The rule that everything there is reconstructible is what makes it
safe, and it holds for a total and for an index. Worth confirming before the
space is declared reserved, because un-reserving it later means moving data.

**7. Strict open versus quarantine.** Deferrable, and now cheaply so: strict is
what version one does, the loosening is forward-compatible, and the case that
used to argue for quarantine - reading a damaged table to repair it - is served
by `TableCursor`, which is fallible per row on purpose.

**8. Whether a table field is declared by attribute or by type spelling.**
Cheap either way. Declaring by attribute fixes the type-alias hole
`get_map_types` has, and switching to it later is a macro change with no effect
on stored data.

**9. Batched notification.** Deferrable, and the compatibility rule above says
how: a subscription option, not a variant. Nothing about version one has to
anticipate it beyond not adding a `Batch` variant.

**10. Whether `TableView` should expose an iterator over the whole view.**
Leaving it out is the cheap answer and it is what `ReactiveMap::entries` argues
for on its own. Additive later.

**11. Whether a view can be built from another view**, and whether a
`RowWindow` should be able to hand its rows to a `TableView` for local sorting
within the page. Both convenient, both purely additive, neither settled here.

Three things this RFC does not know and should not pretend to. Whether the
`Occupied` refusal a document engine raises can be reached by a table write -
`ensure_map` refuses writing under a level that holds a plain value, and a row
written under a table path that some other code left a scalar at would meet it -
has not been traced through. Whether `MigrationContext::scan_map`'s transaction
behaviour on sqlite holds for a hundred thousand rows in one step was not
measured. And whether a `RowWindow`'s range subscription can be expressed
against `SubscriptionKind`, which has `Any`, `ExactPath` and `Prefix` and no
range - the window's prefix is the table, so it would receive every change under
it and filter by comparing keys, which is correct and is more traffic than a
range subscription would be.

## The smallest version worth building first

In order, each standing on its own.

1. **Settle question 1.** Nothing else should start until redb's encoding is
   decided, because the answer changes what "a column was added" costs forever.
2. **Fix migration cleanup for a composite field.** A dropped table field must
   take its rows with it. This is an existing defect, it is already reproduced
   in `tests/migration_cleanup_composite.rs`, and shipping a table before it is
   fixed means shipping a way to leak most of a store.
3. **`ReactiveTable<R, M>` over the map's layout, with no view at all.** Rows,
   ids, `insert`/`insert_with`/`update`/`modify`/`remove`/`clear`, `durable()`,
   `intercept`, `RowChange`, `subscribe_any`, the strict open, and the resident
   projection. This is `ReactiveMap` with a settled id type and a change enum
   that is `#[non_exhaustive]`, and it is worth having on its own.
4. **`TableView<R>` with the default order and no filter or comparator.** The
   order vector, `len`, `row_at`, `range`, `index_of`, `ViewChange` with all five
   variants, and the incremental maintenance. Ordering by id only. This is the
   part a virtualised list actually needs, and doing it before sorting keeps the
   maintenance logic honest - the positions have to be right before a comparator
   makes them interesting.
5. **`filter` and `sort_by` on the view builder**, plus `set_filter` and
   `set_sort` emitting `Reset`.
6. **`scan_range` and `supports_ranged_scan` on `StoreBackend`,** with default
   bodies, implemented properly on redb and sqlite and left at the default on
   the text engines. Then `TableCursor<R>`: `open`, `rows`, `rows_after`,
   `count`. This is worth doing whether or not anything else here is built,
   because `MigrationContext::scan_map` and `InspectorBackend::scan_all` are
   both waiting for it and both materialise whole stores today.
7. **`RowWindow<R>` and `cursor.window_after`.** The resident, range-scoped,
   read-only view that a loaded window comes back as, with its range
   subscription and `total() -> Option<usize>`.

Steps 3 and 4 are the version worth shipping for a GUI. Step 6 is the version
worth shipping for everything that is not a GUI, it is independent of 3 to 5,
and it may well be worth doing first on the strength of its two existing
consumers alone. Step 7 is what joins the two halves, and it should not be built
before there is a screen that needs it.

Everything before 3 is somebody else's bug that a table would otherwise inherit
at scale. Nothing here builds a reactive virtualised list over a table too large
to hold, an index, or a maintained total; those are one feature, and the
condition under which it becomes worth having is written down above rather than
left to be argued again.
