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

On startup, `amethystate` reads the version stored in the database for each prefix and compares it to the version in code. If they differ, the migrator looks for a registered migration step that bridges the gap and runs it.

If no step is registered for the gap, startup fails. If the database version is higher than the code version, startup is blocked to prevent data corruption.

## Automatic collection

The simplest setup collects all migration steps defined via `#[migrate]` automatically:

```rust
let store = StoreBuilder::new("./app.redb")
.collect_migrations()
.build()?;
```

No further configuration is needed if all your steps are defined with the macro.

For cases where codegen isn't enough — cross-node logic, data backfills, external data sources — see [Manual Migrations](./manual).

## What happens on a clean install

On first run there is no stored version — the database is empty. `amethystate` initializes all fields with their `#[amestate(default = ...)]` values and writes the current version. No migration steps run.

## Schema drift

If you change a field's type, rename it, or add or remove fields without bumping the version, no migration runs — but `amethystate` still notices. On every startup the stored schema hash is compared against the current code, and any mismatch is logged as a warning:

```
⚠️  Schema drift detected in prefix 'app'
  + field 'timeout': Duration
  - field 'host' (exists in DB, missing in code)
  ~ field 'port': u16 -> u32
  Suggestion: increment version and write a migration if these changes are intentional.
```

| Symbol | Meaning |
|--------|---------|
| `+` | field exists in code but is absent from the database |
| `-` | field exists in the database but was removed from code |
| `~` | field exists in both, but its type changed |

Drift does not block startup — it is a warning, not an error. If the changes are intentional, increment the version and write a migration step. If not, you may have accidentally renamed or dropped a field.

The full diff is available programmatically via `MigrationReport`:

```rust
let (store, report) = StoreBuilder::new("./app.redb")
    .collect_migrations()
    .build()?;

if report.has_drift() {
    for component in &report.components {
        // component.nagging contains the per-prefix diff
    }
}
```