# TODO

Known and deliberate, kept here so it is not rediscovered.

Sizing note, because it has skewed judgement before: this is a store for
persistent reactive state, not a settings file. Settings are its smallest case -
tens of keys, written by hand. A cache persisted between runs is just as much
the target, and that means thousands of keys, written in bursts, read in scans.
Costs dismissed as trivial at ten keys are not trivial at ten thousand, and the
entries below are sized for the larger case.

## Nothing on the TypeScript side is checked by anything

For the pass that takes the integrations together. `Integrations/typescript.md`
is corrected and every block on it now compiles against the local `js/src`,
which is how the rest of this was found - the page was the symptom.

**The example cannot catch drift, and the pin is the smaller half of why.**
`examples/tauri-settings/package.json` asks for `amethystate` at `0.3.1` from
npm, so `js/` in this tree is never exercised. That is fixable with a `file:`
dependency and it buys nothing on its own, because
`examples/tauri-settings/src/bindings/amethystate.ts` opens with
`// @ts-nocheck` - and that is the only file importing the package. Rename a
class and the imports resolve to `any`, every field of `AppSettings` becomes
`any`, and `tsc --noEmit` still exits zero. It does today: the file imports
`ReadonlyReactiveField` and never uses it, which `noUnusedLocals` would
otherwise reject.

**No CI reaches the examples at all.** `.github/workflows/ci.yml` runs fmt,
clippy, tests and `cargo doc`. Nothing there mentions `examples/` or npm.

**`ReadonlyReactiveField` cannot be produced by codegen.** `FieldKind` in
`crates/core/amethystate-core/src/scheme.rs` has `Plain`, `Nested`, `Volatile`
and `ReactiveMap`, and the TypeScript emitter maps `Plain` and `Volatile` alike
to `ReactiveField`. The class is exported, imported into every generated file,
and reachable only by writing bindings by hand. Either something emits it or
the import stops being generated.

**`js/package.json` points at `tam1sh/amethystate` and `js/README.md` links
`uniproc-dev.github.io`,** while the landing pages use `uniproc-dev`
throughout.

What would close it: the example building against the local package, the
`@ts-nocheck` gone, a CI step that typechecks the example, and some check that
ties the page to `js/src` the way `cargo xtask book` now ties the Rust pages to
the crates.

## Every per-engine suite asks one engine unless told otherwise

Three files are built as one statement asked of every engine, and `cargo test`
puts the question to redb alone in all three - the other four sit behind their
own features, so each needs its own invocation.

| file | how it fans out | what an ordinary run sees |
| --- | --- | --- |
| `backend_conformance.rs` | `engine!` per backend | redb |
| `depth_is_refused_before_it_is_written.rs` | `per_engine!` per backend | redb |
| `non_finite_float.rs` | a `cfg` ladder picking one | redb |

A file whose stated purpose is that "a statement that fails on one engine and
passes on another is the finding" is, by default, asking one engine. The depth
file is the sharpest: the ceilings are 512, 254, 127, 80 and 64, and the whole
point is that each engine answers with its own - a default run sees one of the
five.

Three things have already been found only by running a feature by hand: the toml
branch-read bug, which `an_ancestor_is_not_a_value` asserted correctly and was
never asked; a `kv` snapshot left stale under toml through a rename every other
engine's snapshot had taken; and the confy tests, which turned out to pass on
toml long before anyone looked.

CI does run the matrix, so this is not invisible forever - it is invisible for
as long as it takes a change to reach CI, which is exactly the window in which
it is cheap to fix.

Whatever closes this has to survive being forgotten: one command that runs all
five, and something that fails when an engine is added without joining it.

## The inspector finds no schema at all on a text engine

`get_schema_snapshots` for the text engines returns an empty list every time, so
`amethystate-cli inspect` shows no declared structs for a json, toml or ron
store. The snapshot is written; nothing reads it back.

The encoding is applied twice. Writing puts `meta_key("schema", prefix)` - an
already joined and escaped two-segment path - into the flat metadata document as
**one name**:

```rust
let key = store::meta_key("schema", prefix);
let parts = [key.as_str()];
```

Reading calls `scan(&[])`, and `generic_scan` composes each child through
`prefix.try_push(&k)` and hands back `full.as_str()` - joining a second time, so
the separator inside that one name is escaped: `schema\.ondisk`. The reader then
looks for a literal `"schema."` prefix, which cannot match.

**The repair is not a `strip_prefix` that unescapes.** That was tried and it is
a workaround over untyped access: it undoes the second encoding and still
compares strings, so a prefix that itself holds a separator - `app.panel` -
stays a guess. What the reader wants is the flat document's **raw child names**,
which `Navigable::scan_children` already produces and `generic_scan` is the one
wrapping into a path. So `TextDocument` is missing a way to enumerate records as
names, and the kind is then read as `parse_joined(name).segments()[0]` rather
than matched as text. The inverse of `meta_key` belongs beside it, so one place
knows the layout instead of the writer and the reader each knowing half.

Found by `the_recorded_shape_survives_this_engines_codec`, which is why the
inspector-shaped tests in `shape_on_disk.rs` are redb-only: that file did not
compile without redb at all, and a per-engine test had to stop going through the
inspector to run.

## A value coming in from the disk passes through nothing

Interception is one-way. `run_interceptors` is reached from `field_ops`,
`map_ops` and `Field::set` - every one a write. The path that brings a value the
other way, `set_forwarded` from the store subscription, asks nobody.

There are three doors inward and none of them consults anything the application
declared:

| door | what it does with a bad value |
| --- | --- |
| construction | `store.get::<T>()?.unwrap_or_else(\|\| default.clone())` |
| the subscription - an external edit, or another handle's write | decodes it and puts it in the signal |
| a migration | hands it to the step as raw bytes |

So the only thing a read reacts to is **bytes that will not decode**. That case
is handled: `unreadable` is set and the field answers with its declared default.
A value that decodes perfectly and is nonsense - a window at -32000, a font size
of zero, the name of a theme that does not exist - goes into the signal and out
to every subscriber. A write interceptor cannot help: the value is already on
the disk.

**The declared check is built**, at construction and at the subscription, for a
field and for a struct - *Built: what the check receives, and what it does*
below has the shape it took and *Left on this* has what it did not close. The
reasoning between here and there is what settled it and is left as written.

**A check on the way in cannot have the shape of one on the way out.** An
interceptor is `Fn(Change<T>) -> Option<Change<T>>` and `None` means "refuse
this write". Nothing can refuse what is already stored, so inbound `None` can
only mean "answer with the default instead" - which is exactly what a decode
failure already does. A sanitiser therefore joins a channel that exists: the
field reports the default and says why, through `Field::try_get` and
`unreadable`.

**It has to be on all three doors.** A value sanitised when the file is edited
under a running program but not when the program starts is worse than no
sanitiser, because which one you get depends on whether the store happened to be
open.

**Which rules out a runtime registration, and that is the load-bearing part.**
`intercept` is a method on the handle - `field.intercept(..)` - and the first
door is construction: `field_with_path` reads `store.get::<TValue>(&path)?` and
builds the signal from it *before any handle exists*. Nothing can be registered
on a `Field` in time to see the value that `Field` starts with. So a check on
the way in has to be **declared**, which puts it in the macro's attribute
vocabulary - short again after this release: `default`, `key`, `nested`,
`volatile`, and a fifth for this.

It cannot be a trait on the leaf type either, at least not without asking
something of every leaf: a blanket "no check" impl and a specific one overlap,
and the `Probe` trick answers predicates, not calls.

**On the subscription it runs for what came from outside, and that is
expressible.** The external-change diff emits `source: None` on every event it
raises, so a value from the file watcher or another process is distinguishable
from one this process just wrote and already put through the write interceptor.
Running both on the same value invites the two to disagree.

**For a map it is the same question as the silent skip.** `map_entries` already
drops an entry whose key will not parse as `K`, without a word - see *A key that
will not parse disappears from a scan*. An entry that parses into nonsense wants
the same policy as one that will not parse at all, so these should be decided
together rather than one being silent and the other absent.

### Decided: strict by default, loosened only where it is written down

**Three scopes, and they do not overlap.** They answer different failures and
each has its own addressee, so no "the more specific wins" rule is needed:

| what failed | who is told | policy lives on |
| --- | --- | --- |
| the file - will not parse, will not open, is not this format | whoever called `build` | `StoreBuilder::on_unreadable` |
| a declared path inside a prefix | whoever called `new_with` | `#[amethystate(on_unreadable = ..)]` |
| one value | nobody: it becomes the default and appears in the report | `#[amestate(check = ..)]` |

Quarantining belongs at the top and nowhere else: a file cannot be set aside for
one prefix and kept for another, and a document that will not parse at all is a
failure no per-prefix policy lives long enough to see. A store that quarantined
and started fresh has to say so **to the application**, not only to the log -
someone will have to explain to a person where their settings went.

**Built, and the middle row was the loosening rather than the tightening.**
`#[amethystate(on_unreadable = "use_default")]` is in, with `refuse` the default.

The row was written believing an unreadable field quietly took its default and
construction succeeded. It never did: `field_with_path` reads
`store.get::<TValue>(&path)?`, so a value that will not decode has always
propagated and `new_with` has always returned `Err`. Strict was never the thing
to add. What was missing is the way out - a settings struct that has to open
even though somebody hand-edited one value into nonsense, which is the support
ticket this section opens with.

`use_default` takes the field's declared default, leaves the stored value on
disk for a person to fix, and sets the field's `unreadable` marker at
construction, so `try_get` answers `Err` from the moment it is built until a
change decodes. Nothing new was needed for the reporting: that channel already
existed for the live path. `tests/struct_read_policy.rs` holds it.

`on_delete` is declared the same two ways, with `Keep` the default: a removed
key leaves the field reporting what it last held rather than snapping to a
compile-time guess.

**Both are written as variants, not strings**, and the name is taken from the
last path segment, so `UseDefault` and `OnUnreadable::UseDefault` say the same
thing and neither needs the type in scope. A typo is a compile error listing
what is allowed.

**A field may tighten the struct's read policy and never loosen it.** A struct
that says `Refuse` and a field that answers `UseDefault` is a compile error
naming the field; `UseDefault` on the struct with `Refuse` on the one path that
must be readable is the shape that works. The comparison is against what the
struct *wrote*: with no policy on the struct a field may say either, or the
implicit `Refuse` would make a field-level `UseDefault` unreachable. `on_delete`
has no such rule - neither of its answers is stricter than the other.

**A `nested` struct inherits it, because it is addressable like any other
path.** Every generated constructor has a `new_with_id_under` taking the two
policies, and `new_with_id` is that with the defaults. A nested field is built
through it, handed whatever the parent resolved for that field, and what the
nested struct declared for itself wins over what arrives.

**The pair is checked across that boundary too, while it compiles.** The macro
never sees the other type's attributes, so what it emits instead is a
`DeclaredPolicy` impl carrying what each struct wrote as associated consts, and
a `const _: () = assert!(..)` beside every nested field whose holder promised
`Refuse`. A nested struct that declares `UseDefault` under it fails to build,
named, with the span on the holder's attribute. The same `const _` idiom the
type hashes are pinned with.

Still open on this row: a `ReactiveMap` is built through its own factory and
carries neither policy.

### Substituting the default for a value that would not decode is wrong

Both answers above are about opening. A field that has been running and then
meets an undecodable *change* - the file edited from outside, a migration
leaving something behind - is a different moment, and what happens there is not
a policy anyone chose. `primitives_factory` forwards `on_unreadable`, which is
`default.clone()`, so the live value is replaced by a shipped constant.

Three things are wrong with it, and the third is the one that matters.

**It destroys what the person is looking at.** A window dragged to a good size
snaps back to the factory one because another process wrote nonsense into the
file. The default is a compile-time guess and the least likely correct value at
that moment.

**It wakes the subscribers with it.** This is not a quiet fallback: the signal
fires, so the UI actively redraws to the wrong value. Doing nothing at all would
be strictly better than what happens now.

**It collapses a distinction the rest of the library defends.** The deletion
half of this is built: `on_delete` is a policy now, `Keep` is its default, and a
removed key leaves the field reporting what it last held. What remains is the
undecodable *change*, which still forwards the declared default, so a value that
will not decode is still indistinguishable from one that was never there.
Everything else here works hard to keep absent, null and deleted separate - the
whole of `absent_or_null.rs` is about that.

**Keep the last decodable value instead.** It is what is on screen, it is the
last thing the store actually agreed with, and `try_get` already exists to say
the store no longer does. Keeping it makes `try_get` load-bearing rather than
advisory, which is the point of having it.

That gives three answers, and they are not a ladder - they answer different
moments: refuse at the open, default when nothing is known, keep when something
is.

**And it is the default.** A declared path that cannot be read is far more often
a bug or a tampered file than a thing to shrug at, and shrugging is what makes a
stale value indistinguishable from a successful write - the failure the
non-finite float entry above is still about. This is a behaviour change and it
belongs in this release, where the breaking section is already long: one line
now against years of "why did my settings reset".

**Two things strictness must not sweep in.**

*Half of absence is not a failure - the other half is.* A prefix that was never
written is a first launch: seed the defaults and say nothing. A prefix that
*has* been written and is missing one of its declared paths is damage - a key
somebody deleted, an external edit, a migration that did not finish. Refusing
the first would refuse every first launch; shrugging at the second is how a
setting disappears without a word.

Both discriminators are already on the disk, and one of them is this release's
work:

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

**The dial is the same at all three scales, and starts tight.**

```rust
StoreBuilder::new(path).on_unreadable(Default)          // the whole store
#[amethystate(prefix = "ui", on_unreadable = Default)]  // this prefix
#[amestate(default = 14, on_unreadable = Default)]      // this field
```

Loosening is opt-in and visible in the declaration rather than hidden in
behaviour. `unreadable` and `Field::try_get` are not replaced by any of this -
they become how a *deliberately* lenient field says it fell back.

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
| valid document, rubbish after it | refused | `valid_content_followed_by_rubbish_is_refused` |
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
| value is the wrong type | `Undecodable`, refuses | `tamper_shapes` reports; does not refuse |
| value out of range | `Refused`, takes the default, in the report | `field_check`, `struct_check` |
| a leaf became a branch | refused | `a_leaf_that_became_a_branch_is_reported` |
| nested struct's inner field broken | the parent sees it settled | **no** |

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
| an edit makes a field undecodable | falls back, reports; never refuses | keeps the **stale** value - see the float entry |
| an edit deletes a declared key | falls back to the default, reports | `field_delete` |
| an edit adds an unparseable map key | appears in `unreadable_keys()` | **no** |
| a broken edit is not overwritten by us | left alone | `a_broken_external_edit_is_not_silently_overwritten` |

### Built: what the check receives, and what it does

**A check answers with a reason, not a `bool`.** `fn(&T, &CheckContext) ->
Result<(), Invalid>` on a field, `fn(&Self, &CheckContext) -> Result<(),
Invalid>` on a struct. The reason is the point: it is what `try_get` reports and
what a refused open carries, and a `bool` has nowhere to put it. What a `bool`
would have saved - naming the field - the caller already knows, since the
factory holds the path.

**A check is a bare `fn` and captures nothing**, so it can only reach intrinsic
invariants on its own - `min < max`. The interesting ones are extrinsic: does
that monitor still exist, is that theme installed. That is the Readest failure
exactly, and it cannot be answered without the application's world.
`StoreBuilder::context` is that door, and it is a second word beside `provide`
rather than the same one: a migration step runs once, inside `build`, on the
thread that called it, so `provide` can take an `Rc` and does; a check runs
whenever a value arrives, the watcher's thread included, so `context` asks for
`Send + Sync` and the store keeps it. `require` refuses the value when nothing
was given - a check that cannot reach its world cannot say the value is good -
and travels the same channel the verdict does.

