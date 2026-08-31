---
title: What a store refuses to hold
sidebar:
  order: 14
---

Two different things refuse a write, and only one of them is yours to set.

## How deeply a value may nest

The limit is on the **shape of the value**. A struct holding a struct holding a
`Vec` of structs is three levels before anything else is counted, and a type
that refers to itself - a tree, a menu, an expression - has as many levels as
the data does on the day it runs.

Every engine's codec reads less deeply than it writes. `serde_json` stops at 128
on the way in and has no limit on the way out; `ron` stops at 64; `rmp_serde`
has no limit at all, and the stack runs out around three thousand instead -
killing the process rather than returning an error, on every later start,
because the value is already committed.

So without a check a write past the reader's ceiling is accepted and cannot be
read back: no error anywhere, and on the text engines the whole file is gone,
since the document is parsed as one thing. The ceiling is therefore enforced
always, whatever else is configured, and the refusal happens at the write.

**It is counted during the write, not before it.** The value cannot be inspected
first - by the time it reaches the store it is a `&dyn Serialize`, and a
five-level struct is indistinguishable from a five-level tree. Nor can it be
built and measured, because building it is the dangerous act: on redb that is
exactly what overflows the stack. Serde pushes and the store receives, so the
levels are counted as they go past, inside the codec's own pass.

<!-- shown: how deep the engine you are running reads -->
```rust
let engine = default_backend();
println!("{}: {} levels", engine.extension(), engine.depth_ceiling());
```
<!-- /shown -->

The numbers are measured rather than looked up - `tests/probe_*.rs` walked each
engine to its boundary:

| engine | levels |
| --- | --- |
| redb | 512 |
| SQLite | 254 |
| JSON | 127 |
| TOML | 80 |
| RON | 64 |

redb has no limit of its own. `rmp_serde` recurses until the stack ends, around
three thousand levels, and the process dies rather than returning an error - on
every later start, because the value is already committed. The 512 is imposed
for that reason: far above any data anyone means to store, far below where the
stack gives out.

### The path spends from the same budget

On every text engine a path's levels *become* document levels - `ui.panels.left`
is three levels of nesting in the file before the value starts - so the two are
counted together. A shallow value at a deep path is as unreadable as a deep
value at a shallow one.

SQLite is the exception, since its path is a `TEXT` key rather than nesting, but
paying the path there costs a few levels out of 254 and is not worth a second
rule.

When a write is refused the report says where the budget went: how many levels
the path took, how many were left for the value, and how many this store reads
in all.

## The two caps you can set

Both go through `limits`, and neither can raise the ceiling above - they only
narrow it.

<!-- shown: capping how deep a path may go -->
```rust
let store = StoreBuilder::new(settings)
    .limits(|l| l.key_depth(4))
    .build()?;

let deep = StorePath::from_segments(["a", "b", "c", "d", "e"]);

if let Err(refused) = store.set(&deep, &1u32) {
    println!("{refused:?}");
}
```
<!-- /shown -->

`key_depth` refuses a path with more levels than that, before the value is even
encoded. Useful for a store whose paths are built at run time, where a bug that
nests without bound otherwise shows up as an unreadable file much later.

<!-- shown: promising the contents stay readable on another engine -->
```rust
let store = StoreBuilder::new(settings)
    .limits(|l| l.portable_across([default_backend()]))
    .build()?;
```
<!-- /shown -->

`portable_across` names engines the contents must stay readable on beyond the
one actually running, and lowers the ceiling to the lowest of them. A store on
redb that names RON reads 64 levels rather than 512, so a value too deep for the
file it will be exported to is refused now instead of after the export.

It settles depth and nothing else. A value RON could not *represent* still
writes today; that check is not built.

Naming the engines rather than saying *all* is deliberate. "All" is a moving
target - it changes under a store when an engine is added - and the honest
requirement is usually narrower: an application shipping JSON on the desktop
and SQLite on a phone needs those two and has no opinion about RON.

## What this does not cover

Depth is what the store enforces. What a format can *express* is a different
question, and no setting changes it:
[Limitations](/amethystate/limitations/absent-or-null/).
