# RFC: the ownership tree, and what replaces the schema hash

**Status: built, less one hole named at the end.** What follows is one subject
reached from two directions that turned out to be the same one - who owns which
place, and how a store notices that the declarations moved.

`TYPE_HASH` was a `u32` computed by XOR over field names and their types'
hashes, compared once at the open. It is gone, and the tree written beside it
does the judging: `moved::between` compares the places a prefix recorded against
the places it declares now, and `SchemaSnapshot` is what it reads.

`MIGRATION_DEPS` and the dependency graph are gone with it. Ordering is the
reaching, as [below](#migrations): `MigrationContext::global_get` and
`global_set` bring the prefix they name up to date before they answer, the stack
of prefixes part-way through is what a cycle runs into, and the chain is named
end to end.

## What the schema is

**The subtree one struct creates when it opens.** Not a document, not a
description of user data, not one tree for the whole store: the small set of
places a declaration claims, recorded per prefix - which is what
`SchemaSnapshot` already is.

`StoredShape { role, optional, children, flattened }` is that tree, and it is
already persisted. What it does not carry is a comparable description of what a
leaf holds, and it will not gain one:

**The contents of a leaf are not described.** A leaf's type is the user's, its
`Serialize` is the user's, and the store hands the value to the codec whole.
Describing it would need a probe of the type, a vocabulary to compare in, an
answer for `with` (where a pair of functions decides the stored form and the
type would lie about it), and a rule for recursion. All of that buys an earlier,
coarser signal for something the reader already catches precisely and by path: a
value that will not decode, answered by `on_unreadable` and listed by
`disagreements()`.

So the schema describes **places**, and the reader answers for **values**.

## What breaks

**What moves the ownership tree.** Everything else is data.

| what changed | breaking |
| --- | --- |
| an entry appears in a map | no - the map already owned that space |
| a field is declared where nothing was | yes - free space changed hands |
| a field is no longer declared | yes - its data is out from under any declaration |
| a field's `path`, `rename_all` or `flatten` changed | yes - the data is where the old build left it |
| a field's role changed - leaf, map, node | yes |
| a leaf's type changed | not visible here; the reader catches it |

The line is not "additions are safe". A rename decomposes into a removal and an
addition and is indistinguishable from either, so the signal is **a removal**:
nothing removed means nothing could have moved out from under anything.

And an addition is not the harmless half it looks. Declaring a path annexes the
subtree beneath it - a leaf takes `ui.theme.*`, a map takes `ui.open.*` entirely
- so a place that was open to an extension yesterday is closed today. That is
why entries in a map are free and declarations are not: **one uses ownership
that already existed, the other changes who has it.**

## What the recorded tree is for

**It tells. It never forbids.**

A claim protects one live writer from another. A claim recorded last run, whose
owner this build does not declare, protects nobody: nothing is writing there. It
is memory, not authority.

That distinction is what makes the design need no discipline from anybody. The
alternative - honouring a recorded claim - cannot work, because a build cannot
tell a struct that was deleted from a struct behind a feature flag that is off
here. Both are absent from the inventory in exactly the same way. A rule that
needs that distinction would need somebody to remember, and remembering is not a
mechanism.

So:

- **blocking** is `Places`, built at runtime from `take()` as fields are
  constructed, exactly as it is today. Overlapping prefixes are already refused
  there and nothing needs adding;
- **the recorded tree** says what used to be here and who had it. A declaration
  over a place nobody live holds simply works, and says what it found.

What is given up is protection across builds, and it was never there: a tree
cannot stop a build that is not running. The build that comes back finds a value
it cannot read, which is `on_unreadable`'s business and already handled.

## Where it is written and where it is read

**Written per struct, from what actually opened.** Not from the inventory: a
struct compiled into the binary and never constructed has claimed nothing, and
recording it as an owner would be a lie.

**Merged, never rebuilt.** A run that does not open something must not erase it
- not opening and not existing look the same, and a feature-gated build would
otherwise wipe the places of every build but its own. Entries are dropped only
when something says so.

The tree therefore does not shrink by itself. That is the price of the return
case working: a struct written for a feature, removed, and wanted again two
versions later finds its own places still recorded as its own, rather than
being told there is something foreign in the way.

**Read whole only by the tool.** An application compares its own subtree against
its own record, at the moment it opens, so the complaint lands on the struct that
has the problem. `get_schema_snapshots` is what unions them, and the question it
answers - *what does this store hold that nothing declares any more* - is a CLI
question. A CLI is a different binary with no inventory of the application's
types, so the recorded form has to stand alone: it cannot ask a type anything.
That, and not drift, is the reason it must be readable on its own.

## Identity

**The path is the identity. The struct's name is a label.**

A name is a `&'static str` from a type that may not exist in this build, two
different types in two builds may share one, and one type may be renamed. So the
name is recorded, shown in reports, and never compared.

This is the second instance of the same lesson: `StoredFieldEntry::type_name`
already carries "drift is judged by the hashes and nothing compares this". It is
worth writing down as a rule rather than discovering a third time.

## Migrations

Two things were confused during the design and are separated here.

**Migrations are about data on disk, not about handles.** They run eagerly, all
of them, from the inventory, at the open - which is what the inventory is for:
the steps are free functions with no state, and collecting them is the whole
job. Deferring one until something opens a handle would leave a prefix nobody
visited on an old version, for another build, a tool or a backup to find.

**The stack orders that pass; it does not defer it.** `MIGRATION_DEPS` and its
graph go: a static list on the current type encodes a relationship that belonged
to a step two versions ago and no longer applies. Instead, when a step reaches
outside its own prefix - `global_get` and `global_set` are the only way to, and
are already named for it - the prefix it reached into is migrated first,
recursively. The stack is what detects a cycle, and it names the whole chain.

**The run already rolls back.** `StorageProvider::atomic` wraps the whole pass,
and every engine implements it: redb and sqlite open a write transaction, which
is dropped uncommitted when a step returns `Err`; the text engines clone both
documents before the run and put them back. So a cycle found mid-run costs
nothing - there is nothing written to undo - and the dry pass that would
otherwise be needed on the text engines is not.

That does not fix the torn write, and is not meant to. `RFC-text-atomicity.md`
has seven ways to lose committed data on json, toml and ron, each pinned by a
failing test. Transactionality within the run stops a migration from being an
*additional* source of half-states; the single write underneath stays exactly as
good as it is.

## What this removed

- `TYPE_HASH`, `AmeType`, `hash.rs`, `fnv1a`, and `schema_hash` from the
  comparison;
- `MIGRATION_DEPS`, `depends_on`, `depends_on_raw`, the `petgraph` dependency
  and everything that read it - components and their topological sort;
- the noise: adding a field with a default is the commonest and most harmless
  change there is, and it tripped the hash.

And one limitation went with them. A recursive type did not compile, because
`TYPE_HASH` was a const that read its fields' hashes and rustc refused the
cycle:

```
error[E0391]: cycle detected when simplifying constant ...::TYPE_HASH
```

Nothing const-evaluates over the type graph any more, so `struct Branch {
children: Vec<Branch> }` is storable for free, and
`a_recursive_value_is_one_key_however_deep_it_goes` holds one on all five
engines. The limitation was never in the documentation - nobody had written it
down as a rule, and it was found by a test written for something else.

`#[derive(AmeType)]` goes with the trait, and it was required on every plain
struct used as a leaf or a map's value. Nothing replaces it: such a struct now
needs only what serde needs. The tauri-only `SchemaExportEntry` that derive also
submitted goes with it, which costs the TS export for plain nested structs -
`tauri-wasm` does not compile today for unrelated reasons, and a derive that
does only that can come back when it does.

## Considered and not taken

**JSON Schema, via `schemars` or otherwise.** Easy to produce and hard to
compare: two schemas can be equivalent and textually different - `type: [x,
null]` against an `anyOf`, a `$ref` against an inline, key order, `format`
present or absent - so a differ would have to be right about all of it or report
false drift, which is worse than a hash. It also describes JSON, while redb
holds msgpack and toml holds TOML values.

**Probing the type for a serde-model shape.** Better vocabulary - it is what
every engine actually writes through - and derive-free, since `DeserializeOwned`
is already required. Dropped with the leaf description itself: once the schema
describes places rather than values, there is nothing left for it to describe.

**`#[releases(v1::DebugPanel)]` and `#[amethystate(takes_over)]`.** Both are
signatures, and both need somebody to remember to sign. Once a recorded claim
cannot forbid anything, neither is load-bearing: taking a place nobody live
holds is the default behaviour and says so in the report.

**Lazy migration, driven by the first touch of a prefix.** See above: it makes
the store's integrity depend on where the user clicked.

**One ownership tree for the whole store.** The unit is the subtree one struct
creates when it opens, which is what is stored per prefix already. The whole is
assembled by reading, when a tool asks; it is not written that way.

## What is left open

**Does removing a field delete its data?** No, and the report says so. Rust's
own answer is the precedent: `mem::forget` is safe, not undefined - a leak is
wasteful and not wrong. Orphaned data is a lint, not a refusal.

**How stale is stale?** The tree can record when a place was last claimed, and
then a tool can say *nothing has opened this for eleven runs* rather than
guessing at *nobody declares this*. Cheap, and it is what makes an operator's
decision to drop an entry an informed one.

**What a step does about a place it annexes.** `MigrationContext` already has
`get_raw`, `set` and `delete`, so adopting, converting or discarding what was
there needs no new mechanism. Whether the absence of such a step over occupied
foreign ground should be reported is the one thing here still to decide.

## A namespace chosen when it runs

A struct with no prefix is `SchemaEntry { prefix: None }`, and `ensure_snapshots`
skips those, because there is no path to record one at. When such a struct is a
`#[amestate(nested)]` part that is right and enough: its places are in its
holder's snapshot already, as `StoredShape::children`.

Built on its own it is a different thing. `Struct::new(store, "instances.a")`
puts its places on disk at a path nobody knew until it ran, so the recording
happens where the construction does - `StoreBackend::record_schema`, beside
`set_initialized`, which is the other write into the metadata that a
construction rather than a migration makes.

In `new_with_id` and not in `new_with_id_under`, because the latter is also how
a holder builds a part, and a flattened part is built at the holder's own path:
recording there would file the part's places under the holder's name.

The engines compare before they write, so building the same struct again costs
one read.

## Identity

**A declaration is the places it owns.** Not its name, and not the prefix it
sits under. A prefix is not a place - nothing takes one, `Places` keys on the
paths a declaration owns and holds the name beside them as a label - so a prefix
holds however many declarations sit there, each recorded whole, and two of them
are the same declaration when they own a place in common.

That is decidable rather than guessed at. Declarations at one prefix own
disjoint places or they are refused, so a place belongs to at most one on either
side and a recorded tree meets at most one declared tree.

Sharing no place is not a puzzle either, and needs no tie-break: a declaration
whose every place moved is a removal beside an addition, which is what the two
are - the data under the old places is out from under any declaration, and the
new places annexed whatever was under them. The same rule as a rename, one level
up.

**The version is part of it.** A declaration carries its own, in its own tree,
rather than the prefix carrying one for whatever happens to sit there.

**The name takes no part.** Not in what is compared, not in what triggers a
write. A type renamed while its places stay put has changed nothing about the
store, and `shape_on_disk::a_rename_is_not_a_change_to_the_store` holds it.

## The hole

**The ladder is still the prefix's.** `PrefixMeta` holds one version per prefix
and `MigrationPlan` is keyed by prefix, so the steps under a prefix are one
ladder even where two declarations sit there with versions of their own. The
record above is per declaration; the walk that reads it is not, and moving it
touches `MigrationSet`, `PrefixMeta`, `AppliedStep` and the migration log.