**Which doors it stands at:**

| a value arrives | a field's check | a struct's check |
| --- | --- | --- |
| construction | runs | runs |
| the subscription, `is_external_edit` | runs | does not |
| `load_with`, under `mode = "persistent"` | runs | cannot be declared |
| this process's own write | does not | does not |
| a migration step | does not | does not |

The migration door needs nothing of its own: a step produces the value and gets
raw bytes rather than a `T`, and everything it leaves behind is read again by
construction, in the same process, before any handle exists.

The struct's check does not stand at the subscription. Its fields are built
before `Self` exists, so a live one would need the struct to hold a weak
reference to itself in every field's subscription, and would recompute a
cross-field invariant on every inbound change.

**A refusal is the situation `on_unreadable` already describes**, and there is
no seventh knob. `Refuse` fails construction with the reason and the path;
`UseDefault` takes the declared default, leaves the stored value on disk and
sets the field's `unreadable` marker, so `try_get` answers `Err` until something
passes. Live, a refused external edit keeps the last value and wakes nobody,
which is what an undecodable one already does. The failure is a `Refused` fact
on a `StorageError::Read` report rather than a new variant - the facts are
types.

**A struct's verdict is projected onto its fields**, because `Field::unreadable`
is the only channel there is and a relationship has no field of its own.
`Invalid::at` names the fields it is about and only those report it; naming none
means all of them. Under `UseDefault` the values are **kept** rather than reset:
there is a declared default for a value and none for a relationship, and what
the fields hold is still what the file says.

**Order:** every value first, then children, then parents. A nested struct runs
its own checks inside its constructor, so a parent's check sees children already
settled. `tests/field_check.rs` and `tests/struct_check.rs` hold all of this.

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

## Metadata carries no format version - deliberately, for now

`PrefixMeta` and `SchemaSnapshot` both use `version` for the user's schema
version, from `#[amethystate(version = N)]`. Nothing records how `hash` was
computed, which drift rules produced it, or how steps were ordered.

So changing any of those algorithms is a one-way door: new code reads old bytes
and cannot tell they are old. Changing the hash makes every existing store
report total drift on the first run of the new version, because the stored
number was produced by a formula that no longer exists.

**Decided: not building this yet.** Compatibility is an obligation to somebody,
and right now there is nobody but the author. Change the format, eat the drift
once, move on.

Revisit when that stops being true - the first release someone else depends on,
or the first time "delete your store and start over" is not an acceptable
answer.

### Worked through: an open set of facts, not a number

The shape below replaces the store-level *number* first sketched here. The
number stays useful for one thing only - see "the one-time move".

**Not a version, a set of facts about how the bytes were written.** What is
unrecorded today is not one thing that moves together; it is a handful of
independent settings that change on different schedules, and a single number
cannot say which of them moved. Every row here silently decides how already
written bytes read back, and none of it is on disk:

| fact | value today | what a silent change does |
| --- | --- | --- |
| `codec` | msgpack (redb) / sonic-json (sqlite) / the document's own (text) | nothing reads |
| `codec.struct` | `map`, from `.with_struct_map()` | structs read as arrays - the silent corruption the flag exists to prevent |
| `codec.bytes` | `bin`, from `BytesMode::ForceAll` | a `Vec<u8>` written as `bin` will not read as a sequence |
| `path.sep` | `.` | every key renamed |
| `path.escape` | `\` | every key renamed |
| `layout` | `nested` for data, `flat` for the meta sidecar | the fork recorded at the end of this file |
| `init.marker` | `__init::` on flat engines, `__init.` on text | seeding markers lost, so defaults land on top of the user's data |

**Three reader rules, and the class is a property of the name.**

- a fact in a *deciding* namespace (`codec.*`, `path.*`, `layout`) that the
  reader does not know, **or a known name with a value it does not know** -
  refuse to open, naming the fact. git spells the second case out separately
  because it is easy to miss;
- a fact anywhere else - ignore it, and **preserve it on write**;
- a fact that is absent - the value that held before the name existed.

Class by namespace rather than by a per-fact flag is deliberate. git's
`ec91ffca0455` records the cost of the alternative: four extensions were once
honoured at repository version 0, and *"for compatibility reasons, we are stuck
with that decision"* - a class that is declared can be declared wrongly, and
wrongly once is wrongly forever. A class carried by the name cannot be forgotten
and cannot be loosened without renaming.

**Downgrade is the case that makes the split necessary.** Refuse-on-unknown
alone locks every older build out of every store a newer one has touched, and on
a desktop a rollback is an ordinary operation, not an accident. With the split,
an older build refuses only when a *deciding* fact moved - which is exactly when
it should. The policy, stated:

1. no build refuses because a number is higher; no number does that job;
2. the only refusal is an unknown deciding fact, and it says which;
3. unknown facts and unknown keys survive being written by an older build -
   true by construction today, to be pinned by a test;
4. an older build never *removes* what it does not understand (early proto3
   dropped unknown fields and 3.5.0 reversed it - dropping is silent loss
   through any round trip);
5. a downgrade never rewrites data into an older encoding: it reads, or it
   refuses;
6. whatever a build will not do, it names the fact that stopped it.

**The one-time move, and it is being spent by inertia.** The absence of the
whole set has exactly one honest meaning: written before the set existed. That
works once, and only for files written *before* the set ships. Step 7 of the
seven-step list under "Decided: paths carry segments" is "a format version in
the metadata", and it arrives in the same release as the break - which helps no
file written before it. Cargo is the worked example: cargo before 1.47 ignored
the top-level `version` in `Cargo.lock` entirely, so the marker added when it
was needed did not protect the versions it was meant to protect. **The set has
to land before the break, not with it.**

**Where it cannot live.** Not in `metadata` keyed by prefix: that key is
`prefix.as_str()`, and a prefix literally named `__format` is not forbidden - in
a flat key space there are no unforgeable names. It needs a table of its own on
redb and sqlite, and its own key in the text `.meta`. sqlite's `application_id`
and `user_version` are both free (verified) and are the right size for a fast
refusal, but cannot be the source of truth: the other four engines cannot see
them.

**Global or per prefix.** Encoders are a property of the engine, so global. But
`layout` under "structure where a schema declares it" becomes a property of a
*prefix* - flat where no snapshot describes it, nested where one does. So the
set is probably two-storey. Not settled.

**When a fact is written.** It describes bytes that are already there, so it is
written when the format change actually reaches the disk, not when the code
learns to produce it. That is ZFS's `enabled` against `active`, and it rests on
an explicit rule there - *"Features may not perform enable-time initialization"*
- without which "enabled" would not be safe for a reader that does not know the
feature. The same discipline is what makes an open set safe here.

### Also settled, from the same pass

- **Ignore-unknown is almost already true and is free to pin.** Unknown tables
  and keys survive a write: the text engines serialise the whole loaded
  document, redb and sqlite do not touch rows they did not write. It holds by
  construction and nothing states it. One test - old build opens, writes, the
  new key is still there - makes it a contract.
- **Lazy repair cannot carry an address change.** `scan_prefix` in two
  encodings does not reconcile: subtree bounds are cut at `.`, so an old key
  lands in the wrong subtree with no error. Lazy is right for values and for
  bookkeeping records, wrong for keys.
- **ro-compat is not refused, it is expensive.** Two objections were raised: a
  read-only flag does not restrain a person with a text editor (true, and it is
  the text engines' whole purpose), and `Store` has no read-only mode at all -
  writes go through the debouncer, so an honest one needs a new contract on
  `set`/`delete`. The second is a cost rather than an impossibility, and the
  rollback case is where it would pay: an older build that can *read* the
  settings still starts. Worth it only if rollback is expected to be common.
- **The internal pass must run before the user's migration, and the reason is
  worse than ordering.** `ensure_snapshots` rewrites the snapshot with the
  current schema at the end of every `run()`, and the snapshot is the only
  record of roles - without which the text relayout is undecidable. A late
  internal pass finds its own input already overwritten. Not "the run failed",
  but "there is nothing left to retry from".
- **A separate format identity makes the intermediate state legal.** If the
  format's identity is not `PrefixMeta.version`, then "format new, schema
  version old" is a correct state between the two passes, and they need not
  share a transaction - which is what avoids changing `StorageProvider` to hand
  out one transaction for the whole open.
- **Rewriting keys is not idempotent.** Re-encoding an already-encoded key
  compounds the escape. A repeatable pass has to write the new keys without
  deleting the old, and delete the old in a separate step after the facts flip -
  at the cost of peak double size.
- **The bridge already exists in a dependency.** redb 2.6 added file format v3
  as opt-in (`create_with_file_format_v3`, `Database::upgrade()`); 3.0 dropped
  v2. One minor that reads both and writes the new one on request, then a major
  that cuts the old loose. Same shape as ZFS enabled/active: "the code can" and
  "the disk has" are separate events.
- **One compatibility policy is already in force, unchosen.** `PRAGMA
  journal_mode = WAL` raises bytes 18/19 of the SQLite header to 2, so these
  files already will not open in SQLite before 3.7.0.

## `Kv::check_type` compares printed type names, and only within one run

```rust
let wanted = std::any::type_name::<T>();
match resolve_field(path) {
    Some(meta) if meta.value_type_name != wanted => Err(WriteError::TypeMismatch { .. }),
    _ => Ok(()),
}
```

Two problems, and the second is the one that matters.

**`type_name` is not an identity.** The standard library documents it as
diagnostic output with no stability guarantee: the same type can print
differently depending on how it was named at the use site, and different types
can print the same. Today nothing breaks, because both strings come from the
same build and change together - the check works by coincidence, not by
construction.

**The check should survive a restart, and cannot.** A path claimed as one type
in an earlier run is not checked at all now; the guard only sees what this
process built. That is the wrong scope for a store whose whole point is that
data outlives the process.

`TypeId` is not the answer either, for the same reason `type_name` is not: it is
not reproducible across runs, so there is nothing to compare a stored value
against. Nor is it usable where a compile-time constant is needed - `TypeId::of`
is still not `const` on stable as of 1.90.

What is left is the structural fingerprint, `AmeType::TYPE_HASH`: computed at
compile time from field names and primitive type names, so it is deterministic
across builds and survives renaming a module or the type itself. The cost is a
bound - `Kv::cell` currently takes any `Serialize + DeserializeOwned`, and would
need `T: AmeType`.

**Fixing the XOR fold is a prerequisite, not a separate task.** A fingerprint
that cannot see two fields swapping order or exchanging types is not a basis for
deciding whether a path holds the same type as before.

**Done: there is no type check.** `check_type` is gone rather than repaired -
"Decided: the library guarantees paths, and says nothing about types" below is
what it was traded for. What a path holds is the writer's business; what refuses
a write is ownership of the path, and that is spelled out under "Done: ownership
is by declared path".

## Built: an identical write costs a comparison

All five engines compare the serialised bytes against what is stored - buffered
or committed - and an identical write returns before anything else happens:
nothing buffered, no subscriber called, no flush scheduled.

It sits where the old bytes were already being read. `committed_or_buffered`
fetches them to fill `old` in the `StoreEvent`, so the redb and sqlite dedupe is
a memcmp on a read that had already happened; the text engines compare the
incoming node's bytes against the stored node's before touching the document.

Bytes rather than values, for two reasons that both still hold. A `PartialEq`
bound would break every value type and would compare what is in memory rather
than what will be on disk. And it is correct for floats: `NaN != NaN`, so a
`PartialEq` dedupe fails exactly where it looks like it works, while the msgpack
encoding of `NaN` compares equal to itself.

**Rewriting a value is no longer a way to mean anything.** "Checked again, still
valid" is a real pattern for a cache and it wants an explicit operation rather
than riding on whether the bytes happened to differ - `ReactiveCache` separates
the two already, with the stamp in the meta space changing on its own. That
operation is not built.

This does not fix a GUI binding rendering twice. That is one write and one
notification coming back to its own author, which is what `Watch::external`
is for.

## Two type hashes, both weak, and the weaker one feeds the gate

There are two computations and confusing them sends a fix to the wrong file.

`schema_hash` in `migration/types.rs` folds each field with
`h ^= fnv1a(name) ^ type_hash; h = h.wrapping_mul(..)`. The multiply makes it
order-sensitive, so swapping two fields' types *is* caught here.

`gen_recursive_type_hash` in `amethystate-macros/src/hash.rs` is a bare
`0 ^ fnv1a(name) ^ H(ty) ^ ..` with no seed and no mixing. It emits `TYPE_HASH`
for every derived type and every generated `_Data` struct - and it reaches the
migration gate through `FieldDescriptor::type_hash`, which is an input to
`schema_hash`. So the pure XOR is not a side channel; it is laundered into the
decision that runs migrations.

Reproduced in `tests/type_identity.rs`: 22 `const _: () = assert!(..)`, so the
build fails the moment any of them stops holding. Nothing runs at test time
because nothing needs to.

**The generic impls cancel with themselves.** Three lines in `types.rs` settle
it: `Vec<T>` is `fnv1a("Vec") ^ T`, `Option<T>` likewise, `HashMap<K, V>` is
`fnv1a("HashMap") ^ K ^ V`. Therefore

| | equals |
| --- | --- |
| `Option<Option<u32>>` | `u32` |
| `Vec<Vec<u32>>` | `u32` |
| `Vec<Option<u32>>` | `Option<Vec<u32>>` |
| `HashMap<u32, u64>` | `HashMap<u64, u32>` |
| `HashMap<T, T>` for every `T` | `fnv1a("HashMap")` |

The map row reaches the gate: a `ReactiveMap<u32, u64>` field changed to
`ReactiveMap<u64, u32>` leaves `SCHEMA_HASH` identical. Keys are stored as path
text and parsed with `FromStr`, values decoded by the codec, so every entry is
then read with the two decoders exchanged. No step runs and no drift is
reported.

**Zero is both a value and the sentinel for "unknown".** `component_needs_work`
and `migrate_prefix` both guard on `target_hash != 0`, and a schema hashing to
exactly zero is constructible. Such a prefix leaves schema checking for the life
of the application: no drift is ever detected whatever its fields become.
Separately, five unrelated shapes all hash to zero today - an empty struct, a
unit struct, a tuple struct, an enum, and a union, which the derive accepts
rather than refusing.

**A name and a type cancel inside one field.** `fnv1a(name) ^ type_hash` with
nothing between them, so a brute force finds pairs: `{volume_level: f64}` and
`{span_max_len: bool}` - two structs with no field in common - share a
`SCHEMA_HASH`. Likewise adding two fields can be free.

**A nested struct's swap defeats the multiply.** A nested field contributes
`0xDEADBEEF ^ Inner_Data::TYPE_HASH`, and `TYPE_HASH` does not move when the
inner struct's field types are swapped. A nested struct has no prefix of its
own, so the outer hash is the only gate its data has.

**Two different numbers are both called the schema hash and both are written to
the same stored field.** `SchemaEntry::schema_hash` is `_Data::TYPE_HASH`;
`MigrationStepEntry::schema_hash` is `AmeStateFields::SCHEMA_HASH`; they are
never equal. A migrating run writes the second into `SchemaSnapshot`, and
`ensure_snapshots` immediately overwrites it with the first. Whatever ends up
stored is not the number the gate compares against, so the field cannot be
trusted or reused.

**Also missing, found in passing.** No `AmeType` for `char`, `()`, `Box<T>`,
`Arc<T>`, `BTreeMap`, `HashSet`, arrays or tuples - `Box` in particular is how a
recursive type is written and is simply unusable. Generics are unsupported: the
derive emits `impl AmeType for #name` without `split_for_impl()`. A type
recursive through `Vec` fails const evaluation with E0391 and needs a way to
opt a field out of expansion.

