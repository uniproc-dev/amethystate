# The on-disk format contract

**Status: design. None of this is implemented.** Today a store carries no record
of how it was written, so a build that changes an encoding reads old bytes and
cannot tell they are old. This document fixes what the promise will be, and -
more importantly - which moves stay available afterwards.

This is about the *library's own* representation: the key encoding, the
bookkeeping records, the value encoding, the document layout. The application's
schema is a separate mechanism with its own versions and its own migrations, and
the two must not become one.

---

## 1. What is recorded

Two things, and they answer different questions.

### 1.1 A set of facts about how the bytes were written

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
| `layout` | `nested` for data, `flat` for the meta sidecar | the document is walked by the wrong rule |
| `init.marker` | `__init::` on flat engines, `__init.` on text | seeding markers lost, and defaults land on top of the user's data |

Three of the seven already take different values in one tree. This is not
provision for the future; it is the present, undescribed.

**Every fact is written from the first release that writes facts at all**, at its
current value, even where nothing varies yet. A fact that first appears on the
day it changes is useless for every file written before that day: its absence
would have to be read as "the default for this engine", and that rule is itself
an unrecorded fact.

**The set carries its own presence marker.** Without it, "this engine does not
use `codec.struct`" is indistinguishable from "this file predates facts".

**Sub-facts are scoped by their parent's value.** `codec.struct` and
`codec.bytes` mean something only when `codec` is `msgpack`.

### 1.2 A current/compatible pair for the shape of the bookkeeping records

`PrefixMeta`, `SchemaSnapshot`, `Vec<AppliedStep>`. These evolve the way an
application schema evolves - almost always by adding a field - and for that a
number plus a floor is a better fit than a set of facts, and much harder to get
wrong.

One pair for the meta layer, not one per record. The three are read by
substantially the same code; split them only if they turn out to move on
different schedules.

---

## 2. Reading

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

## 3. Writing

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

## 3.5 Facts belong to whoever owns the bytes

A fact is ours only where nothing beneath us records it.

| | owner | where it is recorded |
| --- | --- | --- |
| page layout, WAL mode | SQLite | bytes 18/19 of its own header |
| redb's file format | redb | one byte inside transaction slot 0 |
| `codec.*`, `path.*`, `layout` | **us** | **nowhere** |

Putting WAL into our set would be a second record of one truth, free to drift
from the first. Our facts pass the test: nothing below knows that a struct was
written with `with_struct_map`.

The ground beneath is not level. SQLite records its own facts *and* leaves us
two free slots (`application_id`, `user_version`). redb records its format
version but leaves no room in a header that is fully spoken for. The text
engines record nothing about anything. The same set has to sit on all three.

**But a floor we impose on an engine is ours, and it is recorded nowhere.**
`PRAGMA journal_mode = WAL` raises bytes 18/19 to 2, so these files already do
not open in SQLite before 3.7.0. That is a compatibility decision this library
made, and it survives only because SQLite is careful on our behalf. It does not
always will be: `STRICT` tables would move the floor to 3.37 without touching
bytes 18/19 at all, because that change is one of the ones SQLite never gave a
name - and a file using it reports `malformed database schema` on an older
build. Inheriting that failure is a choice we would be making silently.

So each backend states the oldest version of its dependency that can still open
what we write, in this document, with a test. It is the same current/compatible
pair as §1.2, pointed at what we depend on rather than at what we write.

**And what distinguishes a fact from a setting:** a fact changes how bytes
already written read back, or who can open the file. `synchronous = NORMAL` is
configuration - it changes durability and speed and nothing about readability.
`journal_mode = WAL` is not.

## 4. Not promised

- That any build reads any file. Only that it says which fact stopped it.
- That a store moves between engines.
- That hand edits to the meta sidecar are supported. The data file is meant to
  be edited; the sidecar is not, and today the only consequence of editing it is
  a log line.
- Anything at all, before the first release someone depends on. Until then the
  answer to a format change is "delete the store", and that is a deliberate
  position rather than an oversight.

---

## 5. Room to manoeuvre

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

---

## 6. What forecloses a move

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
identity hash precisely because developers forget to bump the version. What
catches it is byte fixtures of each old format in the repository with a test
that opens them, and, more cheaply, a debug-mode check that the bytes a build
produces agree with the facts it recorded. Vulkan's answer to the same problem
is a validation layer.

**Shipping the marker with the break spends the one free move.** The absence of
the whole set has exactly one honest meaning: written before the set existed.
That works once. Cargo is the worked example - cargo before 1.47 ignored the
top-level `version` in `Cargo.lock` entirely, so the marker added when it was
needed did not protect the versions it was meant to protect. **The set has to
land before the break, not with it.**

**Freezing the format is a real option with a real price.** SQLite has not
changed its file format since 2004 and got a universal interchange format for
it. The price is that every new capability has to fit in a field that already
exists, and what does not fit is diagnosed by a parser error that lies about the
cause: a perfectly intact database reports `malformed database schema`. **The
quality of an error is a function of whether the change was given a name.**

---

## 7. Deferred, with the cost counted

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
