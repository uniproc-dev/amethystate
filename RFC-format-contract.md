# The on-disk format contract

**Status: in force.** A store records how its bytes were written; a build that
meets a deciding fact it has no name for, or a name it knows at a value it does
not write, refuses the open and says which fact stopped it. This document fixes
what the promise is and - more importantly - which moves stay available.

This is about the *library's own* representation: the key encoding, the
bookkeeping records, the value encoding, the document layout. The application's
schema is a separate mechanism with its own versions and its own migrations, and
the two must not become one.

---

## 1. Two layers, and only one of them moves

The set of facts describing how the data was written has to be read before the
data can be. So it cannot be described by itself, and something has to be true
without being recorded anywhere.

That something is the meta layer's **addressing**, and it is frozen.

| | what | moves |
| --- | --- | --- |
| addressing of the meta | separator `.`, escape `\`, flat keys, one record is one value | never |
| inside a record | `SchemaSnapshot`, the `StoredShape` tree, whatever a record says about the application's data | freely, additively |
| the data | `codec.*`, `path.*`, `layout` | freely, each change named by a fact |

This is the shape ext4 gives its superblock and ZFS its uberblock: an outer
record that never changes, so that it can describe an inner one that does.

**The freeze costs nothing today, because it names what is already true.**
`meta_key` joins once and stores the result whole, and the reason is already
written down beside it - *this file cannot be laid out by a rule that has to be
read out of it*. redb and sqlite hold their bookkeeping in a table keyed by
`&str`, which is the same flatness by another route.

**It is addressing that is frozen, not content.** `StoredShape` already carries
a `children` tree, and that nesting is inside a serialized value rather than in
the key space, so no reader has to know `path.sep` to walk it. Everything the
meta says about the application's data can grow this way - `Role` settling into
`StoredShape` is a new field with `#[serde(default)]`, not a break.

**What the freeze does forbid** is a nested meta key space, another separator,
another escape, and another encoding for a record. Those four are permanent, and
they are permanent deliberately: they are the part we control completely.

---

## 2. What is recorded

### 2.1 A set of facts about how the bytes were written

Not a version. What is unrecorded today is not one thing that moves together; it
is a handful of independent settings on separate schedules, and a single number
cannot say which of them moved.