Checked and clean: `cfg!(feature = "tauri")` reads the right crate's features,
since the facade forwards the feature to the macro crate.

### The book says this out loud, and will have to be rewritten

`landing/src/content/docs/State/defining-structs.md` has a `#[derive(AmeType)]`
section. It used to claim a *unique* `TYPE_HASH`, which the collisions above
disprove; it now says the hash is a summary rather than an identity, that
distinct shapes can share a number, and that a change landing on the same number
goes unnoticed with no drift reported.

That is the honest description of today and it is not a description anyone wants
to keep. Whatever replaces the hash - a wider one, a structural fingerprint, a
recorded shape rather than a number - changes what that section says and how
much of it is a warning. Rewrite it with the change rather than after it: the
paragraph exists to stop a reader trusting the gate, and it should stop existing
the moment the gate is worth trusting.

The same section is also where `AmeType`'s missing impls will surface. `char`,
`()`, `Box<T>`, `Arc<T>`, `BTreeMap`, `HashSet`, arrays and tuples have none
today, and the page does not say so - a reader meets it as a compile error on a
field they had no reason to think was special.

**Direction.** Widening the hash does not fix any of this - every collision
above is structural, not a birthday collision. Two shapes are worth considering:
fold properly (seed, then per field absorb ordinal, name and type, with mixing
between) and reserve zero; or stop hashing at the gate and compare the stored
schema against the declared one, which `SchemaSnapshot` and `calculate_drift`
already have most of the machinery for. The second inverts the residual failure
from a missed migration to a spurious diff, which is the right direction when a
missed migration silently misreads saved settings and a spurious diff costs one
nag. It is also the option that wants the format version above.

### Decided: the library guarantees paths, and says nothing about types

Confirmed by running rather than by deriving - `Vec<Vec<u32>>` and `u32` print
the same number, so do `Vec<Vec<Vec<u32>>>` and `Vec<u32>`,
`HashMap<String, u32>` and `HashMap<u32, String>`, and two structs whose field
names are swapped between the same two types.

Widening or reseeding is not enough, and neither is replacing the number with a
description. The route was walked and abandoned, which is worth recording so it
is not walked again:

- a *shape* composed bottom-up through the trait fixes the cancellation, and has
  to be a function rather than a constant, because only a function can carry the
  visited set that cuts a recursive type - const evaluation has no memory
  between calls, which is why `#[derive(AmeType)]` on `Tree { children:
  Vec<Tree> }` does not compile today even though the recursion terminates;
- the orphan rule then bites for any type from another crate, answerable with
  autoref fallback to an opaque case, feature-gated impls, or both;
- and *none of it works anyway*, because the stored form of a value is decided
  by arbitrary code - `deserialize_with`, `untagged`, `flatten` - which a dozen
  GUI projects use as a matter of course. A description of the Rust type is not
  a description of what is on disk.

**Drift in the value is inexpressible, not merely unimplemented.** A change that
breaks decoding is caught by the read, which happens at the same moment the gate
would have run - `field_with_path` decodes every declared field on
construction - and reports the path, the type asked for and the codec's own
sentence. A change that preserves the type and alters the meaning, seconds to
milliseconds, is invisible to any type description whatever. That leaves the
gate a narrow band where the read already answers, and the current hash does not
even cover that: it cancels.

So the contract narrows to what is exactly knowable, and the type layer goes:

| goes | stays |
| --- | --- |
| `AmeType`, its derive, its bound on the eight `Kv` sites | `StorePath` and everything checking it |
| `TYPE_HASH`, `SCHEMA_HASH`, `FieldDescriptor::type_hash`, `PrefixMeta::hash` | `MigrationContext` - already path-keyed, unchanged |
| `calculate_drift`, `NaggingRecord`, the `target_hash != 0` gates | step ordering, `Gap`, `Downgrade`, per-prefix isolation, the log |
| the orphan hole, recursion in const, `Shape`, serde tracing, a feature matrix | `AmeStateFields::FIELDS` - as the owned *path* set, not a shape |

`AmeType::TYPE_NAME` is replaced by `std::any::type_name::<T>()`, which fixes a
live bug on the way out: `TYPE_NAME` is `stringify!`, the field registry records
`any::type_name`, and `check_type` compares the two - so asking for one path
twice with the same type fails for anything but a primitive. Verified:
``path `b` is already `alloc::string::String`, asked for `String` ``, and `Vec`
loses its parameter entirely.

### Decided: four layers, and none of them is a Rust type

The dead end was trying to derive the stored shape from the declared type. The
stored shape is on disk, in the format's own fundamental types, and every engine
already knows it - a text node is a `serde_json::Value` or a `toml_edit::Item`,
sqlite has a column type, redb has msgpack's tag. The codec even says it out
loud today: `invalid type: integer 800, expected a string`.

So the record is built from what is there, not from what the code says, and it
falls into four layers with four different sources:

| layer | known by | says |
| --- | --- | --- |
| path | the library, exactly | where a value lives |
| role | the schema, exactly | `field` or `map` |
| shape | the disk, exactly | integer, text, object, array |
| meaning | the author | `version` |

None needs a description of a Rust type, so `AmeType`, `Shape`-as-a-trait, serde
tracing, the orphan rule, third-party impls and recursion in const evaluation
all fall away together. A description read off data is finite by construction -
there is no cycle to break - and `deserialize_with` or `untagged` cannot lie to
it, because whatever they produced is what got written.

## The last write of a store's life can fail without a trace

Every backend family flushes its buffer from `Drop` - [`redb`](crates/main/amethystate/src/store/backend/redb/mod.rs),
[`sqlite`](crates/main/amethystate/src/store/backend/sqlite/mod.rs), [`text`](crates/main/amethystate/src/store/backend/text/store.rs) -
and all three discard the result: `let _ = self.save_now();`.

That flush is the one a short-lived process depends on, and it is the one whose
failure nobody can observe. A locked file, a full disk, a permission error at
exit - the process ends reporting success and the data is not there. `Drop`
cannot return an error, so the value is real, but it can log, and today it does
not even do that.

Two levels worth having:

- log the failure at `error`, so the loss leaves a trace;
- an explicit `close()` that returns the result, for callers that would rather
  find out while they can still do something about it.

Found while chasing a suspected loss that turned out to be the separator bug
above. The flush had in fact succeeded - which was only knowable by adding a
probe to `Drop`.

**Done for the first, by one helper the three share.**
`utils::report_closing_flush` logs at `error` with the file, and each `Drop`
hands it the result it used to throw away. `error` rather than `warn` because
the store is past the point of retrying or telling anyone: the background
ladder keeps `warn` for a flush that is being retried and `error` for one that
gave up, and this is the second kind with nobody left to inform.
`a_closing_flush_that_fails_leaves_a_trace` breaks the disk under a redb store,
drops it, and reads the log back.

The second is already there under another name: `save_now` is on `StoreBackend`
and returns the result, so a caller who wants to know calls it before dropping.
What is uneven is the named form - `close` exists on redb and sqlite and not on
text, and sqlite's takes `&mut self` where redb's takes `&self`.

## Migration cleanup addresses a field by its Rust name, not by where it is stored

`#[amestate(key = "...")]` moves a field somewhere else on disk, and the
cleanup that runs after a migration does not follow. `FieldDescriptor.name`
carries the Rust identifier - `fname_str` in `generate/data.rs` - while the path
is built from `e.key.unwrap_or(fname)` a few lines below. With an override the
two are different strings, and the bookkeeping uses the first.

Reproduced in `tests/keyed_field_rename.rs`, the two failing cases `#[ignore]`d
so the suite stays green. A third case is the control: the same removal without
an override cleans up correctly, so the override is what breaks it.

**Reading the old value works.** The migration function is handed the old struct
through `AmeData`, which respects the override, so a rename carries the value
across exactly as written. What fails is only the removal afterwards.

**The old location is never emptied.** `delete(old_f.name)` in
`migration/context.rs` removes `keyed.left_panel_visible` while the value sits
at `keyed.panels.left.visible`. Deleting an absent key is deliberately not an
error, so nothing is reported.

So a renamed field leaves a copy of itself behind at the old path, and a field
dropped from the schema keeps its value forever - which is the worse of the two,
since dropping a field is how a migration is supposed to get rid of something
that should no longer be stored.

**`schema_hash` has the same blind spot.** It folds `name`, so changing only the
`key` moves the data on disk and leaves the hash identical: no migration runs
and no drift is reported. Not covered by the tests above.

The descriptor should carry the stored name alongside the Rust identifier, and
each user should take the one it means. A stored name is a path, and saying so
in the type is what keeps the two from being confused - so this lands as step 1
of the plan under "The API does not distinguish a path from a name".

## The API does not distinguish a path from a name

A dot inside a string means "next level" in some places and is meant to be an
ordinary character in others, and nothing in the types says which is which.
Where the two meet, the composed string has already lost the boundary.

| takes a string that means | | stated anywhere |
| --- | --- | --- |
| `prefix = "..."` | a path | no |
| `key = "..."` | a path - `tests/migration_complex.rs` relies on it | no |
| `Kv` paths | a path | no |
| a `ReactiveMap` key | a name | no, and it is split anyway |

The first three work as intended; they are an unwritten convention, and under
it there is no way to write a name that simply contains a dot. The fourth is a
bug, because the intent there is the opposite.

`#[rename(old => new)]` is safe by construction - it parses `Ident`s, which
cannot contain a dot.

**What the bug costs.** Reproduced in `tests/map_dotted_keys.rs`, both cases
`#[ignore]`d so the suite stays green. Flat backends store the key whole and are
unaffected, so a single-backend run never sees it:

| | `get` by exact key | `keys` / `entries` / `len` | key that prefixes another |
| --- | --- | --- | --- |
| redb, sqlite | correct | correct | correct |
| json, toml, ron | correct | counts nodes at the level | value destroyed |

Three keys `a.exe`, `a.dll`, `b.exe` give `len() == 2`: `a` and `b` are the
nodes. Now that reads come from the map's projection this is invisible while the
process runs - the projection is keyed by `K`, not by the document tree - and
appears on the next start, when the projection is rebuilt from the prefix. The
reproduction reopens the store for exactly that reason.

Worse, writing `a` and then `a.b` turns the leaf into a branch and the value
under `a` is gone - reading it fails to decode. Both writes returned `Ok`.

**Two schemas can claim the same place on disk.** `key = "panels.left.visible"`
under `prefix = "coll"` and a plain field under `prefix = "coll.panels"` compose
to the same path, and nothing checks for it. Reproduced in
`tests/prefix_overlap.rs`, both cases `#[ignore]`d:

- matching types share the slot silently - a write through one struct lands on
  the other's field, last writer wins;
- disagreeing types surface as `invalid type: boolean, expected u32` while the
  second struct is being constructed, which is a decode failure standing in for
  a name collision.

**A prefix can land on another struct's field.** `prefix = "root"` with a field
`b`, and `prefix = "root.b"`, put a leaf and a branch on the same node. This one
is invisible from the public API: `Field::get()` answers from the signal in
memory, so both structs report their own values for as long as the process
lives. Only the store disagrees:

| | `store.get("root.b")` | `store.get("root.b.x")` |
| --- | --- | --- |
| redb | `Some(10)` | `Some(20)` |
| json | `Err(invalid type: map, expected u32)` | - |
| toml | `Some(20)` - the branch's value | - |

The toml row is the worst of the three: no error, the type matches, the number
is wrong. And the damage only becomes visible on the next start, when the
signals have to come off the disk.

**And a map does not merely share a slot - it reads what is below its entries
as those entries.** `a_map_reads_what_is_stored_below_its_entries_as_those_entries`
in the same file. Write `widths.left.px = 800` and `widths.left.pct = 50`, then
open a `ReactiveMap<String, u32>` at `widths`:

```
map.entries() == [("left", 800)]
```

`name_under_key` returns the *first* level below the prefix, so a key two levels
down is reported under the shallower name with the deeper value's bytes. The
second key is gone - two keys collapsed onto one entry name and the scan's order
picked which survived. No error, right type, wrong number, and one row missing.
`clear()` on that map then deletes both.

This is the worst of the family because a map is the only thing that reads its
whole subtree as a set and writes it whole; a `Field` under another `Field` is
harmless, since nobody scans. So the invariant worth enforcing is not "no
registered path may contain another" - `#[amestate(nested)]` does that on
purpose - but the narrower **a path a map owns may not contain any other
registered path**. `observability::SCHEMA_REGISTRY` is already keyed by
`StorePath` and already sees every declared field; it records and refuses
nothing. `Subtree::contains` in both directions is the whole check.

Forbidding a dot inside `key` is a check the macro can make on its own, ahead of
any of the above, and it makes `key` mean a name rather than a path. It closes
one of the three ways to nest - `prefix` and nested field names remain - but it
is the surprising one, and it is compile-time.

### Isolation: where the design got to, and what is still open

Worked through in conversation, not yet built. Two goals that were being
conflated, and they want different things:

- **collision avoidance** - nobody writes where somebody else writes. A check
  suffices, no layout changes.
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

**What the mechanism reduced to: a claim is made by a constructor.** Not by a
walk over `inventory`, not by a walk over `FieldDescriptor`, not by `Role`.
Every path that gets owned is owned because something was built at it, and the
constructor is where the path is composed:

```rust
// primitives_factory::field_with_path, ::reactive_map_with_path_only
store.owners().claim(&path, resolve_instance(instance_id))?;
```

`by` is the struct's own type name, which `observability::register_field`
already resolves from `instance_id` at the same site.

**Idempotent per `(path, by)`.** The claim belongs to the *name*, not to a live
handle, which is what makes it work: nothing has to be released, reconstructing
a struct in the same process is a no-op, and there is no ABA. Two handles on one
region are legal for one schema and refused between two. A registry of *live*
claims had to choose between "drop releases" (breaks two handles) and "drop does
not" (breaks reconstruction); this has neither problem.

That also settles `Kv` for free rather than by a special case: two `Kv` handles
share `by`, so `Kv` against `Kv` is idempotent and allowed, while `Kv` against a
declared schema differs and is refused. **The table replaces `Kv::guard`** - one
mechanism, not two - and a struct hand-placed at a runtime namespace registers
through the same call, so `SchemaEntry.prefix == None` stops being a case at all.

**Overlap is nesting, and there is no third case.** Subtrees are nested or
apart, never half over each other, so the predicate is two containment tests:

```rust
fn overlaps(a: &StorePath, b: &StorePath) -> bool {
    a.subtree().contains(b.as_str()) || b.subtree().contains(a.as_str())
}
```

`a == b` needs no arm: `contains` is reflexive. `contains` is a level boundary,
not a string prefix - pinned by a table of eleven pairs and three property tests
in `core/src/path.rs`, including the two that bit this code before (`ui` against
`uix`, and `ui` against `ui!x`, where `!` also sorts *between* `ui` and
`ui.theme`).

