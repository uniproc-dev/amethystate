# What a text store loses when a write is interrupted

**Status: five of seven still stand.** Ways to lose committed data on the text
engines, each pinned by a test in
`crates/main/amethystate/tests/torn_recovery.rs`. The pins for what is not fixed
carry an `#[ignore]` naming this file, so the failure is a finding rather than a
red tree.

| engine | failing |
| --- | --- |
| json | 6 of 9 |
| ron | 6 of 9 |
| toml | 6 of 9 |

Two of the nine are pins rather than bugs for json and ron - those parsers
refuse every truncation - and bugs for toml only.

Ordered by what it costs. Committed data lost silently comes before an error
message that could be better.

Closed since: a missing data file with a copy beside it is recovered from the
copy rather than read as a new store (`StoreFile::lost_its_file`), and a flush
that reports failure no longer leaves the data file changed - the metadata is
written first and judged first, so the two files cannot come apart with the
error pointing the wrong way.

## 1. A torn TOML write parses as valid and wrong

Cutting `[torn]\na = 11\nb = 22\nc = 33\n` at every byte offset, 9 of 28 cuts
open successfully with no backup present, and three of them return **a different
number** for a committed key:

```
(12, "(Some(1), None, None)")            11 came back as 1
(19, "(Some(11), Some(2), None)")        22 came back as 2
(26, "(Some(11), Some(22), Some(3))")    33 came back as 3
```

The rest drop keys. Nothing looks broken, so nothing is recovered, and the
store's own closing flush writes the stump back - the loss is committed.

This is the worst shape a failure can take here: not an absent value, which a
caller can notice, but a plausible one that is not what was written.

json and ron refuse at every offset, so this is TOML's grammar rather than the
library's write path - a truncated table header or key line stays a legal
document more often than truncated JSON or RON does.

## 2. A torn write that still parses eats the backup that would repair it

The same file cut after the first key, with a complete `.bak` beside it. The
stump parses, so `create_backup` copies the stump over the good backup and
`clean_backups` deletes it. The data file ends as `"[torn]\na = 11\n"` and there
is nothing left to recover from.

This sharpens the recorded "the text engines eat their own backup". The fix that
moved the backup to **after** the read was built for a file that fails to parse.
A torn TOML file never fails, so it walks the success path and takes the backup
with it.

## 3. A backup is trusted because it exists

Both copies are taken once both files have read, so an open that is refused
leaves nothing of its own: that half is closed, and the guarantee is stated as
"an operation that did not happen leaves nothing behind" rather than as a rule
about backups.

What stands is what a leftover copy is worth when one is there anyway - left by
a crash, by a backup tool, by a person. The chain is mechanical, and every step
is something a text store exists to permit:

1. Some earlier run leaves `.bak` holding `1/2/3`.
2. Somebody edits the file by hand to `11/22/33`.
3. The next write is cut off.
4. The open recovers onto the leftover backup and returns `1/2/3`, reporting
   success through a `tracing::warn` and nothing else.

**`recover_from_backup` compares nothing** before copying the backup over the
data - not a marker, not content. mtime is not the answer either: it is a cache
hint everywhere it is used carefully, and git carries a whole "racily clean"
mechanism precisely because it will not trust one. What a leftover needs is to
say what it is, and be skipped when it cannot.

## 4. A killed write leaks a full copy of the document, forever

A child process holds its own data file open so the replace enters its retry
loop, then aborts. The temporary was written and `sync_all`ed before the loop,
so it is on disk at the kill. The parent finds `.tmp9yE715` beside
`settings.json` and `settings.meta`.

Nothing collects it. No open sweeps `.tmp*`, so one accumulates per crash, each
a complete copy of the store's contents - which for a settings file is also a
copy of whatever was in it.

`atomic_write.rs` pins the success path only. "Nothing accumulates beside the
store" is false the moment a write is interrupted.

Windows, all three engines.

## 5. One buffered write erases what another store committed

Store A holds one unflushed write. Store B opens the same path, writes
`d = 444`, flushes, drops. A's `save_now()` returns `Ok` and rewrites the whole
document from memory - `d` is gone, and nobody deleted it.

`pull_external_changes` refuses to pull while `writes != persisted`, so a single
pending write blinds A to everything committed in between.

This sharpens the recorded "an unrelated pending write rolls back a concurrent
external edit". The other writer here is not a person with an editor but a
second `Store` on the same file, whose write was flushed and acknowledged. `d`
has one determined writer and A never touched that key, so the loss is not a
race anyone could call ambiguous.

## Not reproduced

- A kill landing precisely between the two renames of one `persist`. The order
  is now metadata first and data second, and the metadata is what judges the
  data on the way back in, so a kill in the gap reads as a file that holds less
  than the last save said - which is the case
  `read_or_recover_unless` already answers.
- Data recovered from a backup that predates a schema change while the meta
  describes the new one. The file states are constructible; making the damage
  observable needs a failing migration, which pulls in the `#[migrate]`
  machinery. A suspicion, not a finding.

## What these have in common

Three of the five are one missing idea: **nothing compares the two copies before
acting on them.** A backup is authoritative because it is present (3), a stump
is a document because it parses (1, 2). The write path is careful about ordering
and has nothing to say about content.

The other two are the absence of a sweep for what a crash leaves (4), and a read
gate that treats "I have unflushed work" as "nobody else can have written" (5).