| fact | value today | a silent change means |
| --- | --- | --- |
| `codec` | `msgpack` (redb), `sonic-json` (sqlite), the document's own (text) | nothing reads |
| `codec.struct` | `map` | structs read as arrays |
| `codec.bytes` | `bin` | a byte vector will not read as a sequence |
| `path.sep` | `.` | every key renamed |
| `path.escape` | `\` | every key renamed |
| `layout` | `nested`, for the data | the document is walked by the wrong rule |

The floor an engine has to meet is not on this list. It is a fact by the
definition in §5 - it changes who can open the file - but it is not in the set,
because the set cannot be read early enough to be of use: the failure it warns
about is the one sqlite reports as a corrupt schema, from behind the door the
record sits inside. It lives in sqlite's `user_version` instead, and a copy of
it here would be the second record of one truth §5 forbids.

The initialization marker was on this list, at `__init::` on the flat engines
and `__init.` on the text ones, and a silent change to it loses the seeding
markers and lands defaults on top of the user's data. It is not on it any more,
because the difference was in the spelling and not in anything a reader needs
told: the kind is `init` everywhere now, and how a kind is joined to a path is
the meta addressing §1 froze. A fact every writer emits at one value is not a
fact - §7, promotion, applied before the set was ever written.

The join still differs - `::` on the flat engines, a `StorePath` join on the
text ones - and nothing depends on it either way: these keys are built and
looked up whole, never split back into a kind and a path. Worth keeping as it
is for what it would buy if something ever did split them, since a namespace
with a dot in it makes `init.ui.panels` three readings and `init::ui.panels`
one.

Three of the six already take different values in one tree. This is not
provision for the future; it is the present, undescribed.

`layout` describes the data only. The meta's own layout is not here, because §1
froze it - a fact that has to be read to find out how to read the record it sits
in is not a fact.

**Every fact is written from the first release that writes facts at all**, at its
current value, even where nothing varies yet. A fact that first appears on the
day it changes is useless for every file written before that day: its absence
would have to be read as "the default for this engine", and that rule is itself
an unrecorded fact.

**The set carries its own presence marker.** Without it, "this engine does not
use `codec.struct`" is indistinguishable from "this file predates facts".

**Sub-facts are scoped by their parent's value.** `codec.struct` and
`codec.bytes` mean something only when `codec` is `msgpack`.

### 2.2 Where the set lives

As an ordinary meta record, on all three engines, addressed by the frozen rule
of §1. No engine header, no `application_id`, no `user_version`, no slot inside
redb's transaction block.

That answers the difficulty §5 raises about uneven ground. The ground is
only uneven if the set has to sit in what each engine reserves for itself. Every
engine here already has a meta layer of our own making, and all three are flat
and keyed by strings.

The dotted names above are keys *inside* the record, not paths in the store. A
fact called `path.sep` addressed as a store path would have to be found using
the separator it describes; as a field of one serialized value it is just a
name, and the circle does not close.

**One fact is not in the set, and is the exception to "no engine header": the
floor.** A fact only works if it can be read before the thing it warns about
goes wrong, and this one cannot be: our record sits inside the database whose
schema is exactly what an old sqlite fails to parse, so the warning would be
behind the door it was meant to keep shut.

It goes to `PRAGMA user_version` instead, which is in the header and readable
before any schema is. It holds the number sqlite gives its own versions -
`major * 1000000 + minor * 1000 + patch`, the same shape
`sqlite3_libversion_number()` returns - so the comparison is one integer against
another with nothing parsed. §7 calls the move compatibility by construction:
put the thing where a reader that knows nothing can still reach it.

Only sqlite has a floor to record. redb refuses on its own format byte and says
so, and the text engines have no such version to be too old. So this is one
number in one header rather than a fact with three values, and duplicating it
into the set would buy nothing and leave two copies to drift.

**A `user_version` is known to be ours because the header says so.**
`application_id` holds ASCII `AMES`. A file claiming someone else's id is
refused by that name; a file claiming none is claimed on open. §8 warns against
reserving names without a reason, and reading a floor out of a stranger's file
was the reason.

### 2.3 How the meta records evolve

Not by a version pair. §8 argues that a number is as forgettable as a fact, and
a current/compatible pair for the meta layer would be exactly such a number,
introduced by the same document that distrusts them.

What replaces it is a discipline that serde already half-enforces:

- **A meta record only ever grows.** No field is removed.
- **Every field added after the first release carries `#[serde(default)]`.** An
  older record then reads on a newer build.
- **A field is never retyped or repurposed under its own name.** A new meaning
  gets a new name, because a reader cannot tell a repurposed field from a
  correct one.
- **A change that breaks any of the three is a fact in §2.1**, not a version.

The first direction holds by construction: no record sets
`deny_unknown_fields`, so a newer record reads on an older build, and
`a_record_carries_a_field_this_build_has_no_name_for` states it as a rule rather
than an accident. The second is discipline, not construction. Both fields added
since - `StoredShape::children` and `StoredShape::flattened` - carry
`#[serde(default)]`, and the rule is written where the records are declared;
nothing but that stops the next field from arriving required.

### 2.4 The names, which §8 makes permanent

Settled before anything wrote a fact, because a written name is an obligation to
interpret it forever.

- **The record is `format`**, one per store, at the root - the first record with
  no prefix under it, where the other kinds - `meta`, `schema`, `log`, `init` -
  are per-prefix.
- **Its presence is the marker.** A store carrying no set predates facts.
- **A value is a string**, so that it reads in a document a person opens.
- **The six names** are `codec`, `codec.struct`, `codec.bytes`, `path.sep`,
  `path.escape` and `layout`. Three of them already take different values in one
  tree.
- **`application_id` is claimed**, so a `user_version` we read is known to be
  ours. See §2.2.

---

## 3. Reading

A build meeting a store does exactly one of these, and never anything else:

1. **Opens.** Every fact in a deciding namespace is one it knows, with a value
   it knows.
2. **Opens, ignoring what it does not know.** Facts outside the deciding
   namespaces, unknown tables, unknown keys - all left alone.
3. **Refuses, naming the fact.** An unknown name in `codec.*`, `path.*` or
   `layout`, *or* a known name with an unknown value. The second case is listed
   separately because it is the one that gets forgotten.

**No build refuses because a number is higher.** No number does that job.