**The walk is not `windows(2)`,** for that same reason: sorted, the claim that
contains `c` need not be its immediate predecessor. Walk back while the
candidate is a string prefix of `c` - that run is contiguous, and the first
non-prefix ends it - and test `contains` on each. String prefix is the cheap
filter, `contains` is the decision. Forward symmetrically. Inside the walk the
direction is known, so each side needs one containment test, not two.

**No owner tag is carried anywhere.** `(Owner, StorePath)` was the first shape
and it is the same smell this codebase has been bitten by twice - two halves of
one thing that must agree. It is not needed: once overlap is refused, a path
already identifies its owner. `field_with_path`, `FieldInner`, the backend,
subscriptions and `pending` are untouched.

**Detection moves to construction.** `build()` no longer refuses, so an app that
has a silent overlap today fails where it actually conflicts rather than at
boot - which also removes the "this breaks working applications" objection. The
loser is whoever constructs second. That is wrong in one case - `Kv` writing
before its own schema is built - and the fix for it was weighed and dropped:
see below.

**Reading claims off the disk was weighed and dropped.** The idea: schema
snapshots are already written on every open (`MigrationEngine::run` ends in
`ensure_snapshots`, and all three backends run the engine while opening), they
persist, and they carry the field tree with roles - so the declared paths are
knowable before anything is constructed, and even for a schema whose code is no
longer in the build. Two things killed it:

- the snapshot store is keyed by prefix alone, so two schemas at one prefix
  leave one record - **measured**, `tests/snapshot_per_prefix.rs`, `#[ignore]`d.
  Holding several per prefix is possible and claims are what make it addressable
  (paths are stable where a struct name is not, and the refusal makes claim sets
  pairwise disjoint, so a record can be found by path intersection rather than
  by key) - but that is a change to how migration planning finds a stored shape;
- and migrations are the one part of this library that is not worked out. Not to
  be pulled into an unrelated mechanism.

So: **runtime-only, and the meta layer is not touched.**

Recorded as an observation and not as a task, because it is inferred from the
key rather than measured: planning reads one snapshot per prefix, so where two
schemas share a prefix one of them is diffed against the other's record.

**Not part of this, and already done:** a map refusing a key more than one level
below it (`Level::Deeper` in `decode_entry` and `scan_map`). That is the only
mechanism that works against a writer no table knows about - a raw `Store::set`,
a migration, a person with a text editor - so the table does not replace it.
**The table prevents, the read detects.**

### Decided: paths carry segments

Compatibility with existing files is not a constraint - the implementation has
enough bugs that the data written by it is not worth preserving. That removes
the format migration from the work and lets each step land on its own.

Escaping does not disappear, it moves: a flat backend still has to compose one
byte string, and the separator inside it has to be escaped. The difference is
that this becomes a private detail of one engine's key encoding rather than a
rule the API asks callers to observe. Tree backends escape nothing - they walk a
node per segment.

Since the layout breaks anyway, this is the one cheap moment to put a format
version in the metadata. Without it an older file reads as a corrupt one rather
than an old one, and adding it later is a second break.

The steps, each standing on its own:

1. the descriptor carries the stored name next to the Rust identifier - fixes
   the migration cleanup above, touches no layout;
2. a path type carrying segments, with the join done at the boundary with the
   engine; the macro knows its segments at compile time, so the static case is
   `&'static [&'static str]` and allocates nothing;
3. a map key becomes exactly one segment;
4. the macro separates a name from a path - `key` is a name and a dot in it is
   an error, nesting gets its own attribute;
5. `scan_prefix` matches a segment boundary rather than a string prefix;
6. registration refuses two schemas that claim the same path - segments do not
   prevent a collision, they only stop one from happening by accident;
7. a format version in the metadata.

## A key that will not parse disappears from a scan without a word

The text backends rebuild a key from the document tree and read it back with
`StorePath::parse_joined`. Where that fails - a hand-edited file, a key written
before the encoding existed - the code does

    let Ok(child_path) = StorePath::parse_joined(&full_key) else {
        continue;
    };

so the entry is absent from `scan_prefix`, `scan_keys`, and therefore from
`len`, `keys` and the map's projection, with nothing in the log and no error to
the caller. The same shape is in `document.rs`, where a child whose name cannot
be pushed onto the prefix is skipped.

Skipping is the right behaviour; being silent about it is not. This wants the
error carrying enough context to say which key and which file, which is what the
`error-stack` move above is for - so it should be fixed as part of that rather
than by bolting a `warn!` on now and leaving the shape behind.

**Done, both halves.** The two scan walkers now carry the failure up with the
key attached and a line saying the document holds a key this library could not
have written; `generic_scan` logs the child it passed over at `warn`, naming
the prefix and the name, which is what "Decided: a key with no name" below
settles it as.

**`map_entries` still has two of them, and one is a different cause.**
`primitives/map_ops.rs` skips an entry whose path yields no name, and then skips
one where `K::from_str` refuses the name:

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

## Migration cleanup deletes one key, so a composite field survives being dropped

The cleanup emitted by `migrate.rs` and the same loop in
`MigrationContext::nested` call `ctx.delete(field.name)` - a single key. A
`ReactiveMap` field lives at `prefix.field.<key>` and a `nested` field at
`prefix.field.<leaf>`; the branch itself holds nothing, so the delete removes
nothing and every entry stays on disk.

Dropping a `ReactiveMap<String, u32>` field that held `alpha = 7` leaves
`dropmap.cache.alpha` readable afterwards. Same for a dropped `nested` field.

Fails on **redb and sqlite**; the text backends delete a document node and take
the subtree with it, so the two families disagree about what a migration leaves
behind. Reproduced in `tests/migration_cleanup_composite.rs`, with a control
dropping a plain scalar that is cleaned up correctly.

That this is unhandled rather than deliberate is visible in
`tests/migration_reactive_map.rs`, where the migration hand-deletes
`routes.{key}` in a loop to work around it.

Renaming such a field is the same cause with a worse result: the new location is
written while the old subtree stays, leaving two live copies. Distinct from the
`key`-override finding above - this one needs no override at all.

## `Kv::guard` does not cover `as_root` structs

`guard` rejects a path under a declared `prefix`, and an `as_root` struct's
fields sit at bare paths, so nothing matches and `Kv` writes over them with any
type.

`store.kv().set("width", &"oops".to_string())` against an `as_root` struct
owning `width: u32` returns `Ok`, and after a reopen the struct fails to
construct - which is the failure `guard`'s own doc says it exists to prevent.
All five backends. Reproduced in `tests/kv_guard_root.rs`, with a control
showing the identical write against a prefixed struct is refused.

**Done by ownership moving to the declared path.** A root struct's prefix is
the root rather than nothing, so its fields are declared paths like any others
and the walk reaches them without a special case. `` `width` is declared by a
schema `` is what the write gets now, and both tests in `kv_guard_root.rs` run.

## `Kv` refuses the same type it just recorded

`check_type` compares `T::TYPE_NAME` while `register_field` stores
`std::any::type_name::<T>()`. For `u32` the two strings agree; for `String` they
are `"String"` and `"alloc::string::String"`, and for a derived type the bare
identifier against the fully qualified path. So asking twice for the same path
and the same type fails:

    let a = kv.cell("theme", "dark".to_string())?;   // records the long form
    let b = kv.cell("theme", "dark".to_string())?;   // Err(TypeMismatch)

Introduced by moving `check_type` off `std::any::type_name`, which is unstable
across compilers, onto the name a type declares. The move is right; what is
missing is the other half - the registry has to record the same string, or the
comparison has to be over `TYPE_HASH` rather than either name. The entry above
about the two hashes covers what that costs.

**Done: the check is gone.** Neither half was worth having - see the note under
"`Kv::check_type` compares printed type names". `register_field` still records
`std::any::type_name`, now as `value_type_name` and for display only.

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

The same list answers a second question already on this list: what a store still
held when it died. `The last write of a store's life can fail without a trace`
is the same gap seen from the other end.

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

## `fork` is documented as a moment, and it is not one

`reactive/field.rs:100` says provenance travels with the id, that "a fork is how
you deliberately look like someone else", and that `Clone` keeps the id so a
clone stays the same actor. Two things are wrong with that.

**It puts the event in the wrong place.** Nothing happens at `fork()` beyond
minting a `Uuid`. The id is stamped on *every write* as its source and compared
*at every delivery* by `Watch::external`. Neither the original nor the fork
changes at the moment of forking - the doc reads as though the distinction is
created there, and a reader looking for where the filtering lives will not find
it in `fork_with_id`.

**It is asymmetric about a symmetric thing.** "Look like someone else" makes the
fork sound like a disguise the original wears. A subscription on the fork
equally does not hear the fork's own writes and does hear the original's.
Neither is the real one.

What is actually wanted, in the owner's words, is that **writes are ignored by
the handle that made them** - self-echo suppression. That is `external()`, and
it works. `fork` is the odd half: it exists so a handle's writes *do* come back
to another handle watching the same path, which is a real need when two widgets
sit on one field, but "make a second identity" is a roundabout way to spell it.

To rework, not to patch: whether the API should say the thing directly rather
than through minting ids, and whether the pair `clone`/`fork` is the right shape
for it at all. The documentation should not be corrected in place before that is
decided, or it will be written twice.

## The error model's seams with the outside world

Three, none about the contexts themselves - those are right.

`Report<C>` does not implement `std::error::Error`, so `?` from a
`StorageResult` into an `anyhow::Result` does not compile. `Box<dyn Error>`
works; `anyhow` is what an application's `main`, its Tauri commands and its task
bodies are actually written in, and every call site there becomes
`.map_err(|e| anyhow!("{e:?}"))`, which throws away the tree the whole
conversion was for. `Report::into_error()` is the sanctioned exit and nothing
points at it.

`error_stack` is in every public signature and is not re-exported from the
facade, though `serde`, `uuid`, `inventory` and `serde_json` all are. A caller
who wants `.attach()` must add the dependency themselves and keep the version in
lock-step or the traits do not apply.

There is no `From<Report<StorageError>>` for `Report<WriteError>`, so the store
layer and the reactive layer do not compose with a bare `?`. `WriteError` is
local, so the impl is allowed.

## `AmeType` locks every foreign type out, and the user cannot let it back in

`Kv::get`/`set`/`cell`/`map` and every persistent leaf field require
`T: AmeType`. Impls exist for the numeric primitives, `bool`, `String`, `Vec`,
`Option`, `HashMap`. `IpAddr`, `Duration`, `PathBuf`, `SystemTime`, `BTreeMap`,
`HashSet`, arrays and tuples are therefore unstorable - and the user cannot fix
it, because both trait and type are foreign and the orphan rule forbids the
impl. This is not a coverage gap that more impls close; it is a hole with no
user-side patch, and it needs an escape hatch before the bound spreads further.
`Kv::get` takes the bound and never uses it.

A type the user writes is not locked out - `#[derive(AmeType)]` covers it. The
hole is a type from another crate, where neither the trait nor the type is
theirs. The way out is written up under the schema hash below: make the trait
optional, with the shape falling back to the type's written name when no impl
exists.

## Smaller, and cheap

- ~~`ReadOnlyReactiveMap` and `WritableReactiveMap` alias `Field`, not
  `ReactiveMap`, and take one type parameter where a map needs two.~~ **Done.**
  They alias `ReactiveMap<K, V, _>` now. A copy of the field aliases whose
  right-hand side was never changed, public, and used by nothing - the workspace
  had no callers, which is how it survived. The field aliases are live, so the
  pair earns its place once it means what it says.
- `reactive_map_with_path<TScope, ..>` binds `TScope: StateScope` and never uses
  it; callers turbofish four parameters for nothing.
- `Kv::keys` returns absolute paths, where `ReactiveMap::keys` returns
  `Vec<K>`. It should return the names below the namespace. (It returns
  `Vec<StorePath>` rather than `Vec<String>` now, which is the type being
  honest, not the answer being right.)
- ~~`StorePath::from_static` is public and unchecked, with a doc saying
  `joined` must match `segments` and nothing enforcing it. It exists for the
  macro; `#[doc(hidden)]` it.~~ **Stale, and so is the advice.** `check_static`
  enforces both at const-eval time, so a `const` whose halves disagree does not
  compile. And hiding it would now be wrong: its doc contemplates a
  hand-written `StateScope`, and an author writing one needs to be able to find
  the constructor.
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
  the misclassification say so itself. See the entry below on asking the
  compiler instead of guessing.
- Every prefixed struct gets a generated `new()` that calls `global_store()`,
  so the most obviously named constructor is the one that panics when there is
  no global store. There is no `try_init_global`.
- `ReactiveCell::update`/`modify` return `SourceGone` for an absent map key,
  whose message sends the reader looking for a lifetime bug they do not have.
  `KeyNotFound` is in the same enum.
- The README's headline example does not compile: `amethystate::Result` does not
  exist.

## The async twin of a map clear tells a subscriber something else

`map_ops_async`'s `clear` iterates entries and deletes them one by one where the
sync one calls `delete_prefix`, because **`AmeBackendAsync` has no
`delete_prefix` and no `scan_keys`** and `AmeBackendSync` has both. So an async
`clear()` delivers N `Delete` events where a sync one delivers one
`DeletePrefix`, and a subscriber cannot tell "the map was cleared" from "every
entry happened to be removed". The two traits are meant to be twins.

## One decode written twice

`primitives_factory::decode_entry` and `context.rs::scan_map` are the same
function twice. They differ in what decodes - `Store` against
`MigrationBackendAdapter` - so the extraction wants a closure, and the two
halves are ~12 lines each.

## Errors that reach nobody

From an audit of every bare `?` and every silent skip in `core/` and
`amethystate/src`. Ordered by what it costs.

**A failed migration is invisible through `StoreBuilder::build`.** The engine
turns a failure into data - `ComponentOutcome::Failed { error }` inside an
`Ok(report)` - and `build` (`store/builder.rs:262`) discards the report.
`build_with_migration` calls `log_to_tracing`; `build` does not, and
`MigrationReport` is not `#[must_use]`. Confirmed by running it: a store at v1
with a v2 step that returns `Err` opens successfully, silently, holding
pre-migration data, and the application then runs new code against old data.
That is the thing migrations exist to prevent. Every doctest takes this path.

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

**`restore_from_backup` discards its errors while `open` claims the restore
happened.** `text/store.rs:95-104` is four `let _ =` over `fs::copy` and
`fs::remove_file`; `open` then attaches "the files were restored from their
backups". If the copy failed, that attachment is a claim the discarded error
would have refuted, and a reader who believes it will not check the file.

**`entry_cell` turns a read failure into "the key is empty"**
(`reactive/entry_cell.rs:61`), which is the vocabulary the cell reserves for a
removed key. The real defect is the signature: `entry_cell` returns
`ReactiveCell<V>` with nowhere to put an error.

**Poisoned-lock fallbacks that silently disable a subsystem.**
`map_core.rs:289,298,310` fail open in `notify` while the same file uses
`.lock().unwrap()` in seven other places - so a poisoned mutex makes
`subscribe_any` panic while `notify` quietly delivers to nobody, permanently.
`observability/mod.rs:77,87` does the same to the registry that `Kv::check_type`
consults, turning off the guard against one path being claimed as two types.

**`Kv::keys` breaks the `Kv` error type** (`store/kv.rs:204`): it returns
`StorageResult` where every other method returns `WriteResult`, so a caller
using `get` and `keys` in one function needs two error types.

## The background flush can fail silently, and a waiter on it can hang

Found while converting the engines to `error-stack`, in redb and sqlite alike.

