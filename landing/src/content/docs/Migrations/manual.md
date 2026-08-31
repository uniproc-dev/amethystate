---
title: Manual Migrations
sidebar:
  order: 22
---

Codegen migrations cover the common case: rename fields, change types, fill in defaults. When that isn't enough — cross-node reads, data backfills, key cleanup inside a `ReactiveMap` — you can write migration steps by hand.

## Entry point

Manual steps are registered through the `.migrations()` builder method:

```rust
let (store, report) = StoreBuilder::new("./app.redb")
    .migrations(|m| {
        m.collect_codegen(); // include all #[migrate] steps
        // register manual steps here
    })
    .build()?;
```

`collect_codegen()` pulls in all steps defined with the `#[migrate]` attribute. You can call it alongside manual steps in any order — the migrator resolves execution order via topological sort regardless.

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

`for_node::<T>()` targets the struct by its prefix. `.step(version, description, closure)` registers the transformation that brings it from `version - 1` to `version`.

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
| `ctx.scan_map::<K, V>(key)` | Scan all entries under `prefix.key.*` and return them as a `HashMap<K, V>`. |

Useful when migrating a `ReactiveMap` field without going through `AmeData`:

```rust
let old_routes = ctx.scan_map::<String, String>("routes")?;
for (k, _) in &old_routes {
    ctx.delete(&format!("routes.{k}"))?;
}
```

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

## Cross-node dependencies

If a step reads from another node via `ctx.global_get`, that node must have already migrated. Declare the dependency explicitly with `.depends_on()`:

```rust
m.for_node::<Profile>()
    .depends_on::<Identity>()
    .step(2, "snapshot plan from identity", |ctx| {
        let plan = ctx
            .global_get::<String>("complex_identity.plan")?
            .expect("identity should have migrated first");
        ctx.set("plan_snapshot", &plan)?;
        Ok(())
    });
```

The migrator uses declared dependencies to determine execution order. If `Identity` itself has steps, they are guaranteed to complete before this step runs.

Dependencies compose into a graph. A chain like `Workspace` → `Profile` → `Identity` means all three migrate in order, regardless of registration order.

## Accessing MigrationContext from #[migrate]

When a codegen migration needs to clean up keys that `AmeData` doesn't cover — for example, deleting old entries from a `ReactiveMap` — the `#[migrate]` function can take a `MigrationContext` as a second argument:

```rust
#[migrate]
fn migrate_proxy_config_v1_to_v2(
    old: AmeData<v1::ProxyConfig>,
    ctx: &mut MigrationContext,
) -> amethystate::Result<AmeData<ProxyConfig>> {
    for key in old.routes.keys() {
        ctx.delete(&format!("routes.{}", key))?;
    }

    let endpoints = old.routes
        .into_iter()
        .filter(|(k, _)| k != "obsolete")
        .map(|(k, v)| (k, ProxyEndpoint { url: v, timeout_ms: 5000 }))
        .collect();

    Ok(AmeData::<ProxyConfig> {
        name: old.name,
        endpoints,
    })
}
```

Without the explicit `ctx.delete` calls, old `routes.*` keys would remain in the store after migration. `AmeData` only covers fields that exist in the struct; anything else has to be cleaned up manually.

## Failure and rollback

Nodes linked by dependencies are grouped into a transaction. If any step in the group fails, all changes in the group are rolled back. Nodes in other groups are not affected.

```
❌ Component ["complex_broken_child", "complex_broken_root"] failed: Migration error: intentional failure
   Transaction rolled back. Data for these prefixes remains unchanged.
```

After a failed component, the store is still usable. Nodes that migrated successfully are available. Nodes in the failed component remain at their previous version.

The full outcome is available in the `MigrationReport` returned from `.build()`:

```rust
let (store, report) = StoreBuilder::new("./app.redb")
    .migrations(|m| { ... })
    .build()?;

if report.has_failures() {
    for component in &report.components {
        // component.outcome, component.prefixes
    }
}
```