---
title: Introduction
sidebar:
  order: 1
---

`amethystate` is a persistent reactive state library for Rust GUI applications.

You declare what the application's state is, and the library keeps one copy of
it - on disk, in memory, and in front of whoever is watching. There is no save
layer to write and nothing to reconcile.

## The moving parts

### How it is stored

Everything in a store lives at a path: dotted levels that nest the way a
directory does. A struct does not become a table or a file of its own - its
`prefix` says where it sits, and each field is a path under that prefix. So
nothing has an id or a schema to register: the path is the identity, and it is
decided where the struct is declared.

What that becomes on disk is the engine's business, and there are five of them
to choose from - see [Installation](/amethystate/getting-started/installation/).
redb and SQLite keep one file; the text engines keep the data in one file and
the schema bookkeeping in another beside it.

### How it is saved

Writing a field updates memory at once, so the next read sees it and everything
watching it hears about it. The disk write is debounced: a burst of writes
settles into one flush. A write carrying the value already stored is dropped,
so a slider that rounds to the same step, or a form that fires on blur without
an edit, costs nothing.

The store flushes what it holds when it closes, so a graceful exit loses
nothing. A write that cannot wait for the debounce can ask for the disk
directly - see [Durability](/amethystate/concepts/durability/).

### Change travels

A write notifies whatever subscribed to that field. Collections whose keys
are not known at compile time store each entry as its own path, so a change is
observable per entry as well as for the whole.

Two shapes of state are available, and they differ in this part alone:

- **Reactive** - fields are handles you read, write and subscribe to. This is
  the mode the rest of the book assumes.
- **Persistent-only** - fields are plain Rust values on a plain struct, saved
  when you say so. For frameworks that own their update loop, and for state
  nobody needs to watch. It does not observe changes made elsewhere.

### The schema is on disk too

Beside the data the store keeps what it knows about the shape of that data:
which version of each struct wrote it, and what those fields looked like.
Opening the store compares that record against the code that is running.

A struct whose version went up is migrated by the steps you declared. A struct
whose fields changed *without* a version bump is drift: the mismatch is reported
with a diff and startup continues, because refusing to start over a renamed
field would be worse than saying so.

Which steps run depends on how the store was opened, and this is the one thing
worth reading twice.
[`StoreBuilder::build`](/amethystate/migrations/manual/) runs the migrations
declared by hand.
[`build_with_migration`](/amethystate/migrations/defining-steps/) also collects
every `#[migrate]` step in the binary, and says what the pass did. So a store
opened with `build` in a binary full of `#[migrate]` steps migrated nothing,
and said nothing about it.