The debouncer callback is `FnMut()` with nowhere to return to, so it discards
every error: redb's closure is an `Option`-returning block full of `.ok()?`
(`backend/redb/mod.rs:235-254`), sqlite's uses `Err(_)` and `.is_err()`
(`backend/sqlite/mod.rs:587-644`). A full disk, a missing table and the test's
`SIMULATE_WRITE_FAILURE` all collapse into one bare `false`, and nothing is
logged even though `tracing` is already in scope in both files. This is the
background write path, so a user's data fails to land with no trace anywhere.

Worse in sqlite: if `conn.transaction()` or any of the three `prepare` calls
fails, the closure returns **without** calling `commits_save.finished(..)`. A
`Commit` riding on that flush is never woken. That is a hang, not a lost error.

redb's synchronous `flush_prefix` has the matching hole: `commits.finished(true)`
is only on the success path (`backend/redb/mod.rs:134`), so every `?` above it
returns without telling the waiters anything.

`StorageError::CommitFailed` is the context these want, and `CommitSignal`
already carries a failure flag - what is missing is calling it on the way out.

**Done, on all five engines.** The background debouncer retries a failed
flush at a fixed interval instead of swallowing the first failure, and keeps
retrying until it lands or the store is dropped. `retry_budget` does not bound
that - a full disk is usually somebody about to delete something, and a store
that stopped trying could not heal when they did. It bounds the *silence*: a
streak outliving it escalates once, waking any `Commit` waiter with a failure
and asking `on_persist_failure` what writers should be told from there.

That answer is [`AfterGivingUp`]: `Fail` (the default with no callback) marks
`PersistHealth`, so every later write returns `StorageError::CommitFailed`
naming the reason until a flush lands and clears it; `Ignore` says nothing and
keeps buffering; `Poison` is the old behaviour, now opt-in. Poisoning the
writer for a disk that is briefly full is the reaction least worth having by
default - the application is running, its reads are fine, and the thing it
most needs is to be told, not killed. All three configurable per store
(`StoreBuilder::retry_interval`, `::retry_budget`, `::on_persist_failure`).

The `changes.is_empty()` early return that skipped notifying entirely - so
`flush_async()` on an idle store hung forever, no failure required - is gone
too, folded into the same mechanism as a trivial success. `apply_pending`
factors the table-writing loop out of both the sync and background paths,
which is also what gives the background one a real error to log instead of
`.ok()?`.

[`AfterGivingUp`]: crates/main/amethystate/src/store/config.rs

**What "retry" cannot mean on redb, found while building this.** A real I/O
error - not the test's `SIMULATE_WRITE_FAILURE`, which returns before ever
reaching `Database` - sets an `AtomicBool` in redb's own `CachedFile`
(`cached_file.rs`, `io_failed`) that nothing in the crate ever clears. Every
`begin_write` *and* `begin_read` after that checks it first and returns
`StorageError::PreviousIo` without touching disk - confirmed against redb
4.1.0's own source, including its own test at `db.rs:1395-1410` doing exactly
this. So a retry loop that just calls `begin_write` again is not retrying the
failing operation; on the one failure mode this was built for, it is spinning
at `retry_interval` until the budget runs out, on a `Database` handle that
already decided it is dead - and taking every *read* down with it, not only
writes. The doc's own wording says how to recover: close and reopen the
`Database`. Doing that live would mean every holder of `db: Arc<Database>` in
`RedbStoreInner` - not only the flush path - going through something
swappable (`ArcSwap` is already a workspace dependency) that notices
`PreviousIo`/`DatabaseClosed` and reopens rather than a bare `Arc`.

**Done: redb trades the handle in.** `Fail` and `Ignore` both promise that a
flush landing later heals the store; on redb that was a promise the engine
could not keep, since the retry could never land. It now reopens instead.

`db` is an `ArcSwapOption<Database>` rather than an `Arc<Database>`, and the
`None` is the point: redb holds the file lock for as long as a `Database` is
alive, so reopening is not "make the new one and swap it in" - the old has to
be dropped before `Database::create` can take the lock back. The caller holds
`write_lock` across the gap.

Which also settles what a durable write does, and it needed no separate code:
`flush_prefix` takes `write_lock` first and the reopen holds the same lock, so
a commit runs before or after the swap and never during. A durable write waits,
which is what it promises anyway; a read or a scan takes no such lock, sees the
`None` and is told, rather than blocking a UI thread on a file operation. Keep
the two on one lock and that stays true for free.

Both flush paths reopen on `PreviousIo` - the background one so the retry loop
lands on the next attempt, the synchronous one so a durable write recovers
instead of reporting something the caller can do nothing about.

The one thing that had to be true is that nobody else holds a `Database`, or
the lock never comes back. One did: the background flush held its own clone,
which would have kept the file locked for the life of the thread. It holds the
swap now. `the_database_can_be_traded_for_a_fresh_one_under_a_live_store`
exists to fail the moment a second handle reappears anywhere.

A real `PreviousIo` end to end is covered too, and it was worth the trouble.
`a_disk_that_fails_for_real_is_recovered_by_trading_the_handle` opens the store
on a `StorageBackend` that fails its writes - redb's own seam, reached through
`create_with_backend`, so the latch that follows is redb's rather than a
simulation of it - takes the disk away, gives it back, and asserts the
buffered write lands. It failed on its first run, because `is_previous_io`
answered `false` to a genuine `PreviousIo`: the predicate matched on this
crate's `RedbStoreError`, and the errors that actually carry the latch are
redb's own. `begin_write` fails with a `TransactionError` and `commit` with a
`CommitError`, and `.doing()` is a `change_context` that leaves them in the
report unwrapped. So the reopen would never have fired on the one failure it
was built for, and every test that passed until then had reached the latch by
constructing it rather than by breaking a disk.

The whole of it lives in `backend\redb\recovery.rs` - the swappable handle, the
predicate, the trade, and the tests that break a disk to reach it.

The failing disk is armed by path rather than by a flag, and that is not
tidiness. A global switch is consulted by `create_database`, so while one test
held it on, any store opening in parallel got a broken disk - which is exactly
what `test_drop_behavior_is_deterministic` did, being one of the tests here
without `#[serial]`. It arrives as a failure in a test that has nothing to do
with any of this, whose own code never mentions a disk. Naming the one path
that may break means a test that did not ask for one cannot be handed it, and
the guard puts the disk away even when an assertion panics.

**sqlite and the text engines do not share redb's problem.** Neither rusqlite
nor SQLite itself has anything resembling `io_failed`: a failed write rolls
back its own transaction and leaves the `Connection` usable for the next one,
which is the whole premise `busy_timeout`-style retrying on SQLite already
relies on. The text engines write a whole file with `persist_atomic` and have
no live handle to poison at all. So the same mechanism - retry, budget,
poison, notify - is wired into all five engines now, and only on redb is the
retry itself unable to do what its name says; sqlite and the text engines get
a real second chance, not just a wait.

**Done, the rest of it.** `apply_pending` (redb, sqlite) factors the
table-writing loop out of both the sync and background call sites within each
engine - not across engines, which the architecture pass below this entry
found not worth it. `utils::init_key` replaces four hand-written
`format!("__init::{namespace}")`s with one. `RetryPolicy`
(`StoreConfig::retry_policy`, `StoreBuilder::retry_interval`/`::retry_budget`)
and `on_persist_failure` are configurable per store, defaulting to a 5 second
interval and a 60 second total budget - sized against this project's own
stated write profile (thousands of buffered keys in a burst, not a handful of
settings) rather than guessed, and closer to what a survey of comparable
systems found than the redb `busy_timeout` convention would suggest on its
own: nothing surveyed actually retries silently for a bounded time and then
deliberately crashes - the real spectrum runs from failing fast with no retry
at all (redb's own stance, and Core Data's explicit advice against retrying a
failed save) to a bounded *count* of attempts degrading to read-only rather
than crashing (RocksDB, VS Code) to crashing on the very first failure with no
retry (PostgreSQL's fsync `PANIC`, adopted because retrying itself was unsafe -
Linux clears the dirty-page error flag after reporting it once, so a retry can
silently succeed over data that never actually landed). What landed is closest to the middle
of that spectrum - keep trying, degrade rather than die, and let the
application escalate if it wants to - with the crash kept available and
nobody's default. Three tests in `redb/mod.rs` pin it: writes fail rather than
the process, a disk that comes back heals the store with nothing restarted,
and `Poison` still takes the writer down for an application that asks.

## The sqlite migration adapter still scans by `GLOB`

`backend/sqlite/migration.rs:128` builds its prefix scan as
`WHERE key GLOB ?` with `format!("{}*", prefix)`. `utils::key_range` exists
precisely so this is not done: a name may hold GLOB metacharacters - `panel[0]`
is a name - and nothing escapes them. `ui*` also matches `uix.width`, with no
separator boundary. The main engine's path was fixed; this one was missed.

## The text engines replace two files with no barrier between them, and eat their own backup

Both read from the code, both verified. `RFC-text-atomicity.md` is the campaign
that went looking for what they cost: seven ways to lose committed data, each
with a test that fails on the current code, and three of them sharpen this entry
rather than repeat it - the barrier is missing without any crash at all, an
ordinary I/O failure on the bookkeeping file is enough, and the error return
then says nothing landed when half of it did.

**`StoreFiles::persist` is two atomic replaces, not one operation.**

```rust
pub fn persist(&self) -> StorageResult<()> {
    self.data.persist()?;   // rename #1
    self.meta.persist()?;   // rename #2
}
```

Each half is `persist_atomic` - temp file in the same directory, `sync_all`,
rename - so neither file is ever torn. But nothing joins them, and the data file
goes first. A process killed between the two renames leaves new data beside old
bookkeeping: the snapshot, the migration log and the init markers all describe a
document that is no longer there. `PrefixMeta` then reads as a version the data
has already moved past, so the next open either replays migrations over migrated
data or refuses as a downgrade.

Harmless while the two describe the same shape. It stops being harmless the
moment the on-disk *format* is what changed, because then the header says one
layout and the file holds another - which is exactly the state a format version
exists to make impossible.

**`create_backup` overwrites the backup on every open.**

```rust
pub fn create_backup(&self) -> StorageResult<()> {
    if self.path.exists() {
        std::fs::copy(&self.path, &self.backup_path)
```

`create_backups()` runs unconditionally at open (`text/store.rs`). So if the
previous run died after `persist`, its `.bak` is still on disk holding the last
good document - and the very next open copies the half-written file over it. The
evidence is destroyed by the one action taken because something might have gone
wrong.

The doc comment on `backup_of` reads as though this were considered - it says
the naming scheme avoids "a `store.bak` a person put there themselves". That is
about the *rejected* alternative (`with_extension`, which collides `store.db`
and `store.meta` onto one name); the copy itself is unconditional.

The fix for both is one change: write `.meta` first carrying a marker that the
rewrite is in flight, then the data, then `.meta` again to clear it. And treat
an existing `.bak` as an unfinished previous run rather than as something to
overwrite. That also makes a half-written store *detectable*, which it is not
today.

## Two flushes racing leave the older document on disk

Found by `tests/atomicity_stress.rs::writers_racing_each_other_all_land`, which
fails about twice in ten runs. Four threads write their own paths on one store,
flushing when they feel like it; every writer joins, a final `save_now` returns
success, the store is reopened - and one path comes back holding a value from
earlier in the run. Not the default, which is what a write that never happened
would look like: an earlier state of the same document.

`StoreFile::persist` (`backend/text/store.rs:120`) is the whole of it:

```rust
let content = self.doc.read().serialize()?;
persist_atomic(&self.path, &content, self.write_policy)?;
```

The read guard is a temporary and dies at the end of the first statement. The
replacement then runs holding nothing, so two flushes - the debouncer's thread
and a `save_now` from anywhere - interleave as: A serialises, B serialises, B
replaces, A replaces. The file ends up with what A saw.

Each replacement is still atomic. What is missing is that the flushes are not
ordered with respect to each other, so atomicity per write buys nothing once
there are two writers. `save_now` returning `Ok` means this thread's replacement
landed, not that it is the one still there.

The fix is a lock held across serialise-and-replace rather than across the read
alone. Per file rather than per store, since the two files are already written
one after the other - which is its own gap: a crash between them leaves the data
and the schema bookkeeping describing different stores, and nothing puts those
two replacements in one transaction.

## Decided: the library refuses, the application sanitises

A value the format cannot hold is refused, and nothing in the library ever
coerces one quietly. What `NaN` should become - zero, the last good value, a
refused form - is known only to the application, and it can already say so:
`ReactiveField::intercept` and `ReactiveMap::intercept`/`intercept_key` take a
change and return it rewritten or refuse it, before anything lands or fires.
That is a sanitising layer at `set` and it exists today.

```
set(v)
  |- the application's interceptors    <- coercion lives here, and only here
  |- the codec's gate                  <- refusal lives here
  |- the value lands
```

The gate cannot be an interceptor: that list is the application's and is
ordered, so something installed after it could put back what it rejected. It is
the last thing before the document.

This is not a design to invent. **toml already has it**: `u64::MAX`, `u128`,
`i128`, `()`, unit structs, `Some(None)` and a `Vec<Option<T>>` holding a `None`
are all refused at the write with a report naming the path. The work is giving
the other four the same treatment against their own limits, with toml as the
worked example.

Two places, because they need different information:

- `serialize_node` holds the value and the format, and nothing else. It is where
  json writes `null` for a `NaN` today, and refusing there closes five of the
  json findings in one edit.
- `set_node` holds the path. Depth needs both halves, so it cannot live above.

What the gate is actually for, after the probes: non-finite floats on json and
sqlite, `Option<Option<T>>` flattening on json, sqlite **and** redb, and depth.
Two things that looked like gate material are not - ron's lost variants and
redb's positional structs are both defects, and leave for the list above.

The round-trip verification flag shrinks accordingly. Once the gate covers those
three, what is left for it is asymmetries nobody has enumerated. Keep it as an
option, off by default, and build it last - it was justified when it was the
only thing addressing the class, and it no longer is.

## Decided: the layer to unify is the node, not the parser

The traversal is already unified and I was wrong to say otherwise. `Navigable`
abstracts a node - `get_child`, `insert_child`, `is_map`, `scan_children` - and
all three text engines walk it through the same `generic_get` / `generic_set` /
`generic_scan`.

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

### Rejected: tree-sitter

It fixes the differing depth limits, ron's lost variants, the per-format
document quirks and comment preservation - and none of the top tier above, which
is path handling, scan logic and the value codec, all of it above the parser.

The costs are not small. Tree-sitter parses and does not print, so three parsers
become three grammars plus three CST mappings plus three writers; even a
splicing printer needs per-format knowledge of quoting and table headers. It is
a C dependency with a grammar's worth of C each, on a library whose sqlite
backend exists for mobile. A CST of a hundred-thousand-entry document is heavy
against an envelope where 100k is already the edge. And two of the five engines
gain nothing.

Running the grammars in WASM removes the C toolchain and adds a WASM runtime,
which is heavier than the C it replaces, and lands on iOS where JIT is
forbidden - the same platform that motivated avoiding C. It would earn its
place only for grammars supplied at run time, which is a different product.

### Rejected: reading a damaged file partially

The one thing tree-sitter is uniquely good at is a tree with error nodes from
broken input, and it is not wanted. An application running on a half-parsed file
runs on state no version of the program ever wrote - neither the old state nor
the defaults, but an arbitrary subset. That is a member of the category above,
added deliberately.

It is not wanted for diagnosis either. What helps when a file is broken is the
file: a person opens it, support attaches it to a ticket. A tree with error
nodes is a worse artefact than the original, and the one useful thing - where it
broke - the existing parsers already give precisely (`recursion limit exceeded
at line 128 column 255`).

