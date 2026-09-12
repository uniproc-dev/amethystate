---
title: "ReactiveCell<T>"
sidebar:
  label: ReactiveCell
  order: 10
---

The one type every reactive value can become. A field, a map entry, a path with
no struct behind it, or a plain in-memory value all erase into it - so code that
needs "a `u64` I can read, write and watch" does not have to name which of the
four it got, or carry the store backend and access mode in its own signature.

<!-- shown: four ways to reach a cell -->
```rust
let width = state.sidebar_width().cell();
let cpu_column = state.widths().entry_cell("cpu".to_string());
let by_path = store.kv().cell("dragging", 0u64)?;
let loose = ReactiveCell::new(0u64);

let mut columns: HashMap<String, ReactiveCell<u64>> = HashMap::new();
columns.insert("sidebar".to_string(), width);
columns.insert("cpu".to_string(), cpu_column);
columns.insert("dragging".to_string(), by_path);
columns.insert("loose".to_string(), loose);
```
<!-- /shown -->

Three of those write through to the store. `ReactiveCell::new` is the one that
does not: it holds its value in memory and nothing survives the process. A cell
from a field declared `#[amestate(volatile)]` is the same - the field never had
a store subscription for the cell to commit through.

`kv.cell` is the one that needs no declaration at all: it takes the path
and the default at the call, and remembers the type for the rest of the run - a
second call at the same path with a different type is refused.
[Kv](/amethystate/primitives/kv/).

## Reading

```rust
fn get(&self) -> Option<T>
```

`get` answers `None` where the value is not there to be had - a map entry whose
key is absent, or a cell whose source is gone. That is what separates it from
[`Field`](/amethystate/primitives/field/), where the declaration's default means
there is always a value.

It reads a cache the cell holds, so it costs the same as reading the primitive
directly - cheap enough for a render loop that reads every frame.

<!-- shown: reading, writing and watching a cell -->
```rust
let current = cell.get();

let _sub = cell.subscribe(|width| println!("width -> {width:?}"));

cell.set(200)?;
cell.update(|width| width + 10)?;
cell.modify(|width| *width += 10)?;
```
<!-- /shown -->

## Writes land where the value lives

A cell writes through to whatever is behind it. There is no way to obtain one
whose writes go into a cache and stop there.

The cache is left alone on the way in: it is updated when the store reports what
it committed, so a refused write never shows up in `get`.

## A cell onto a map entry

<!-- shown: a cell onto a map entry -->
```rust
state.widths().insert("cpu".to_string(), &120)?;

let cpu = state.widths().entry_cell("cpu".to_string());
let absent = state.widths().entry_cell("gpu".to_string());

assert_eq!(cpu.get(), Some(120));
assert_eq!(absent.get(), None);

state.widths().remove("cpu")?;

assert_eq!(cpu.get(), None);
assert!(cpu.set(80).is_err());
```
<!-- /shown -->

An entry cell is empty while its key is absent, and removing the key empties it
again. `set` on an empty one is refused - a cell is a view onto an entry, and
putting the key back is the map's business.

## What a cell keeps alive

These are `Rc` and `Weak`, and not by analogy: what a cell keeps is an
`Arc::downgrade` of the field.

`cell()` and `entry_cell()` make a **view** - the `Weak`. The cell holds its
source weakly and reads `None` once the last real handle to that source goes.
Like a `Weak`, the cell itself is fine; what fails is the upgrade inside it,
which is where the `None` comes from and the `WriteValue::SourceGone` on a
write.

`into_cell()` and `into_entry_cell()` make a cell that **owns** its source -
the `Rc`. The handle you hand them is the one they keep, and they keep it
strongly. Nothing is taken from the struct - a handle cannot be taken out of
one at all: `state.sidebar_width()` and `state.widths()` both hand out an
`Arc::clone` of the same thing, so the cell becomes one more owner, exactly
like another `Rc::clone`. Reach for those wherever the cell is the handle that
survives - stored in a component, put in a `HashMap`, handed to another thread.

<!-- shown: a view, and a cell that owns what feeds it -->
```rust
let view = state.sidebar_width().cell();
let owned = state.sidebar_width().into_cell();

drop(state);
```
<!-- /shown -->

Both of those came from the same field, and the owning one is what keeps it
alive - so the view above still answers after the struct is dropped, and goes
empty only once the owning cell goes too. It is the same count an `Rc` keeps.
