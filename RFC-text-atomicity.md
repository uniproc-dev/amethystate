# What a text store loses when a write is interrupted

**Status: found, not fixed.** Seven ways to lose committed data on the text
engines, each pinned by a test that fails on the current code.
`crates/main/amethystate/tests/torn_recovery.rs` holds all nine tests and the
whole file fails under `--features json,toml,ron`. That is deliberate: they are
pins for bugs, not regressions, and the tree stays red there until the bugs go.

Two of the nine are pins rather than bugs for json and ron - those parsers
refuse every truncation - and bugs for toml only.

| engine | failing |
| --- | --- |
| json | 7 of 9 |
| ron | 7 of 9 |
| toml | 9 of 9 |

Ordered by what it costs. Committed data lost silently comes before an error
message that could be better.

## 1. A missing data file starts fresh and deletes the backup beside it

Write the store, copy the document into `.bak`, delete the data file, reopen.
Committed `11/22/33` comes back as `(None, None, None)`, and the backup is gone
afterwards. Both copies destroyed in one open.

```rust
let good = std::fs::read_to_string(&files.data).unwrap();
std::fs::write(&files.data_backup, &good).unwrap();
std::fs::remove_file(&files.data).unwrap();
```

**Why.** `StoreFiles::load_and_back_up` branches on the read:

```rust
match self.load_or_empty() {
    Ok(doc) => { self.create_backup()?; Ok(doc) }
    Err(unreadable) => match self.recover_from_backup() { .. }
}
```

`load_or_empty` answers a missing path with `Ok(D::empty())`, so the read never
fails and the recovery arm is unreachable. `create_backup` then has nothing to
copy and skips, the empty document is persisted over the data file, and
`clean_backups()` removes the `.bak`.

`Store/files.md` promises recovery for a half-written file. An absent one is
read as a brand-new store - the one case where a backup exists and cannot be
reached.

## 2. A torn TOML write parses as valid and wrong

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

## 3. A torn write that still parses eats the backup that would repair it

The same file cut after the first key, with a complete `.bak` beside it. The
stump parses, so `create_backup` copies the stump over the good backup and
`clean_backups` deletes it. The data file ends as `"[torn]\na = 11\n"` and there
is nothing left to recover from.

This sharpens the recorded "the text engines eat their own backup". The fix that
moved the backup to **after** the read was built for a file that fails to parse.
A torn TOML file never fails, so it walks the success path and takes the backup
with it.

## 4. A backup a refused open left behind becomes the truth later

`load_and_back_up` backs the data up first and reads the meta second.
Unreadable bookkeeping with no `.meta.bak` aborts the open **after** the data
backup was written, and nothing cleans up on that path. The directory is left as
`settings.json`, `settings.json.bak`, `settings.meta`.

The chain from there is mechanical, and every step is something a text store
exists to permit:

1. A refused open leaves `.bak` holding `1/2/3`.
2. Somebody edits the file by hand to `11/22/33`.
3. The meta is repaired, so the store opens again.
4. The next write is cut off.
5. The open recovers onto the leftover backup and returns `1/2/3`, reporting
   success through a `tracing::warn` and nothing else.

**`recover_from_backup` compares nothing** before copying the backup over the
data - not mtime, not a marker, not content. A backup is trusted because it
exists.

## 5. A flush that returned `Err` has already committed the data file

Hold `settings.meta` open with `FILE_SHARE_READ` so the second rename fails.
`save_now()` returns `Err`, and the data file on disk has already changed from
`"a": 11` to `"a": 44`.

The caller is told nothing landed. Half of it did.

This sharpens "the text engines replace two files with no barrier between them":
no crash is needed. An ordinary recoverable I/O failure on the bookkeeping file
- which a save rewrites in full even when it has not changed - produces the
split state, and the error return actively misinforms about it.

Windows, all three engines.

## 6. A killed write leaks a full copy of the document, forever

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

## 7. One buffered write erases what another store committed

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

- A kill landing precisely between the two renames of one `persist`. Not
  attempted: finding 5 produces the same divergence deterministically and with a
  wrong return value on top.
- Data recovered from a backup that predates a schema change while the meta
  describes the new one. The file states are constructible; making the damage
  observable needs a failing migration, which pulls in the `#[migrate]`
  machinery. A suspicion, not a finding.

## What these have in common

Four of the seven are one missing idea: **nothing compares the two copies, or
the two files, before acting on them.** A backup is authoritative because it is
present (4), a stump is a document because it parses (2, 3), a data file that is
gone is a store that is new (1). The write path is careful about ordering and
has nothing to say about content.

The other three are the seam between the two files (5), the absence of a sweep
for what a crash leaves (6), and a read gate that treats "I have unflushed work"
as "nobody else can have written" (7).