The answer to a damaged file stays: refuse, name the position, leave the file
alone, and let the application choose between stopping and starting fresh. This
is also what keeps quarantine file-level rather than structural - a partial read
would be a structural quarantine under another name.

## What the tests do not test

In `TEST-AUDIT.md`, because it is long enough to bury everything around it.

Four passes over the suite, one per area, each asking whether a test would say
so if the behaviour under it were broken. The expected answer was loose error
matching - `is_err()` on a `Report` whose context says exactly what happened -
and that is there. It is not the largest part. The largest part is tests that
**never run** or that **cannot fail**: a macro golden that `macrotest` writes
for itself rather than failing, an adapter crate outside the workspace whose 324
lines of tests only build on one machine, and about a dozen tests that stay green
under a named one-line mutation of the code they exist to guard.

Every entry there names the line to change, so it is confirmable in one edit
rather than by argument.

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

Not codec limits and not policy - logic in the store, and mostly small. Four of
these are silent data destruction through the public API.

| what | where | confirmed |
| --- | --- | --- |
| a path that computed to nothing is the root, and a struct written there replaces the whole document | shared `generic_set` | `tests/empty_path_is_the_root.rs` |
| `delete_prefix` destroys siblings whose name has a character below `.` | `utils::key_range` + sqlite skipping `is_under` | measured, 6 characters |
| reordering two same-typed struct fields silently swaps their values | the binary codec writes structs positionally | `tests/field_order_is_load_bearing.rs` |
| `scan_keys` of a leaf returns the leaf, so a recursive walk never ends | shared `scan_keys_recursive`, `store.rs:1116` | `tests/scan_keys_of_a_leaf.rs`, all three text engines |
| a migration's prefix scan uses `starts_with` / `GLOB` rather than `is_under` | `redb/migration.rs:116`, `sqlite/migration.rs:132` | redb measured; sqlite read, not run |
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
disagreement is reported rather than discovered later, which is the decision
already recorded under *a document engine refuses where it cannot represent*.

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

`where_a_value_is_written_does_not_decide_whether_it_can_be_read` is `#[ignore]`
on that contrast: the same `Deep(120)` survives at a two-level path and not at a
ten-level one. A check that round-trips the value in isolation passes it in both
places and is therefore wrong, which is worth writing down because it is the
cheap implementation and the obvious one to reach for.

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

- `set_or_create` appears in five pages and exists nowhere; it is `insert`
  since the rename. One section is built entirely on it.
- `StoreBuilder::collect_migrations` and `amethystate::Result` do not exist.
- The migration pages destructure a report out of `build()`, which returns a
  store. `Migrations/overview.md` also documents a `~` row - `field 'port':
  u16 -> u32` - in the drift output, which `log_to_tracing` cannot print: the
  diff is `added` and `removed` only, and a type change under one name nags with
  no field named at all. That is deliberate and pinned by
  `a_type_that_changed_under_one_name_nags_without_a_diff`; only the page
  disagrees.
- `Concepts/reactive-cell.md` documents the owning cell throughout: it teaches
  building cells and dropping the struct they came from, which now yields a map
  of dead cells, and never mentions `into_cell`, `into_entry_cell` or
  `Kv::cell`. `entry_cell` is shown with a `default` argument it no longer
  takes, and `get()` is used as `T` rather than `Option<T>`, so several
  snippets would not compile.
- `Concepts/kv.md` predates namespaces: `keys` is shown with an argument, and
  every dotted example now addresses one name rather than the levels it means.
  It also teaches the type check (`// Err(TypeMismatch)`), which is gone along
  with the variant; what refuses a `Kv` write is ownership of the path.
- The dioxus and leptos pages name the provider component `amethystateProvider`;
  it is `AmeStateProvider`. The dioxus page uses both.

Rustdoc has its own: the macro's own documentation gives constructors that do
not exist (`new(&Arc<Store>)` where the real one takes no arguments and the
store is already `Arc`-backed), and says `default` is required on leaf fields
where the code falls back to `Default::default()`. `Kv::set` and `Kv::remove`
open by promising the durability their `Durable` counterparts provide.

`Concepts/observability.md` promises `location` is the caller's `file:line`,
which it now is: `#[track_caller]` runs the whole way through the `Watch`
builder - `register`, `register_with_source`, `stream`, the `watch_raw`
declaration and its four implementations - so a subscription made the way the
subscriptions chapter teaches records the call site rather than a line in this
library.

### Done: the pages that went with the access modes and the lookups

`State/defining-structs.md` lost the *Cross-struct references* section and the
four attribute rows; `Migrations/overview.md` lost its `lookup` row; the three
integration pages now describe `use_read_only_field` by what it returns rather
than by a handle kind that no longer exists; and the macro's own rustdoc lost
the same table rows and its *Lookups and Permissions* example, which was
`rust,ignore` and so had never been compiled.

What is left of the dependency ordering in `Migrations/overview.md` still has to
be revisited once the graph is demand-driven.

## What tampering with a text document does, found by doing it

`tests/tamper_*.rs` write a store, edit the file the way a person or another
tool would, and reopen. Every failing test asserts the behaviour that would be
right, so its failure message is the finding. Worst first; six of these lose
data with no error at all.

The suite is ordinary tests now: what still fails carries an `#[ignore]` naming
the finding, and everything else is green. Every file but
`tamper_engine_contrast.rs` is gated on a text feature, and that one is the
control - on redb and sqlite it passes, which is the point of it.

**A gate is not a choice of engine.** `#![cfg(any(feature = "json", ...))]`
decides whether a file is compiled; which engine it runs against is
`default_backend()`, and that prefers redb. With `default = ["redb"]`, any
build that has redb on runs these tests against redb, so the seeded document is
a file the store never opens - which is the `--all-features` cell of CI. Under
`--features json` the suite was 0 of 6 on `tamper_names`, and `watcher_race`
was green while testing nothing, the file watcher being a text-engine part that
redb does not have. Every store in the eight affected files now names its
backend, through `common::text_backend()` where the format follows the build
and `Backend::Toml` in the toml-only file. A test about documents that does not
say which engine it wants is asserting about redb.

**A level named `.` is the whole document.** `normalise_parts` maps `["."]` to
the root (`document.rs:45`), and `StorePath::segment(".")` is a legal one-level
path, so `kv.set(".", &value)` replaces the entire document and `get_raw` on
`.` returns the whole store. `delete(["."])` removes nothing and emits a
`Delete` anyway. json, toml, ron; redb and sqlite have no root alias and are
unaffected. `tamper_dot_sentinel.rs`, 7 failures on each format.

**An empty TOML file is a valid empty document.** `TomlDocument::parse`
(`toml_doc.rs:84`) has no root check, where json and ron reject a non-object
root. An editor's truncate-then-write window therefore reads as "every key
deleted": subscribers are told, and the next save writes the emptiness back.
The watcher's debounce cannot help, because the truncated file parses.
`tamper_live.rs`, `tamper_toml_inline.rs`.

**Writing under a TOML inline table or array-of-tables empties it.**
`ensure_map` tests `is_table()`, false for `Item::Value(InlineTable)` and
`ArrayOfTables`, and replaces the node (`toml_doc.rs:24`). `cfg = { width,
height }` plus one `set(["cfg","scale"])` loses both. `tamper_toml_inline.rs`.

**A declared section holding a scalar or a list is wiped at startup.** Same
`ensure_map` in all three formats; `field_with_path` writes its default when the
read is `None`, and the walk to the parent replaces whatever stood there.
`tamper_shapes.rs`.

**TOML reads a section back as one of its children.** `with_bytes_de` renders a
non-value node as `val = ...` and cuts at the first `=`, which for a table is
the one inside it (`toml_doc.rs:150`). `[cfg.width]\npx = 800` reads as
`Some(800)`. json and ron error here, which is the right answer.

**Deleting inside a TOML inline table reports success and removes nothing.**
`remove_child` uses `as_table_mut()` (`toml_doc.rs:33`); `store.rs:426` emits
the `Delete` regardless, so a bound `Field` resets to its default while the
store still holds the old value, and a restart brings it back.

**The metadata is a second file nothing binds to the data.** Versions,
snapshots and `__init` markers live in `path.with_extension("meta")`
(`store.rs:191`). Losing it replays migrations over migrated data - 21 doubles
to 42, then to 84 - restores defaults the user deleted, and a forged marker
suppresses the real ones. redb and sqlite keep this in the same transaction as
the data, so it cannot come apart. `tamper_meta.rs`.

Decided: bind the two files rather than merge them. Folding the metadata into
the data document would make every save rewrite bookkeeping that can be large
next to the data it describes. Instead the metadata carries a checksum of the
data, which has to be *maintained* and not only checked - a checksum that goes
stale on the first ordinary write reports a divergence on every startup - so it
is written in the same save, data first and metadata second, and a crash between
the two reads as a divergence rather than as quietly wrong state.

What a divergence then means: the metadata is untrusted, so nothing is replayed
and nothing is re-seeded from it. Versions cannot be recovered, so a migration
does not run and says why. The `__init` markers can be recovered, through the
empty node written up above.

**An unrelated pending write rolls back a concurrent external edit.**
`sync_external_changes` refuses to pull while `writes != persisted`
(`store.rs:826`) and a persist writes the whole document from memory, so one
buffered write anywhere discards every hand edit, including to untouched keys.
`tamper_live.rs`.

**A broken external edit is dropped without a word and then overwritten.**
`D::parse` fails, `sync_external_changes` returns early (`store.rs:815`),
nothing reaches the caller, and the next save replaces the half-written file.

**The data and metadata shared one backup path.** Fixed: the copy keeps the
whole name and adds `.bak`, so `store.db.bak` and `store.meta.bak` are two
files. `with_extension("bak")` gave `store.bak` for both, so the second copy
landed on the first and the data had no backup left - and it named a file the
store never created, a `store.bak` a person put there themselves, which it
overwrote and then deleted. Both tests in `tamper_broken_file.rs` run now.

**A key with no name is invisible to every scan.** Decided: it stays that way,
and now says so. A document may hold `{"": 1}` and a level with no name is not a
path, so the scan passes over it and logs at `warn` - listing it would hand back
a key that does not read back as a path, and refusing would let one name nobody
meant to write stop the store from listing anything else. The value keeps its
place in the file and survives a save; nothing addressed by a path reaches it.
Written up on `scan_keys` and `Kv::keys` through `store/scan_contract.md`.

Making it addressable was weighed and dropped. Only one case is genuinely
ambiguous - `["cfg", ""]` already joins to `"cfg."` and is merely refused, while
`[""]` collides with the root - so a marker pair such as `\0` would settle it.
That means changing `join`, `parse_joined` and `joins_to` together, in the one
function that has no right to be wrong, to address a key that nothing in the
library can write and nobody writes on purpose.

**Duplicate keys diverge, and every engine is already right.** `{"a":1,"a":2}`
opens on json and ron with the last value winning and the first gone at the next
save; toml refuses to open, naming the line. Neither is a defect of ours: the
TOML spec forbids a key defined twice, and RFC 8259 leaves it undefined, so
last-wins is what every other json tool does with the same file. Unifying them
would make each engine behave unlike every tool a person edits that format with,
which is the more surprising answer. The parsers resolve it before the document
reaches us, so there is nothing to report even if we wanted to.

Held up under the same tampering, worth knowing: wrong scalar types at a
declared field fail loudly on all three; undeclared keys survive a rewrite; a
truncated or scalar-rooted file is refused and left byte-for-byte intact; a
scan over a prefix lists the value at the prefix itself identically on all five
engines.

## What the conformance suite says the engines disagree about

`tests/backend_conformance.rs` states twenty-nine properties about what a store
is and runs each against every engine compiled in. redb and sqlite pass all
twenty-nine. What the three text formats fail is the finding: they share one
implementation and diverge from the flat engines in exactly one place, the
document walk.

json and ron fail two: `a_scan_lists_exactly_what_is_under_the_prefix` and
`writing_then_deleting_leaves_the_store_as_it_was`. toml fails those and
`an_ancestor_is_not_a_value`, through the `with_bytes_de` cut at the first `=`.

**Where the scan one actually comes from, traced.** `scan_prefix_impl` and
`scan_keys_impl` (`text/store.rs`) both pass `target_depth = parts.len() + 1`,
so the walk descends exactly one level below the prefix. A value written three
levels down is reported at the intermediate node - the failing case scans `..`
and gets back `\.\..\\` where `\.\..\\.\\` was written - and `node_to_bytes` on
that intermediate node hands back a serialized submap as if it were a value.
That is also the toml `an_ancestor_is_not_a_value` failure.

**And the cap cannot simply be lifted.** A struct value is stored as one node,
and on json that node *is* an object with children, so an unbounded walk
descends into the struct's own fields and reports each as a key. The cap is a
workaround that happens to hold while values live exactly one level below their
prefix. The document has no way to say "this map is a value, not structure" -
which is the fork recorded at the end of this file, and the schema-declared
boundary is what closes it. Nothing smaller does.

Four that used to fail no longer do. `a_level_named_dot_is_an_ordinary_level`
and `a_leaf_and_a_branch_coexist_at_one_name` are written up above.
`a_write_leaves_every_other_path_alone` went with the second path parser.
`deleting_what_is_not_there_changes_nothing` was toml alone, and its cause was
`Navigable::get_child_mut` reaching a child through `Item`'s `Index`, which
inserts the key it is asked for - so the walk to an absent path built the
levels on the way. It now goes through `as_table_like_mut`, which is what
`remove_child` beside it already did.

Read those counts against the next paragraph: which inputs a property sees is
not the same twice.

**The failing set moves between runs, so it is not a gate.** `config()` sets
`cases: 24` with `failure_persistence: None` and no seed, so every run draws
different names. Two runs of the same tree gave json 2 and toml 4 one time and
json 3 and toml 3 another - a genuine regression is indistinguishable from a
different draw. Either pin the seed for the properties whose divergence is
recorded, or `cfg_attr`-ignore them per engine so what is green is decided
rather than drawn. The generated-input value is worth keeping somewhere; it is
worth keeping away from the set that says whether the tree is broken.

### Decided: a document engine refuses where it cannot represent

A tree cannot hold a value at a node and values under it at once, so property 12
asks the text engines for a document that does not exist. Three ways out were
weighed: make the flat engines enforce the same restriction (a range scan on
every write, and it forbids what those engines can do perfectly well); give the
document a reserved key for "the value of this node" (kills hand-editability,
which is the reason the text engines exist, and collides with a real key of that
name); or let the engines differ and replace destruction with refusal.

The third. Property 12 becomes a disjunction - the two coexist, *or* the second
write is refused and the first survives - which all five engines can satisfy and
which still forbids the thing that actually hurts, silent destruction.

That generalises: the suite states one contract for engines built on genuinely
different substrates, and the parts of it a document cannot honour are better
recorded per engine than demanded of everyone. What stays universal is the
narrow surface the schema itself uses, because the generated code calls
`field_with_path` without knowing the engine and is unsound if that surface
differs.

Two things the change has to get right. The destruction is one line, written
three times - `ensure_map` replaces the node when it is not a map
(`json_doc.rs:25`, `toml_doc.rs:25`, `ron_doc.rs:33`), and `insert_child` calls
it, so both write orders destroy through it. And the refusal must not travel up
through `field_with_path`'s seeding write, which nobody asked for: a field whose
parent is occupied keeps its default in memory and leaves the file alone,
rather than failing the whole struct.

