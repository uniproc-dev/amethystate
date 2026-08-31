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
match StoreBackend::files(&store) {
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
        println!("kept while rewriting: {}, {}",
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