**Direction is derivable without an ordering.** An unknown fact means the file
was written by something newer; a set missing facts this build knows how to
write means it is older.

## 4. Writing

- **What a build does not understand survives its writes.** Unknown facts,
  unknown keys, unknown tables. This already holds by construction - the text
  engines serialise the whole loaded document, redb and sqlite do not touch rows
  they did not write - and a test makes it a contract rather than an accident.
- **A build never removes what it does not understand.** Early proto3 dropped
  unknown fields and 3.5.0 reversed it: dropping is silent loss through any
  round trip.
- **A downgrade never rewrites data into an older encoding.** An older build
  reads, or it refuses. There is no automatic migration downwards.
- **A fact is written when the change reaches the disk, not when the code learns
  to produce it.** ZFS states this as a rule - features may not perform
  enable-time initialisation - and without it a fact recorded ahead of the bytes
  it describes is a lie.

## 5. Facts belong to whoever owns the bytes

A fact is ours only where nothing beneath us records it.

| | owner | where it is recorded |
| --- | --- | --- |
| page layout, WAL mode | SQLite | bytes 18/19 of its own header |
| redb's file format | redb | one byte inside transaction slot 0 |
| `codec.*`, `path.*`, `layout` | **us** | **nowhere** |

Putting WAL into our set would be a second record of one truth, free to drift
from the first. The same goes for redb's file format version: redb records it,
redb refuses on it, and a copy of it here could only ever disagree. Our facts
pass the test - nothing below knows that a struct was written with
`with_struct_map`.

Which engine version wrote a file is a different thing and not a deciding fact.
It decides nothing, it duplicates nothing, and it is worth having for a bug
report, so it belongs in the additive namespace of §7 rather than in `codec.*`.

**But a floor we impose on an engine is ours, and it is recorded nowhere.**
`PRAGMA journal_mode = WAL` raises bytes 18/19 to 2, so these files already do
not open in SQLite before 3.7.0. That is a compatibility decision this library
made, and it survives only because SQLite is careful on our behalf. It does not
always will be: `STRICT` tables would move the floor to 3.37 without touching
bytes 18/19 at all, because that change is one of the ones SQLite never gave a
name - and a file using it reports `malformed database schema` on an older
build. Inheriting that failure is a choice we would be making silently.

So the floor is a fact by the definition below - it changes who can open the
file - and it is not a second record of anything: sqlite's bytes 18/19 carry
*sqlite's* minimum from WAL, and ours carries the one our own decisions add up
to, including the ones sqlite never named.

It is recorded, and it is not in the set. Where the two engines differ is not
whether they leave us a slot but what they do when they refuse: redb refuses on
its own format byte and says so, and our floor coincides with its, because
nothing we enable moves compatibility past that byte. sqlite refuses on a schema
it cannot parse, with a message about corruption, which is why §2.2 sends its
floor to `user_version` - where it can be read in time.

sqlite's floor is measured and recorded: 3.7.0, where `journal_mode = WAL`
arrived, written to `user_version` on open and read back before any statement
that would parse the schema. Nobody has checked whether anything since has
raised it.

**redb and the text engines have no measured floor.** redb is pinned at `4.0`
and resolves to 4.1.0 with nothing asking it for a format version. Measuring
those two, and pinning each with a test, is work this document does not do.

**And what distinguishes a fact from a setting:** a fact changes how bytes
already written read back, or who can open the file. `synchronous = NORMAL` is
configuration - it changes durability and speed and nothing about readability.
`journal_mode = WAL` is not.

## 6. Not promised

- That any build reads any file. Only that it says which fact stopped it.
- That a store moves between engines.
- That hand edits to the meta sidecar are supported. The data file is meant to
  be edited; the sidecar is not, and today the only consequence of editing it is
  a log line.
- Anything at all, before the first release someone depends on. Until then the
  answer to a format change is "delete the store", and that is a deliberate
  position rather than an oversight.

---

## 7. Room to manoeuvre

Which moves stay available once the contract is in force.

**Promotion.** A fact that every writer emits stops being a fact and becomes
part of the base format. Vulkan does this with extensions promoted into core
versions. Without it the set only grows.

**The additive namespace is unlimited.** Anything outside `codec.*`, `path.*`
and `layout` can be added forever without cutting anyone off, because the
contract already says unknown names there are ignored and preserved.