The collision is reachable from the schema, not only from `Kv` - `prefix =
"root"` with a field `b` alongside `prefix = "root.b"`, as written up above - so
the refusal has to name both declarations, not just the two paths.

### What the refusal can and cannot see

Half of it is undetectable, which the first attempt at the change proved by
breaking every migration test on the text engines. A serialized struct is a map
with children; so is a level with values under it. In a document the two are the
same bytes. The store's own bookkeeping writes a struct at `schema.<prefix>` and
also writes under it, so a rule of "refuse a write at a level that has children"
refuses the library's own meta writes.

So the two directions are not symmetric:

- Writing *under* a level that holds a plain value is unambiguous - a scalar is
  never a branch - and is refused.
- Writing *at* a level that has children is refused only when the incoming value
  is not itself a map. A map written over a map is taken as the update it almost
  always is.

What is left uncovered: a struct written over a level that had unrelated values
under it. The flat engines keep both, a document engine cannot, and nothing in
the bytes says which was meant. That is the residual divergence, and property 12
is written to allow it rather than to pretend otherwise.

It kills a third idea too, and this one is worth writing down because it looks
harmless. Pruning a branch that a delete just emptied would fix
`writing_then_deleting_leaves_the_store_as_it_was` - the byte-identity property -
but a node that has just lost its last child is `{}`, and a field whose value is
an empty map is stored as `{}` as well. Deleting inside a stored map would then
delete the field. The property it buys is cosmetic and the failure it risks is
not, so the empty node stays and the property stays recorded. `delete_prefix`
removes the subtree node whole, and `load_map` skips a scanned key equal to the
map's own path, which is where the leftover actually used to hurt.

### The empty node is load-bearing, not litter

There is a second and stronger reason not to prune it, found while working out
what a lost metadata file can be recovered from. "This namespace was seeded" is
one bit that no amount of reading the data reproduces - except that it does,
through exactly this leftover:

```
{ "items": {} }        the map existed and was emptied  -> do not seed
{ "unrelated": 1 }     the map never existed            -> seed
```

Without it the two are the same observable state, and `tamper_meta`'s
`losing_the_metadata_file_does_not_resurrect_removed_defaults` and
`a_forged_marker_does_not_suppress_the_defaults` demand opposite answers for it.
So the byte-identity property is not a deferred fix, it is a permanent
divergence: a document engine cannot both round-trip byte for byte and remember
that a namespace was once written.

The flat engines have no such node - there is no key at `items` - and need none:
their metadata lives in the same transaction as the data and cannot be lost on
its own. The recovery route exists exactly where it is needed.

The same ambiguity kills the matching idea for `delete`, and there it cannot be
worked around. `delete` refusing to remove a node with children looks right -
`delete(["a"])` where only `a.b` exists should take nothing, which is what a
flat engine holding no key at `a` answers - but a field whose value is a map or
a struct is stored as exactly that node, so the rule refuses to delete it.
`set` can tell the two apart by looking at the value being written; `delete` is
handed nothing but a path. So it removes whatever is there, and property 5
belongs in the same recorded-divergence bucket as property 12 rather than being
demanded of everyone.

**A scan on a text engine goes one level deep.** `scan_prefix_impl` and
`scan_keys_impl` set `target_depth = parts.len() + 1` (`text/store.rs:825`,
`:1004`), so a scan lists direct children only; anything deeper comes back as
the intermediate branch, with a serialized subtree for its value. redb ranges
the whole subtree and sqlite ranges `key_range`, so both list every key at any
depth. A value three levels down is invisible to a scan of its grandparent.
`ReactiveMap` survives this only because a map's entries are always exactly one
level below it. `Store::scan_keys` means two different things depending on the
engine.

**`delete` at a path that holds no value takes everything under it.**
`generic_delete` removes the node, so `delete(["a"])` where only `a.b` exists
deletes `a.b`. On the flat engines there is no key at `a` and nothing happens.
On a document engine `delete` and `delete_prefix` are the same call.

**On toml, deleting an absent path creates the levels on the way to it.**
`Navigable::get_child_mut` for toml is `Item::get_mut`, which is
`Index::index_mut`, which does `entry(key).or_insert(Item::None)`.
`generic_delete` walks the heads with it, so the walk vivifies, and the phantom
branches are then listed by the next scan. json and ron do not - a difference
*within* the shared text implementation.

**Reading a path that holds no value but has values under it gives three
answers.** redb and sqlite say `Ok(None)`. json and ron give a decode error, the
branch object not being a `u32`. toml gives the child's value, through the
`with_bytes_de` cut at the first `=`. None of the text answers is `None`.

**The error model does not agree.** redb and sqlite report undecodable bytes
with `current_context() == StorageError::Codec`. All three text engines wrap it
once more at `text/store.rs:652`, so the outermost context is `Read` and `Codec`
is a frame below. A caller matching on `current_context()` cannot tell "the
bytes are the wrong type" from "the file would not read". This is exactly what
the error model was meant to make assertable.

**Events: covered now, and it found what it was written to find.** Properties
22-24 state what one operation emits: a write is one `Set` carrying the value
that landed and the one it replaced; a delete is one `Delete` carrying the value
that went, and a delete that removed nothing says nothing; `delete_prefix` is
one `DeletePrefix` at the prefix rather than a `Delete` per key.

The middle one failed on **all five** engines, not only the text ones - each
emitted a `Delete` with `old: None, new: None` for a path that held nothing, so
a subscriber acted on a change that did not happen. Each engine now returns
before the event, and before scheduling a flush for a document it did not
change.

Still uncovered: concurrency between two handles, the async surface,
`is_initialized` across a failed flush, and value shapes past `u32`/`String` -
nested structs, enums and sequences are where the three text formats differ most
from each other and from msgpack.

## A cleared map leaves a node behind, and only on the text engines

`clear()` deletes the prefix. On redb and sqlite the keys go and nothing is
left. On the text engines the container stays: after clearing `probe.items` the
json document holds `{"probe": {"items": {}}}`, and the next scan of the prefix
reports the prefix itself as a stored key with an empty object for its value.

Two consequences, one of them already load-bearing. `load_map` reads a scan
strictly, so an empty node at the map's own path was a hard failure on reopen -
`clear_survives_a_store_rebuild` went red on all three text engines. It now
skips a scanned key equal to the map's path, on the grounds that a map's entries
are the level below it and nothing is stored at the path itself; that is right
whatever the engine leaves behind, and it does not soften the strictness about
keys that really are under the path. `map_len` still counts the node, though
`ReactiveMap::len` reads its own projection and so does not.

The root cause is `delete_prefix` not pruning a branch it emptied, which is also
why `writing_then_deleting_leaves_the_store_as_it_was` fails on the text
engines. Fixing the prune fixes both, and would let the skip go.

## An interceptor says why it refused, and the field drops it

`FieldCore::run_interceptors` returns `Err(String)` naming what happened -
`"Maximum intercept depth reached"` is a bug in the caller's code, a refusal by
a filter is not - and both call sites throw it away with `map_err(|_| ...)`
(`field_ops.rs:22`, `reactive/field.rs:452`). The report that reaches the caller
says only "an interceptor rejected the change", so a validating interceptor
turning a value down and interceptors recursing past the depth guard are the
same message.

The map side is fixed: `map_apply_change` attaches the sentence and names what
the change reached, so a refused `insert` and a refused `clear` no longer render
identically. The field wants the same, and the ephemeral branch in
`field.rs:452` wants a `Report` rather than a bare `FieldError`, which is
separately why that one carries no path at all.

**Done.** Both field call sites carry the reason through
`FieldError::intercepted`, and the ephemeral branch builds a `Report` naming the
field and saying that nothing was going to be stored either way. Both are pinned
by snapshot in `tests/error_reports.rs`, so a refusal that collapses back to one
sentence fails a test.

## The schema belongs in the store, as JSON Schema

Its own track, and the answer several entries here have been waiting for.

**Everything migrations, in one place.** The entries are spread through this
file because they were found at different times; what they have in common is
that the code is the only thing that knows a shape. Ordered by what has to be
decided first.

| entry | where it stands |
| --- | --- |
| this one - the schema in the store | decides the rest |
| The fork under all of it: is the file a store, or a picture of a type | the same question, asked wider |
| Metadata carries no format version - deliberately, for now | where a format version would live, once there is a schema |
| Two type hashes, both weak, and the weaker one feeds the gate | stops being the gate; kept as the record |
| Reordering struct fields silently corrupts data on redb | the fold was written and reverted for this reason |
| `build()` runs no generated migrations, and nothing at the call site says so | becomes a comparison the store makes, not a question of who collected what |
| Migration cleanup addresses a field by its Rust name, not by where it is stored | six ignored tests waiting on it |
| Migration cleanup deletes one key, so a composite field survives being dropped | same repair as the row above |
| `#[migrate]` can only be found through the linker | **done** - `#[migrate(explicit)]` and `add_steps` |
| The sqlite migration adapter still scans by `GLOB` | independent, and small |
| A failed migration is invisible through `StoreBuilder::build` | inside *Errors that reach nobody*; independent of all of this |

The last two are the only ones worth touching before the track is decided.
Everything above them is either waiting on it or is the record of why something
was stopped.

The code is the only thing that knows a shape today. A snapshot records what
was seen last time, `inventory` gathers what is declared this time, and a pair
of weak hashes stands in for comparing them - which is why a hash collision is
a data-loss bug rather than a diagnostic one, and why the entry above on
reordering fields had to reason about fold algorithms at all. A schema the
store carries in a form that is not this crate's own would replace the hashes
with a comparison, and the comparison would be able to say *what* differs
rather than *that* something does.

JSON Schema rather than a private format because the point is that the file
answers for itself. A store that carries one can be read by something that is
not this library, migrated by a tool that is not this binary, and diffed by a
person - none of which is possible while the shape lives only in a Rust type
and a `u32`.

JSON Schema is the **model**, not the storage: each engine encodes the document
the way it encodes everything else, msgpack under redb included. Writing it as
literal text inside a binary store so that `strings` finds it buys nothing - a
binary engine needs a viewer either way, and sqlite's existing viewers already
render its json, so nothing about them changes.

**Decided, and it is two halves.** Plain JSON Schema for what a value looks
like - nothing of this crate's invention in it, so anything that reads JSON
Schema reads ours. And this store's own semantics as a role per declared path,
from a closed set: `field`, `map`, `nested`, `table`. What the schema cannot
say is which paths are levels and which hold values, and that is exactly what a
store needs to know; the roles say it and nothing else has to.

Three of the four are already in the code - `Role::Field`, `Role::Map`,
`Role::Node`, which is `nested` under another name - and carried per field in
`FieldDescriptor`. `table` arrives with the primitive; see the reactive-table
RFC. So the vocabulary is not being invented here, it is being written down and
made persistent.

### How the shape is learned: ask the compiler, not the spelling

A field contributes four scalars to the document - its name, its role, whether
it is optional, and the name of its type. Three of those are answers about a
type, and the compiler is the one that has them.

`AmeType` is required of nested `_Data` structs, which the macro generates, and
of nothing else. A leaf may be any type at all, including a foreign one from a
crate where no derive can be added, and what the macro writes about it is its
name. Describing it further at compile time would mean requiring a derive on
user types, which costs more than the description is worth - but at run time
serde will describe it for nothing, which is *A leaf is opaque at compile time,
and serde can open it at run time* below.

The questions are asked through a probe, which is inherent impls for the shapes
this crate knows and a trait fallback for every other type:

```rust
pub struct Probe<T: ?Sized>(PhantomData<T>);

pub trait AnyShape { const OPTIONAL: bool = false; const ROLE: Role = Role::Field; }
impl<T: ?Sized> AnyShape for Probe<T> {}

impl<T> Probe<Option<T>>            { pub const OPTIONAL: bool = true; }
impl<K, V> Probe<ReactiveMap<K, V>> { pub const ROLE: Role = Role::Map; }
```

An inherent associated const shadows the trait's, so the compiler picks. The
macro emits `<Probe<#ty>>::ROLE` and never looks at how the type was written.
Measured on stable 1.95, including in `const` context.

Two properties follow, and both are why this rather than matching on the
spelled name:

- **`Option<Foreign>` answers `true` while `Foreign` implements nothing.** The
  modifier is visible without the inner type being bound by anything.
- **Aliases and renamed imports resolve.** `type Maybe = Option<Foreign>` is
  optional, `use Option as Perhaps` is optional, `type Port = u16` is not. A
  name match would answer all three wrong and report drift for a rename.

`OPTIONAL` decides whether `null` joins the property's type and whether the
property is in `required`. It says nothing about what is underneath, which is
why the probe needs no way to hand back an inner type - it answers predicates
only, so it needs no associated types and depends on no unstable feature.

**Built, in `shape.rs`, and it reaches the file.** `Role` and `optional` come to
`FieldDescriptor` from the probe, the branch the macro picked from the spelling
is asserted against what the type answers, and `SchemaSnapshot` carries the
whole thing down - `StoredShape` per field, recursively through a `Node`'s
children, which the snapshot did not record at all before. So the shape is in
the store rather than only in the binary that opened it, which is the half this
track exists for. See *The macro picks the branch, the compiler judges the pick*
for what is left on the macro end.

A snapshot written before this holds no `shape`, and it reads back as `None`
rather than as a default. Absent has to mean unknown: a comparison that read the
default as a claim would report every store written before today as having
changed shape, when all that changed is what gets written down.

**What a field records is its name, its shape, and its spelling.** No
`type_hash`: the only thing that read it was the per-field type comparison, and
the alternative - comparing `type_name` - is the mistake the probe exists to
avoid, since a spelling moves when a rename or an alias does and the type has
not. So `type_name` stays as what a person or the inspector reads, and nothing
compares it.

**Which leaves the comparison saying less, on purpose.** `SchemaDiff` is
`added` and `removed`, by name. A field whose type changed under one name nags -
the whole-struct hashes still disagree - and the diff has nothing to say about
it, which `a_type_that_changed_under_one_name_nags_without_a_diff` pins. What
replaces it is a comparison of two schema documents, and that is this track.

### A leaf is opaque at compile time, and serde can open it at run time

What the macro writes down about a leaf is the name of its type, because that is
all a macro can know without requiring a derive on user types. So these three
write the same record - `"Mode"`, role `Field`, not optional:

| before | after |
| --- | --- |
| `enum Mode { A, B }` | `enum Mode { A, B, C }` |
| `enum Mode { A, B }` | `enum Mode { X, Y }` |
| `struct P { x: u8 }` | `struct P { x: u8, y: u8 }` |

**But serde already knows, and it will say.** A derived `Deserialize` tells the
`Deserializer` what it expects before any data is read, and the arguments carry
the answer:

```rust
fn deserialize_struct(self, name, fields: &'static [&'static str], v)
fn deserialize_enum(self, name, variants: &'static [&'static str], v)
```

A `Deserializer` written to record which method was called and stop reports
`Struct { name: "Point", fields: ["x", "y"] }` and
`Enum { name: "Mode", variants: ["Idle", "Busy", "Off"] }` - the whole variant
list, from a type that implements nothing of this crate's. No instance is
needed: this is `T::deserialize`, not `Serialize`, which would need a value and
would then show one variant out of three. Measured, not assumed.

The only bound is `Deserialize`, which every leaf already satisfies - a leaf
that could not be deserialized could not be stored.

