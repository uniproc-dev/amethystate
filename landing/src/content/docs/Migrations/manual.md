---
title: Manual Migrations
sidebar:
  order: 22
---

Codegen migrations cover the common case: rename fields, change types, fill in defaults. When that isn't enough — cross-node reads, data backfills, key cleanup inside a `ReactiveMap` — you can write migration steps by hand.

## Entry point

Manual steps are registered through the `.migrations()` builder method:

<!-- shown: registering steps by hand -->
```rust
let (store, report) = StoreBuilder::new(app)
    .migrations(|m| {
        m.for_prefix("net")
            .step(1, "move off the privileged port", |ctx| {
                ctx.set("port", &8080u16)
            });
    })
    .build_with_migration()?;
```
<!-- /shown -->

`build_with_migration` runs these and every step declared with `#[migrate]`, and hands back what the pass did. Plain `build` runs only the steps registered here; `m.collect_codegen()` inside `.migrations()` adds the `#[migrate]` ones to them.

Steps for one line run in version order. Nothing orders the prefixes up front: a step that reads another prefix brings that one up to date first — see [Reading across prefixes](#reading-across-prefixes).

## Defining a step

```rust
m.for_node::<Profile>()
    .step(2, "split full name", |ctx| {
        let full_name = ctx.get::<String>("full_name")?.unwrap_or_default();
        let mut parts = full_name.splitn(2, ' ');
        ctx.set("first_name", &parts.next().unwrap_or_default().to_string())?;
        ctx.set("last_name", &parts.next().unwrap_or_default().to_string())?;
        ctx.delete("full_name")?;
        Ok(())
    });
```

`for_node::<T>()` targets the struct by its prefix and its `id`. By hand, `for_prefix("net")` names a prefix's unnamed line and `for_named("ui", "panels")` the line declared with that `id`.

One version is reached by one step. A step written by hand to a version a `#[migrate]` step already reaches is refused as the pass starts, with `MigrationError::StepTwice` naming both. And a `#[migrate]` step keeps its struct where it stands: one between two prefixes, or two `id`s, is a compile error, because it would read the new place, find nothing and write the defaults. Moving what is stored is a step of its own, through `global_get` and `global_set`. `.step(version, description, closure)` registers the transformation that brings it from `version - 1` to `version`.

## The context API

Inside a step closure, `ctx` gives you low-level access to the node's stored keys. All key arguments are relative to the node's prefix unless noted.

### Basic operations

| Method | Description |
|--------|-------------|
| `ctx.get::<T>(key)` | Read a key. Returns `Result<Option<T>>`. |
| `ctx.set(key, value)` | Write a key. |
| `ctx.delete(key)` | Remove a key. |
| `ctx.rename(from, to)` | Copy a key to a new name and delete the old one. No-op if the source key is absent. |

### Compound operations

These combine read, transform, and write into a single call:

| Method | Description |
|--------|-------------|
| `ctx.transform::<TOld, TNew>(key, f)` | Read a key, apply `f`, write the result back under the same key. No-op if key is absent. |
| `ctx.merge::<T1, T2, TNew>((k1, k2), into, f)` | Read two keys, combine them with `f` into a third key, delete the originals. No-op if either source is absent. |
| `ctx.split::<TOld, T1, T2>(from, (k1, k2), f)` | Read one key, split it into two with `f`, delete the original. No-op if source is absent. |

Examples:

```rust
// multiply a value in place
ctx.transform::<u16, u16>("sample_rate", |r| Ok(r.saturating_mul(10)))?;

// combine first_name + last_name into display_name, remove originals
ctx.merge::<String, String, String>(
    ("first_name", "last_name"),
    "display_name",
    |f, l| Ok(format!("{f} {l}")),
)?;

// split "host:port" into two separate keys
ctx.split::<String, String, u16>(
    "address",
    ("host", "port"),
    |s| {
        let (h, p) = s.split_once(':').unwrap();
        Ok((h.to_string(), p.parse()?))
    },
)?;
```

### Collections

| Method | Description |
|--------|-------------|
| `ctx.scan_map::<K, V>(key)` | Scan all entries under `prefix.key.*` and return them as an `IndexMap<K, V>`, in the order the map itself walks. |

Useful when a `ReactiveMap` field is migrated without going through `AmeData` — read its entries, then write them where they belong now:

```rust
let old_routes = ctx.scan_map::<String, String>("routes")?;
ctx.delete_prefix("routes")?;

for (name, url) in &old_routes {
    ctx.set(&format!("endpoints.{name}"), url)?;
}
```

Taking the old level off is one call rather than a loop: a map owns everything under it, so `delete_prefix` is the whole of it.

Every entry has to come back. A step reads the map, changes it and writes it back, so an entry left out on the way in would be an entry deleted. One that will not read as `K` or `V` is an error, and the step's transaction rolls back.

### Global access

| Method | Description |
|--------|-------------|
| `ctx.global_get::<T>(path)` | Read any key from the store by its full path. |
| `ctx.global_set(path, value)` | Write any key in the store by its full path. |

`global_get` and `global_set` bypass the node's prefix entirely. Useful when a step needs to read from a node that has already migrated:

```rust
let plan = ctx.global_get::<String>("identity.plan")?.unwrap();
```

### Scoping

`ctx.scoped(sub_prefix)` returns a new `MigrationContext` rooted at `{current_prefix}.{sub_prefix}`. Used internally by `ctx.nested()` and rarely needed directly.

## Values from the application

A `#[migrate]` step is a bare `fn` and captures nothing, and a closure registered here has to be `'static`. So what a step needs from the application — a lookup table, the settings it is porting away from — is handed to the builder, one value per type, and asked for by type:

<!-- shown: a value the application provides -->
```rust
struct LegacyDefaults {
    port: u16,
}

let (store, report) = StoreBuilder::new(app)
    .provide(LegacyDefaults { port: 8080 })
    .migrations(|m| {
        m.for_prefix("net")
            .step(1, "fill in the port the old build assumed", |ctx| {
                let port = ctx.require::<LegacyDefaults>()?.port;
                ctx.set("port", &port)
            });
    })
    .build_with_migration()?;
```
<!-- /shown -->

`ctx.provided::<T>()` answers `None` where nothing of that type was given. `ctx.require::<T>()` makes that a failure, `RunStep::NothingProvided`, which names the type asked for and what was on offer instead.

## Reading across prefixes

A step that reads another prefix through `ctx.global_get` needs that prefix to have migrated first. Nothing is declared for this and no order is worked out up front: the read itself is what brings the other prefix up to date, on the spot, inside the same transaction.

```rust
m.for_node::<Profile>()
    .step(2, "snapshot plan from identity", |ctx| {
        let plan = ctx
            .global_get::<String>("complex_identity.plan")?
            .expect("reading it is what migrates it");
        ctx.set("plan_snapshot", &plan)?;
        Ok(())
    });
```

So what a read sees is the migrated value, and the ordering is the reaching rather than a list somebody has to keep in step with the code. A chain — `Workspace` reads `Profile`, `Profile` reads `Identity` — migrates all three in that order whatever order they were registered in.

A prefix reaching back into one that is part-way through is a cycle: neither can go first, and it comes back named end to end rather than at the one link that closed it.

## Reaching the context from #[migrate]

The same context is available to a generated step: a `#[migrate]` function can take a `MigrationContext` as a second argument, and everything on this page applies to it. What that is for, and what a step already cleans up without being asked, is on [Defining steps](/amethystate/migrations/defining-steps/#cleanup-the-step-already-does).

## Failure and rollback

One pass — the prefix it started at and every prefix its steps reached — is one transaction. A step that fails rolls the pass back and leaves the rest of the store alone.

What happens next is the open's to decide. `build` refuses to open, with `OpenStore::Migrating` carrying what the step said: the data under that prefix is not what the code now declares, and a store opened over it would hand new code old data.

<!-- shown: a step that fails refuses the open -->
```rust
let opened = StoreBuilder::new(app)
    .migrations(|m| {
        m.for_prefix("net").step(1, "turns the data down", |_| {
            Err(MigrationError::Custom("this data is not ours".into()).into())
        });
    })
    .build();

assert!(matches!(opened, Err(OpenStore::Migrating { .. })));
```
<!-- /shown -->

`build_with_migration` opens anyway and hands back the report, for an application that would rather decide. The prefixes that failed stay at the version they were at; everything else migrates as usual.

<!-- shown: opening anyway, and reading what failed -->
```rust
let (store, report) = StoreBuilder::new(app)
    .migrations(|m| {
        m.for_prefix("net").step(1, "turns the data down", |_| {
            Err(MigrationError::Custom("this data is not ours".into()).into())
        });
    })
    .build_with_migration()?;

for component in &report.components {
    if let ComponentOutcome::Failed { error, .. } = &component.outcome {
        eprintln!("{:?} was left as it was: {error:?}", component.prefixes);
    }
}
```
<!-- /shown -->

A failed component names the prefixes its pass held, and the error of a failed step carries the prefix, the version the step was taking it to, and the store's file.

### What a stopped open leaves on disk

A failing step leaves nothing on disk: its pass is rolled back before anything is written. The text engines are the ones with more to say, because they keep data and metadata in two files and replace them one at a time. So the metadata is written first, with a record of the write under way, then the data, then the metadata itself. A process that stops in between leaves that record behind, and the next open reads it: data that is what was being written finishes the metadata, data that is what the stopped open found drops the record and the migration runs again, and anything else refuses the open, since which data the metadata describes cannot be told.
