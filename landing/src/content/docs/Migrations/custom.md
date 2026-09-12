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

A path in `amethystate` is a list of levels, and the dots are only how one is spelled where a path is shown: `prefix.field` for an ordinary field, `prefix.field.key` for a map entry. The bytes in the database look nothing like it. A flat engine lays the levels down in order, ending each with a zero byte and standing in for the zeros and ones inside a name with pairs - so that byte order is level order and a subtree is a byte prefix with nothing left to check. Nothing escapes the dot: it is an ordinary character in a name, and the terminator is what marks the boundary.

So matching your own keys against the string `prefix.field` gets you nowhere; what they have to match is the list of levels. There is no automatic path out of another layout: a one-time export is what it takes.

The simplest approach is to write a one-time migration step using the manual migration API — read from the old database inside the step closure and write values via `ctx.set`. See [Manual Migrations](./manual) for the full context API.

## Custom file format

If your format is not TOML, JSON, or RON, you need a one-time conversion before the first run. Read the old file with whatever parser you currently use, then write the values into an `amethystate` store directly. After that, remove the old file and let `amethystate` manage persistence going forward.