Driving that to the end is what
[`serde-reflection`](https://crates.io/crates/serde-reflection) does, and what
it yields is the whole recursive shape:

```
Leaf:  Struct([mode: TypeName("Mode"), where_: TypeName("Point"),
               tags: Seq(Str), note: Option(Str)])
Mode:  Enum({0: Idle → Unit, 1: Busy → Struct([since: U64]),
             2: Off → NewType(Bool)})
Point: Struct([x: U8, y: U8])
```

Including the variant *indices*, which is what a non-self-describing codec
writes - msgpack under redb - so an old registry is enough to reread old bytes
rather than only to notice they changed.

**Where tracing gives up**, from the crate's own error type and measured:

| cause | error | recourse |
| --- | --- | --- |
| `flatten`, `tag`, `tag` + `content`, `untagged` | `NotSupported` | none - the type is not ours |
| a hand-written `Deserialize` that validates | `Custom` | trace a value instead |
| an enum met only inside a container | `MissingVariants` | trace that enum by name, which we cannot reach |
| an empty `Option`, `Vec` or map in a traced value | `UnknownFormatInContainer` | a fuller value |
| `Serialize` and `Deserialize` disagreeing | `Incompatible` | none |

The first row is the uncomfortable one: those attributes exist for
self-describing formats, which is what json, toml and ron are, so a config
struct using `#[serde(flatten)]` is not exotic and will not be described.

`trace_type` re-drives passes only for `T` itself, so an enum reached through a
struct stays incomplete - and `registry_unchecked` must not be used to paper
over it, because an enum recorded with one variant of three reads later as two
variants removed. A partially seen enum is not written down.

**So the rule is quiet:** `trace_type`, failing that `trace_value` from the
declared default, failing that no entry in the document at all - and a leaf with
no entry is described by the name of its type, which is exactly where this
already stands. Tracing cannot make the record worse, only longer, and its
failure costs nothing. Opening a store must not fail because someone's type
would not describe itself.

Two things to hold on to when this is built. The registry is *forward*-looking:
comparing needs the older document to have been written at the time, same as the
shape record. And `serde_reflection`'s own `ContainerFormat` must not be
persisted as-is - that would adopt another crate's data model as this store's
file format; it maps into the document described above.

What it settles, and why it is a track rather than a task:

- **The two weak hashes** stop being the gate, so neither has to be made
  strong. The entry above stays as the record of what they could not see.
- **Metadata carrying no format version** is the same question asked smaller: a
  schema in the store is where a format version would live.
- **`build` running no generated migrations** turns from a silent skip into a
  comparison the store can make: the schema says what shape is expected, and a
  store that does not match it can say so without depending on who collected
  which steps.
- **Reading the schema from the store instead of `inventory`** - which the fork
  below already names - is this, done.

Nothing here should be patched around while this is undecided, which is why
the hash work above was stopped rather than finished.

## The fork under all of it: is the file a store, or a picture of a type

Two models, and the library is currently both.

**A picture of a type.** `to_writer(file, &config)`. The file is nested because
the struct is, a person opening it sees their own shape, and there is nothing to
address: the root is read and written whole. This is what a plain
`to_writer(file, &config)` gives, and what `as_root` writes.

**A store that happens to be text.** Keys are paths, each with its own value,
its own event, its own deletion. VS Code's `settings.json` is this - flat, keys
like `"editor.fontSize"`, no object under `editor`.

We address by path, which promises the second, and write nested, which delivers
the first. Every ambiguity recorded in this file lives on that seam: a
serialized struct and a branch are the same bytes, a leaf and a branch cannot
share a name, `delete` and `delete_prefix` are one call on a document, a cleared
map leaves a node behind, and a scan has to guess how deep a value is.

**Writing the joined path as the key would end it.** The key becomes exactly
`StorePath::as_str()` - the same string the flat engines already use, escaping
included, injectivity already proved. Then a document holds keys, not nodes:

```json
{ "cfg.panels.left": { "width": 800 } }
```

`cfg.panels.left` is a key and `{"width": 800}` is its value; that the value is
an object stops being anyone's business. The picture-of-a-type model does not
die, it moves inside the value, which is where VS Code keeps it too
(`"editor.rulers": [80, 120]`).

What it would close: `Occupied` and both its variants, the empty node after
`clear`, `a_leaf_and_a_branch_coexist_at_one_name`, the depth of a scan, the
difference between `delete` and `delete_prefix` on a document, and `Navigable`
with the four `generic_*` walkers - `TextDocument` becomes a flat map.

What it would cost: every file already written nested needs migrating on open;
any file some other library wrote by serializing a struct whole is nested, and
stays a foreign format to import rather than our own; toml needs quoted keys (`"editor.fontSize" = 14`), which is legal and
against its idiom; and a path into the interior of a value - `["cfg", "panels",
"left", "width"]` - stops resolving, though `#[amestate(nested)]` already writes
each leaf as its own key and so is unaffected.

### Decided: structure where a schema declares it, flat keys everywhere else

Neither extreme. A document is nested exactly as far as a schema says it is, and
whatever no schema declared is one flat key at the deepest declared point:

```json
{ "app": { "width": 1280, "panel": { "visible": true }, "myplugin.enabled": true } }
```

`app` and `panel` are nested because they are declared nodes. `myplugin.enabled`
is one key because nothing declared it. The ambiguity becomes a decision rather
than a guess: **a node is a level exactly when the schema says so, and anything
else is a value.** Undeclared data cannot be ambiguous at all, since it is never
nested.

That also settles the case that silently corrupted a map: a declared `map`'s
entries are the level below it, so `panels.left` is an entry and `{"width":
800}` is its value, whole. The walk stops there because a role said where the
boundary is, not because of a depth cut.

**What the record has to hold, and it is small.** At each point the reader
answers one question - descend, or take this whole - so the record is a tree of
name and role, and nothing else:

| role | the reader does |
| --- | --- |
| `node` | descends by name |
| `map` | descends one level; each entry is a value |
| `field` | stops; the node is the value |
| no record | nothing was nested here, so the key is a joined remainder - split it |

`FieldDescriptor` already carries exactly this. `type_hash` and `type_name` are
not consulted in any of the four cases, so the layout needs two of the four
layers - path and role - and nothing about types. `AmeType` leaves the critical
path and stays a migration concern.

**The prerequisite, now load-bearing.** Reading the file needs the schema, not
just writing it: without one, `{"panels": {"left": {"width": 800}}}` could be an
entry holding a struct or three levels, and no amount of string handling tells
them apart - the escaped flat remainder reads correctly without a schema, a
struct-valued node does not. So the snapshot has to be read from the store, per
prefix, and "the set it reads is the linked one" above stops being deferrable:
two binaries linking different structs would otherwise read one file into
different data rather than merely refusing different writes.

At read time the file's own snapshot wins - it describes how the file was laid
out. At write time the code's declaration wins. The difference between them is
drift, which is the migration half.

**Two edge rules.** The metadata file is always flat, because reading the data
file needs the schema and the schema lives in the metadata - a file that can
only be read once you have read it is not a design. And a prefix with no
snapshot is read flat: nothing was declared, so nothing was nested.

**Decided: a migration has no layout at all.** Two schemas are live while one
runs - the file was laid out by the old one and the code declares the new - and
they disagree about the same string. If `app.panel` was a `field` holding a
serialized struct and becomes a `node` with `width` under it, the bytes are the
same and only the schema says which reading is right.

Reading by the file's schema and writing by the code's does not survive the
ordinary case: step 2 reads what step 1 wrote, so the write went down in the new
layout and the read looks for it in the old. That is the mainline, not an edge.

So the prefix is flattened for the duration - joined key to value, no levels -
the steps run against that, and the document is laid out once at the end from
the new schema. There is nothing to invent: `MigrationContext` already addresses
by joined key, `ctx.get(key)` and `ctx.set(key)` with `key: &str`, so the flat
view *is* what a step already works with. The whole class of "which schema
applies right now" then does not arise, rather than being answered by a rule.

The alternative was one live schema mutated path by path as the migration
rewrote each. More precise - it leaves paths the migration never touched alone -
but that precision is only worth something while a layout changes in part, and
after this it never does: the document is written out whole.

The cost is the prefix in memory and rewritten entire, which is not a cost on
the engines this applies to. A text engine holds its whole document anyway, and
nobody puts a hundred thousand records in one.

Migration of files already written nested is not a concern - there are no users.

**Done so far:** the metadata file is flat. Its keys were `["meta", prefix]` -
two levels whose second name held the dots itself - and are now one joined key,
`meta.app.panel`. That also ends a quieter oddity: an `as_root` struct's
namespace is the empty string, so its marker was written as a child with no
name, which is exactly what a scan reports as a name no path can hold.

## The text engines take a path apart and put it back on every call

`TextDocument` addresses a node by `&[&str]`, so every `get`, `set` and `delete`
allocates a `Vec<&str>` out of a `StorePath` that already holds the levels, and
the scan walkers allocate one more per child. `generic_scan` then builds a
`StorePath` back out of that slice to compose the child keys.

The second parser is gone: `split_path` cut the joined form by
`str::split('.')`, knew nothing about the escape, and sent `delete_prefix` at a
level that was not there - so the delete removed nothing and returned `Ok(())`.
`delete_prefix` now hands the whole subtree to `delete_subtree`, and the
document walkers compose child keys through `StorePath::try_push`. The tamper
suite that reproduced it is ordinary tests under `tests/`.

The same family, found since and fixed: the scan walkers asked the joined
prefix `!prefix_str.ends_with('.')` before listing the value at the prefix
itself. A trailing dot in the joined form is an escaped one - `cfg.b\.` is a
level called `b.` - so the value at any such path was missing from its own
scan, on the text engines only. Pinned in
`tests/delete_prefix_dotted_keys.rs`.

What is left is the cost, not a defect: `TextDocument` addresses a node by
`&[&str]`, so the levels are taken out of a `StorePath` that already holds them
and put back again per call. The trait should take `&StorePath` and walk it by
`segment_at`, and `scan` should hand back the child's name rather than a joined
string. Nothing outside these three files sees the trait, so it costs the three
document impls and the two walkers.

**But it is not the mechanical change it looks like.** Two callers mean two
different things by `parts`. The data document is addressed by levels. The meta
document is addressed by *one* level: `store::meta_key` builds a `StorePath`
like `meta.ui.theme` and `text/migration.rs` passes it as `&[key.as_str()]`, so
the sidecar holds flat keys with literal dots inside one name. Change the
signature to `&StorePath` and those calls silently start nesting - the meta file
re-lays itself out on disk. It wants two operations, not one signature.

## One conformance suite for the backends, run against each

Every engine has its own unit tests, written when it was written, and they
overlap by accident rather than by design. Almost every defect found this week
was a difference between engines that no single suite was watching: a prefix
scan that stopped at a level on one and at a character on another, a key with a
separator that survived on the flat engines and split on the tree ones, a
migration cleanup that removed a subtree on one family and nothing on the other.

What is wanted is one set of tests, parameterised by engine, that says what a
store is regardless of which one is underneath - and a per-engine file left with
only what is genuinely particular to it.

`tests/durability_crash.rs` is what one of these looks like: one statement, run
against every engine compiled in. Widening it from redb alone immediately turned
up a difference nothing was watching - see the granularity entry above.

A good part of it belongs as properties rather than examples, because the
statements are universally quantified and the interesting inputs are the ones
nobody thinks to write: a value written at a path reads back at that path and
nowhere else; a scan under a prefix returns exactly the keys written under it;
`delete_prefix` removes exactly the subtree and nothing beside it; a name
holding the separator stays one level through a write, a reopen and a scan.

**After the error model, not before.** Half of what such a suite should pin is
what happens when an operation fails - which error, for which cause - and today
those are not distinguishable enough to assert. Written now it would test the
successes and stay silent about the failures, which is the half that differs.

### The suite draws different inputs every run

`config()` sets `cases: 24`, `failure_persistence: None` and no seed, so which
paths and names a property sees is fresh each time. The recorded divergences
therefore move: two runs an hour apart gave json 2 failures and json 3, and a
property that failed on json passed on toml in one run and the reverse in the
other. A regression is indistinguishable from a different draw, which is the
one thing a suite kept as a gate has to be able to say.

Either pin the seed for the properties that record a divergence, or
`cfg_attr`-ignore those per engine so what is green is green every run. The
second says which engine fails which property in the source, where the reader
is, rather than in whichever run they happen to read.

**Done, and by neither of those.** `failure_persistence` is on: a
counterexample is recorded in `.proptest-regressions` beside the suite and
replayed before the new draws, so what fails once fails every run afterwards.
Two are recorded already, both shrunk to a name that is a lone backslash.
Three toml runs now name the same three properties where the count used to
wander.

Pinning the seed would have frozen the suite into twenty-four examples that
never find anything again - determinism bought by ending the search, which is
the opposite of what a property suite is for. `cfg_attr`-ignoring the three was
dropped for a different reason: they are not accepted divergences. Each waits
on a question open above - the scan's depth, the empty node after a delete,
toml's `with_bytes_de` - and marking them ignored would have decided those by
hiding them.

### What the suite does not reach yet

Ordered by what it costs. **Events**: `StoreOp` appears nowhere in the tests,
and `StoreEvent`'s `old` and `new` bytes are asserted nowhere - one operation
emitting a different op or different bytes per engine is unwatched, and
`text/store.rs` emits a `Delete` for a removal that did not happen.
**Concurrency between two handles**: two handles on one store exist in the
tests but are only ever driven in sequence. **The async surface**: two files,
both through `block_on`. **`is_initialized`**: the happy path only, never
across a failed flush. **Value shapes**: the conformance suite writes `u32` and
one `String`; enums, sequences and nested structs - where the formats differ
most - are never round-tripped.

### Two file-watch tests are load-sensitive

`json_store::store_tests::file_watch_emits_set_for_external_change` and
`..._delete_for_external_removal` fail when the machine is running several test
binaries at once and pass on their own. They wait a fixed interval for the
watcher, so what they measure is the machine as much as the store.

## Documentation

The public API is documented with runnable, asserted examples: `Field`,
`ReactiveMap`, `ReactiveCell`, `Kv`, `Store`, `StoreBuilder`, `Watch`, and the
migration builder and context. The macros keep `ignore`
examples - `#[amethystate]` cannot expand inside this crate's own doctests,
because the macro resolves the crate to `crate` and a doctest compiles as a
separate crate where that means something else. Examples reach the same types
through `store::field_with_path` and `Kv`, which need no macro.

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

## The builder named a file for one engine and opened it with another

Reported from an application built on this, not found here, which is the part
worth keeping: it is reachable by the shortest path the API offers.

```rust
StoreBuilder::located(|at| at.app(app, config))?  // settings.redb, from the default engine
    .backend(Backend::Json)                       // changes the engine, not the file
```

Picking a location ends in `new`, which fills in an extension when the path has
none, and it takes it from `default_backend()`. `backend` set only its own
field. So the json engine opened a redb file and failed on its first byte with
`stream did not contain valid UTF-8` - a message about encoding, for a mistake
about which file to open, which is why it cost the reporter a debugging session
rather than a glance. `StoreBuilder::new("app/settings").backend(Json)` does the
same thing with no location involved at all.

**Done by remembering who chose the extension.** The builder keeps
`caller_named_extension`, and `backend` re-derives the extension when the
answer is no. An extension the caller spelled is theirs - a `.conf` some other
tool already watches is not renamed because an engine was named - and one this
crate invented belongs to whichever engine actually runs. Four tests in
`store::builder::tests`, including the two-`backend`-calls case.

The application worked around it by rebuilding the path with `etcetera` and the
right extension, ten lines duplicating this crate's logic. Those can go.
