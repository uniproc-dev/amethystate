---
title: What is on disk
sidebar:
  order: 15
---

How many files a store keeps depends on the engine. Rebuilding their names from
the one you passed in means writing down a rule the engine owns, so the store
says it instead:

<!-- shown: asking a store where its files are -->
```rust
match StoreBackend::files_layout(&store) {
    Some(StoreLayout::Single { data }) => {
        println!("everything is in {}", data.display());
    }
    Some(StoreLayout::Sidecars {
        data,
        meta,
        data_backup,
        meta_backup,
    }) => {
        println!("values:      {}", data.display());
        println!("bookkeeping: {}", meta.display());
        println!(
            "kept while rewriting: {}, {}",
            data_backup.display(),
            meta_backup.display(),
        );
    }
    None => println!("this engine does not say"),
}
```
<!-- /shown -->

The answer is a shape rather than a list, so reaching a particular file is a
match and never a search. An engine with no separate bookkeeping cannot be asked
for it, and one that has it cannot be missing it.

A file is named whether or not it exists at that moment. A backup is there only
while a rewrite is in flight, and its name is worth having either way.

## redb and SQLite: one file

Both keep the values and the bookkeeping inside a single file, in structures of
their own. Nothing is written beside it.

## The text engines: two files, and two more while writing

| file | holds |
| --- | --- |
| `settings.json` | the values |
| `settings.meta` | the schema bookkeeping |
| `settings.json.bak` | a copy of the data, while it is being rewritten |
| `settings.meta.bak` | the same for the bookkeeping |

The sidecar carries what the store needs in order to read the data back: which
version of each struct wrote it, what those fields looked like, and what the
migration pass has already done. A person can read the data file on its own; a
store opening it without the sidecar has lost the schema it was written under.
Both belong to the store, and a backup takes both.

`.bak` is appended to the whole name rather than replacing the extension.
Swapping it would give `settings.bak` for both files, so the second copy would
land on the first and the data would have no backup left - and it would also
name a file the store never created, a `settings.bak` somebody put there
themselves.

## What the backup is for

It is a guard on the open, not a copy kept for you. Opening the store reads the
data, backs up what it could read, and runs the migration pass; a pass that
succeeds deletes the backup, and one that fails puts it back. In a store that
started normally there is no `.bak` at all.

Which means a `.bak` sitting there is a previous open that never finished. The
state one leaves behind, made by hand:

<!-- shown: a previous open that never finished -->
```rust
std::fs::write(&backup, &good).unwrap();
std::fs::write(&data, "{ this never finished").unwrap();
```
<!-- /shown -->

A good backup beside a half-written data file. On the next open the data will
not parse, the backup will, and the store recovers from it and carries on -
saying so through `tracing::warn!` under the `amethystate` target.

The order matters and is the whole point. The backup is taken **after** the
read rather than before it: a copy exists to hold a readable file, so copying a
half-written one over it destroys the only intact copy in exactly the case the
backup is kept for.

The copy is taken immediately before the migration pass runs, once everything
else that could still refuse the open has gone by. An open that is refused is an
operation that did not happen, and it leaves nothing of its own: a `.bak` beside
the store is read by the next open as an unfinished previous run.

## What the copy cannot promise

It answers one question - *the migration did not finish, put the file back* -
and it is asked others it has no way to answer.

**A file cut short still parses.** A write that stops after the first key
leaves a document with one key in it, and that is a document: it opens, it
reads, and the keys that were committed and are now missing look exactly like
keys nobody ever wrote. Nothing inside the file says how long it was meant to
be.

**Whether to recover is decided by whether the file parses.** So the stump is
taken as the store, and the copy that would have repaired it is deleted at the
end of that open without being read.

**The copy carries no age the store can trust.** A `.bak` is recovered from
when the data will not read, whether it holds the write before this one or a
hundred before it. A modification time is not an answer - it moves when
somebody copies the directory.

All three are one fact from three sides: from outside, a document is whatever
it parses as. Telling a whole file from a stump would mean the store writing
its own length or checksum into it, and then it is no longer a file a person
can edit, which is the reason the text engines exist at all.

What that leaves for an application: a `.bak` is a fault report, not a restore
point. It says an open did not finish. Where the data matters, keep copies of
your own - the store's copy belongs to the migration and cannot be borrowed as
a backup.

An engine that owns its file outright has none of this: redb and SQLite commit
through their own write-ahead logs, and a write that is cut off is either there
whole or not there at all.
