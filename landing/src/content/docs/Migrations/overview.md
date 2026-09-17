---
title: Migrations
sidebar:
  order: 20
---

## What migrations are for

State structs evolve. Fields get renamed, types change, new fields are added. Without a migration layer, any structural change either silently produces wrong data or requires you to handle versioning manually.

`amethystate` tracks schema versions explicitly and runs the appropriate transformation steps on startup, before any application code runs.

## What migrates and what doesn't

The migrator works exclusively with persistent data.

| Field kind | Migrates? |
|------------|-----------|
| Regular fields | ✓ |
| `nested` structs | ✓ |
| `volatile` fields | ✗ — in-memory only, no stored data |

## How versioning works

Every `#[amethystate]` struct has a `version` attribute that defaults to `0`:

```rust
#[amethystate(prefix = "app", version = 2)]
pub struct AppConfig { ... }
```

On startup, `amethystate` reads the version stored in the database for each line - the prefix, and the struct's `id` where it has one - and compares it to the version in code. If they differ, the migrator looks for a registered migration step that bridges the gap and runs it.

If no step is registered for the gap, startup fails. If the database version is higher than the code version, startup is blocked to prevent data corruption.

## Automatic collection

`migrate` collects every step declared with `#[migrate]` and hands back what the pass did:

```rust
let (store, report) = StoreBuilder::new("./app.redb").migrate()?;
```

No further configuration is needed if all your steps are defined with the macro. `build` runs no step at all: it is the open for a store that has nothing to migrate, and a binary full of `#[migrate]` opened that way migrates nothing.

For cases where codegen isn't enough — cross-node logic, data backfills, external data sources — see [Manual Migrations](/amethystate/migrations/manual/).

## What happens on a clean install

On first run there is no stored version — the database is empty. `amethystate` initializes all fields with their `#[amestate(default = ...)]` values and writes the current version. No migration steps run.

## When the struct changed and the version did not

A rename, a field moved under a different level, a value that became a map — with the version left alone — asks for no step, so none runs. It is noticed anyway, reported on every startup until somebody answers it, and does not block the open. [Schema drift](/amethystate/migrations/drift/) is that whole subject.