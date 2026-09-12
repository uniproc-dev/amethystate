---
title: Installation
sidebar:
  order: 2
---

```toml
[dependencies]
amethystate = "0.20"
```

That gives you the default engine, redb. Everything else on this page is about
choosing a different one.

## Choosing an engine

Five engines can hold the store, and exactly one of them opens the file at run
time. Which one is a compile-time choice, made by Cargo features.

| feature | engine | file |
| --- | --- | --- |
| `redb` | redb | `.redb` |
| `sqlite` | SQLite | `.db` |
| `json` | JSON | `.json` |
| `toml` | TOML | `.toml` |
| `ron` | RON | `.ron` |

**The text engines write two files**: the data, and a `.meta` sidecar. The
sidecar carries what the store needs in order to read the data back - which
version of each struct wrote it, what those fields looked like, and what the
migration pass has already done. A person can still read the data file on its
own; the store opening it without the sidecar has lost the schema it was
written under, so both belong to the store and a backup takes both. redb and
SQLite keep the same record inside their single file.

The format sets what the store can express.
[Choosing an engine](/amethystate/choosing/absent-or-null/) measures what each
engine does with the same values.

### When it should not be redb

Engine features are additive, and when more than one is compiled in the store
opens with the first of **redb, SQLite, JSON, TOML, RON**. So adding `json`
alone leaves redb in charge and nothing says otherwise - you have to turn the
default off:

```toml
amethystate = { version = "0.20", default-features = false, features = ["json"] }
```

Compiling several in at once is legitimate - a tool that reads whichever file
it is pointed at, or a test suite that runs the same case over each. Name the
engine explicitly when the store is built and that order stops mattering.

### SQLite

SQLite is built from source, so you need a C toolchain. In exchange the minimum
SQLite version is this library's choice rather than whatever the user's
distribution ships.

That is not cosmetic. Start using some SQLite feature — `STRICT` tables, say —
and the minimum jumps to 3.37. Nothing in the file records that it did, so an
older SQLite reports a corrupt schema rather than a version it cannot read,
about a file that is perfectly intact.

```toml
amethystate = { version = "0.20", default-features = false, features = ["sqlite"] }
```

## Tauri

Tauri integration includes a plugin, async backend, and Rust and TypeScript bindings generator. Enable it with the `tauri` feature:

```toml
amethystate = { version = "0.20", features = ["tauri"] }
```

See [Tauri integration](/amethystate/integrations/tauri/) for setup and usage.

## Migrating from an existing solution

See [Migrating from a custom solution](/amethystate/migrations/custom/).

## Framework integrations

See [Integrations](/amethystate/integrations/overview/).
