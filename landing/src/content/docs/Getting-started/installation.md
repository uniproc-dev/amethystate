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

| feature | engine | file | what it is for |
| --- | --- | --- | --- |
| `redb` | redb | `.redb` | the default: a fast embedded database |
| `sqlite` | SQLite | `.db` | when something else has to read the same file |
| `json` | JSON | `.json` | a file a person can open and edit |
| `toml` | TOML | `.toml` | the same, in the format config files usually take |
| `ron` | RON | `.ron` | the same, in a format that spells Rust types out |

**The text engines write two files**: the data, and a `.meta` sidecar. The
sidecar carries what the store needs in order to read the data back - which
version of each struct wrote it, what those fields looked like, and what the
migration pass has already done. A person can still read the data file on its
own; the store opening it without the sidecar has lost the schema it was
written under, so both belong to the store and a backup takes both. redb and
SQLite keep the same record inside their single file.

The format sets what the store can express.
[Limitations](/amethystate/limitations/absent-or-null/) measures what each
engine does with the same values.

### Turning the default off

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

SQLite is compiled in from source, so building it needs a C toolchain. That
keeps the SQLite version this library's choice: a system SQLite would be
whatever the user's distribution ships, and a feature adopted later can raise
the floor by years without SQLite recording that it did.

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