**The bridging release.** One minor that reads both encodings and writes the new
one only when asked, then a major that drops the old reader. redb did exactly
this: 2.6 added file format v3 behind `create_with_file_format_v3` and
`Database::upgrade()`, 3.0 removed v2. The same shape as ZFS `enabled` against
`active`: "the code can" and "the disk has" are separate events, and keeping
them separate is what lets a mixed population of builds coexist.

**Compatibility by construction.** The best move, and the one that costs design
effort rather than compatibility. ext4's htree indexes are `compat` - a reader
that knows nothing about them still walks the directory correctly - because
someone disguised the index nodes as directory entries a naive walk skips. The
v2 pack index in git is detectable by an old reader because someone chose a
first word that is impossible for a v1 file. Neither needed a flag. Every change
should be asked this question before it is given a fact.

**Refuse and convert.** When the conversion needs information the file does not
carry - an undeclared nested subtree that could be either a key holding an
object or a path to a value - a migration would be guessing, and guessing in a
store is silent corruption. A converter has somewhere to ask.

That case is not hypothetical and it is not only about conversion. A document
engine cannot tell a value that is itself a map from a level with values under
it by looking at the file. Where a schema declares the path, `Role` says which
it is, and the store records that, so the answer survives the binary that
declared it. Where nothing declares it, the file is not asked: the path is
written whole, as one name, in a plane beside the declared trees - the layout
the metadata file has always used, and for the same reason, that a key written
whole needs no schema to be read back.

So the question is not answered by guessing, and it is not answered by
bookkeeping either. It stops being asked.

---

## 8. What forecloses a move

**A loosening, once granted, is permanent.** git honoured `extensions.*` at
repository format version 0 before deciding it should not have, and the fix
could only apply to new extensions: *"for compatibility reasons, we are stuck
with that decision."* Four extensions still live in a mode where the protection
is illusory. This is why the class of a fact is carried by its namespace rather
than declared per fact - a class that is declared can be declared wrongly, and
wrongly once is wrongly forever.

**Absence already means something.** A fact that has been written cannot be
unwritten: its disappearance reads as "an old file". The set's presence marker
is what keeps this from being fatal, but it does not make a fact removable.
Reserve names deliberately, not liberally - each is an obligation to interpret
it forever.

**A change you did not record is a change you cannot detect later.** This is the
current state, generalised. A number is as forgettable as a fact - Room added an
identity hash precisely because developers forget to bump the version. Catching
it is a testing problem and is settled there, not here: fixtures of each old
format with a test that opens them, and a debug-mode check that the bytes a
build produces agree with the facts it recorded. Vulkan's answer to the same
problem is a validation layer.

**The clock starts at the first release someone depends on.** The absence of the
whole set has exactly one honest meaning - written before the set existed - and
that works once. Cargo is the worked example: cargo before 1.47 ignored the
top-level `version` in `Cargo.lock` entirely, so the marker added when it was
needed did not protect the versions it was meant to protect. **The set has to
land before the break, not with it** - but "the break" means a break someone's
files live through, and today there are none. Until facts ship, deleting the
store is still the answer, and breaking is free.

**Freezing the format is a real option with a real price.** SQLite has not
changed its file format since 2004 and got a universal interchange format for
it. The price is that every new capability has to fit in a field that already
exists, and what does not fit is diagnosed by a parser error that lies about the
cause: a perfectly intact database reports `malformed database schema`. **The
quality of an error is a function of whether the change was given a name.**

§1 takes this price deliberately, and only for the meta's addressing, where the
surface is four constants we control and the alternative is a set of facts that
cannot be read without itself.

---

## 9. Deferred, with the cost counted

**Read-only compatibility.** The third outcome between "works" and "refuses" -
an older build that can still read the user's settings, so the application
starts. Two objections: a read-only flag does not restrain a person with a text
editor, which is the text engines' entire purpose; and `Store` has no read-only
mode at all, since writes go through the debouncer, so an honest one needs a new
contract on `set` and `delete`. The first stands. The second is a cost, not an
impossibility, and rollback is where it would pay. Worth taking if rollback
turns out to be common.

**A format number.** Only ever as a name for a set of facts - so a bug report
and a fixture have something to call it. Never as the thing a decision compares.
