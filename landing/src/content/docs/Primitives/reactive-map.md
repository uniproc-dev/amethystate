---
title: "ReactiveMap<K, V>"
sidebar:
  label: ReactiveMap
  order: 9
---

A collection whose keys are decided at run time. Each entry is stored at its own
path, so a change is observable per entry as well as for the whole - which is
what separates it from keeping a `HashMap` in one field.

## Reading

<!-- shown: reading a map -->
```rust
let one: Option<u64> = widths.get("cpu");

if let Some(width) = widths.get("cpu") {
    println!("cpu is {width}");
}

let there = widths.contains_key("mem");
let how_many = widths.len();

for key in widths.keys() {
    println!("{key}");
}

for (key, value) in widths.entries() {
    println!("{key}: {value}");
}
```
<!-- /shown -->

`len` is answered from the map's own projection, so it is a counter read rather
than a scan, and it counts writes that have not reached disk yet.

`keys` and `entries` walk that projection one entry at a time. `keys` touches no
values at all - nothing beside a key is deserialized.

### Looking without taking

`entries` clones what it hands out, because `Iterator` cannot lend from the
thing it is iterating. `view` lends instead: bind it, then `iter()`, and nothing
is cloned that you do not clone yourself.

<!-- shown: looking at a map without cloning it -->
```rust
let held = widths.view();
let total: u64 = held.iter().map(|(_, width)| width).sum();

let mut widest = 0;
for (_, width) in &widths.view() {
    widest = widest.max(*width);
}
```
<!-- /shown -->

A `for` keeps the temporary alive for the whole loop, so the binding is only
needed when what you read outlives the loop.

### A walk owns a version

`view`, `entries` and `keys` each take the map's current version and read from
that. Nothing is locked while one is alive, so a walk neither waits for a writer
nor keeps one waiting, whatever thread either of them is on.

Writing to the map during a walk is ordinary, the walking thread included:

<!-- shown: walking a map and writing to it -->
```rust
let mut walked = Vec::new();

for (key, width) in widths.entries() {
    widths.remove(&key)?;
    walked.push((key, width));
}

assert_eq!(walked, [("cpu".to_string(), 120), ("mem".to_string(), 80)]);
assert!(widths.is_empty());
```
<!-- /shown -->

Every key is still offered, because the walk reads the version it started on and
the removals go to the map. A write that lands during a walk is in the next read
rather than that one.

That is the thing to know when a walk is kept rather than finished. A `view`
bound for a frame holds what the map held when it was taken, so two reads made
from it agree with each other even if the map moved between them. Holding it
costs the version it pins, which is what the map has dropped since - not a copy
of the map.

## Writing

<!-- shown: adding, changing and removing an entry -->
```rust
widths.insert("cpu".to_string(), &120)?;

widths.update("cpu", &200)?;
widths.modify("cpu", |width| *width += 10)?;

let absent = widths.update("gpu", &90);

widths.remove("cpu")?;
widths.clear()?;
```
<!-- /shown -->

`insert` adds a key or replaces it. `update` writes a key that is **already
there** and fails with `KeyNotFound` when it is not.

The reason is what subscribers are handed. `MapChange::Update` carries the
previous value, and a key that does not exist has none, so `update` on an absent
key could only announce a change it was unable to describe.

`modify` is `update` reached without rebuilding the value, which matters when it
is a large struct.

`remove` sends one `MapChange::Remove` carrying this handle's id, and one store
write. `clear` is the other shape: one event, one delete, whatever the map held.

To drop a subset, walk and remove - the walk reads its own version, so the loop
is the plain one:

```rust
for (key, width) in widths.entries() {
    if width < 80 {
        widths.remove(&key)?;
    }
}
```

## What order entries come back in

Sorted the way the store orders the keys these names become, so a scan and a map
list their entries alike. That is **not** the key type's own `Ord`:

<!-- shown: the order entries come back in -->
```rust
counts.insert("9".to_string(), &1)?;
counts.insert("10".to_string(), &1)?;
counts.insert("a.b".to_string(), &1)?;
counts.insert("a1b".to_string(), &1)?;

let order: Vec<String> = counts.keys().collect();
assert_eq!(order, ["10", "9", "a1b", "a.b"]);
```
<!-- /shown -->

Numbers sort as text, so `"10"` comes before `"9"`. A name holding the separator
sorts by its escape: `a.b` lands after `a1b`, because the key the store writes
begins `a\.`.

Insertion order is not recorded anywhere. If the order matters - table columns,
steps in a list - keep that order yourself and use the map for lookup.

## Hearing about a change

<!-- shown: hearing about a change -->
```rust
let _all = widths.subscribe_any(|change| {
    println!("{change:?}");
});

let _cpu = widths.subscribe_key("cpu".to_string(), |change| {
    println!("cpu: {change:?}");
});

widths.insert("cpu".to_string(), &120)?;
widths.insert("mem".to_string(), &80)?;
```
<!-- /shown -->

`subscribe_any` fires for every change to any key; `subscribe_key` narrows to
one. Both return a handle, and dropping it ends the subscription.

`subscription_with()` is the same with the links: `.key(..)` to narrow,
`.external()` to skip your own writes, `.stream()` to take the changes into a
loop of your own instead of a callback. See
[Subscriptions](/amethystate/concepts/subscriptions/).

### What `external` filters

On a map it filters `Update` and nothing else. `Insert`, `Remove` and `Clear`
reach every subscriber including the one that caused them.

The line is between editing a value and changing what the map holds. A value you
wrote yourself is your own business. A key appearing or disappearing changes the
shape of the map, and a view listing the keys has to rebuild whether or not it
was the one that added the key.

That has a consequence worth knowing: `insert` on a new key is an `Insert` and
on an existing one an `Update`, so whether your own call comes back to you
depends on whether the key was already there.

## One entry as a cell

`entry_cell(key)` gives a [`ReactiveCell`](/amethystate/primitives/reactive-cell/)
onto one entry, for handing a single value somewhere that should not know about
the map.
