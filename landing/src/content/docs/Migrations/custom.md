---
title: Migrating from a custom solution
sidebar:
  order: 24
---

## Plain serde + file

If your current setup is a struct, `serde`, and a manual load/save call, the migration is straightforward. Use `as_root` with the matching backend to read the existing file without any data conversion:

```rust
#[amethystate(mode = "persistent", as_root)]
pub struct AppConfig {
    pub name: String,
    pub port: u16,
}
```

```toml
amethystate = { version = "0.20", default-features = false, features = ["toml"] }
```

`as_root` writes fields directly to the store root with no prefix namespace — the same flat layout your existing file has. On first load, `amethystate` reads the existing keys as-is.

## Embedded database (redb, sled, and similar)

`amethystate` uses its own key layout: `prefix.field` for regular fields, `prefix.field.key` for map entries. If your current database uses a different layout, there is no automatic path — you need a one-time export before switching.

The simplest approach is to write a one-time migration step using the manual migration API — read from the old database inside the step closure and write values via `ctx.set`. See [Manual Migrations](./manual) for the full context API.

## Custom file format

If your format is not TOML, JSON, or RON, you need a one-time conversion before the first run. Read the old file with whatever parser you currently use, then write the values into an `amethystate` store directly. After that, remove the old file and let `amethystate` manage persistence going forward.