---
title: "Kv"
sidebar:
  order: 11
---

Reactive values addressed by path, with no struct declared. For a key set that
is not known at compile time, or where a schema is more ceremony than the job is
worth.

<!-- shown: reading and writing without a schema -->
```rust
let kv = store.kv();

kv.set("theme", &"dark".to_string())?;
let theme = kv.get::<String>("theme")?;

let ui = kv.namespace("ui");
ui.set("width", &800u32)?;
let under_ui = ui.keys()?;

kv.remove("theme")?;
```
<!-- /shown -->

## A name is one level

Every name here is a **single level**, and `namespace` is what nests. So
`kv.set("ui.width", ..)` writes one key called `ui.width` - a name that happens
to hold a dot - while `kv.namespace("ui").set("width", ..)` writes `width`
inside `ui`.

The dot in a name is escaped on disk, so the two are different places and stay
different. Getting this the wrong way round is the mistake to watch for: a
dotted string looks like a path and is not one.

`keys()` lists everything under the handle it is called on, at every level below
it and sorted. Narrowing a listing is what `namespace` is for; the root handle
lists the whole store.

<!-- shown: what a listing covers -->
```rust
kv.set("theme", &"dark".to_string())?;
kv.namespace("ui").set("width", &800u32)?;
kv.namespace("ui").namespace("panel").set("left", &true)?;

let from_ui: Vec<String> = kv
    .namespace("ui")
    .keys()?
    .iter()
    .map(|key| key.as_str().to_string())
    .collect();

assert_eq!(from_ui, ["ui.panel.left", "ui.width"]);
```
<!-- /shown -->

The paths that come back are whole, so a `width` inside `ui` is listed as
`ui.width`, and `theme` at the root is not listed at all from a `ui` handle.

## Cells and maps

<!-- shown: a cell and a map with nothing declared -->
```rust
let width = kv.namespace("ui").cell("width", 800u32)?;
let flags = kv.map::<String, bool>("flags")?;

flags.insert("dark".to_string(), &true)?;
```
<!-- /shown -->

What comes back is an ordinary
[`ReactiveCell`](/amethystate/primitives/reactive-cell/) or `ReactiveMap`, so
subscriptions work as they do for declared fields. The type of a cell comes from
its default, so there is nothing to spell out twice.

## Where a struct already lives

A declared `prefix` belongs to that struct, and writing inside it through `Kv`
is refused:

<!-- shown: writing where a struct lives -->
```rust
let refused = kv.namespace("network").set("port", &"8080".to_string());

kv.namespace("networkish").set("port", &"8080".to_string())?;
```
<!-- /shown -->

The refusal is `WriteError::SchemaOwned`, and it names the path, the declared
prefix and the struct that declared it.

The reason is what a wrong type does to the struct. Storing a `String` where a
`u16` is declared leaves the field's subscription unable to decode it, so the
field goes on showing its old value while the store holds something else - and
the next startup fails outright reading that path back.

## One path, one type

<!-- shown: asking for one path as two types -->
```rust
let ui = kv.namespace("ui");

let _width = ui.cell("width", 800u32)?;
let refused = ui.cell("width", String::new());
```
<!-- /shown -->

Nothing here records a type. A cell reads its path to seed itself, so the second
ask finds what the first stored and fails to decode it. The error is a codec
error naming what it found.

The check is therefore the stored value, and that has consequences a registry
would not. It holds across restarts, because what an earlier run wrote is still
there to be read. It holds over a path that was empty, because the first cell
**writes its default** there. And it does not hold for raw access at all:

```rust
kv.set("thing", &1u32)?;
kv.set("thing", &"now a string".to_string())?;   // replaces it
kv.get::<u32>("thing")?;                          // Err, at the read
```

A raw `set` overwrites whatever was there, whatever its type. The disagreement
surfaces at the next `get` that asks for the old one.

## What you give up

No versions, no migrations, no drift detection - those belong to the declared
structs. If the data has a shape worth evolving, declare it.
