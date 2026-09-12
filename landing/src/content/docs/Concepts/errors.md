---
title: What a failure carries
sidebar:
  label: Errors
  order: 20
---

**Each point in the API fails with the set that is possible there.** A
constructor answers with the five ways a struct can refuse to open; a `Kv` write
with the six ways a raw write can be turned down. The set lists them, so a
`match` over it is complete and the compiler keeps it that way.

Each set is an ordinary `std::error::Error`. So a refusal gets one of two
treatments, and both are cheap: take it apart, or hand it on.

## Taking a refusal apart

<!-- shown: telling one refusal from another -->
```rust
let refused = match Panel::new_with(&store) {
    Ok(_) => return Ok(()),
    Err(why) => why,
};

let said = match refused {
    OpenStruct::Refused { at, said } => format!("{at} was turned down: {said}"),
    OpenStruct::WillNotRead { at, why } => format!("{at} holds something else: {why}"),
    OpenStruct::Taken(taken) => format!("{} already holds it", taken.held_by),
    OpenStruct::NotAPath(why) => format!("that is not a path: {why}"),
    OpenStruct::Store(disk) => format!("the store: {disk}"),
};
```
<!-- /shown -->

No `_` arm, and that is the point of saying it in the type. A catch-all is
where a caller who wanted *refused* and met a slightly different failure ends
up without noticing; here there is nothing for it to catch. And when a call
gains a way to fail, every caller that was handling them all stops compiling -
which is what a caller wants to be told.

Every one of these sets is exhaustive, for the same reason.

**A variant carries everything known where it was raised:** the place together
with its owner, the place together with the reason, both sides of a collision.
`OpenStruct::Taken` carries four things - which place, who wanted it, through
which path it is held, and by whom - because a collision is only diagnosable
with all four and neither declaration mentions the other. A field you do not
need is one to skip; the one you need is there.

## Handing it on

<!-- shown: letting the caller's own error type take it -->
```rust
fn with_anyhow(store: &amethystate::Store) -> anyhow::Result<()> {
    store.set(["ui", "width"], &800u32)?;
    Ok(())
}

fn with_a_box(store: &amethystate::Store) -> Result<(), Box<dyn Error + Send + Sync>> {
    store.set(["ui", "height"], &600u32)?;
    Ok(())
}
```
<!-- /shown -->

`Send + Sync + 'static`, a `Display` writing one line, and a `source()` chaining
down to the cause. So `anyhow`, `eyre` and `Box<dyn Error>` all take one with a
plain `?`, and giving up costs nothing.

## The sets

| set | raised by |
| --- | --- |
| `OpenStruct` | `new`, `new_with`, `new_with_id`, `new_with_id_under`, `load`, `load_with`, `Kv::cell` |
| `OpenStore` | `StoreBuilder::build`, `build_with_migration`, `located` |
| `LoadMap` | `Kv::map`, and a map field's own constructor |
| `ReadValue` | `Store::get` |
| `WriteValue` | `Store::set`, `Store::delete`, `Field::set`, `ReactiveCell::set`/`update`/`modify`, `ReactiveMap::insert` |
| `KvWrite` | `Kv::get`, `Kv::set`, `Kv::remove` |
| `ScanKeys` | `Store::scan_keys`, `Store::scan_prefix`, `Kv::keys` |
| `Flush` | `save_now`, `close`, `flush_prefix` |
| `RunStep` | every `MigrationContext` method, and what a migration step hands back |

They overlap and they are still separate types. `WriteValue` has `Intercepted`,
`Absent` and `SourceGone`, which a raw `Kv` write cannot reach; `KvWrite` has
`Declared`, which a write through a field cannot. Four variants are shared.
Written as one set, every caller would read past arms that cannot fire where
they are.

A map gets its own set rather than sharing `OpenStruct` because it is opened
over what is already under it, and so meets two failures nothing else can: a
stored key that is not one of its entries, and an entry whose *name* will not
read as the key type. A migration step gets one because the distinction it
needs - *this record is not what I expected*, which a step can often skip past,
against *the disk is broken*, which it cannot - has nowhere else to live.

`Field::try_get` is the odd one out and deliberately so: it is not a write, and
not the store's failure, but what this field and the store do not agree about.
It answers with a `Disagreement` - a path and one of four reasons - which is an
ordinary `Error` too. What each reason means:
[Defining structs](/amethystate/state/defining-structs/#what-a-value-going-wrong-does).

## The report is still there

A set's `Store(..)` variant carries what the engine said, whole, as a `Report`
from [`error_stack`](https://docs.rs/error-stack). So does every variant that
classified one: `WillNotRead`, `WillNotEncode` and `TooDeep` hold their report
rather than a rendered sentence, because the numbers are the diagnosis.

<!-- shown: the report under a variant that named the failure -->
```rust
let refused = store.get::<u16>(["port"]).unwrap_err();

let amethystate::store::ReadValue::WillNotRead { at, why } = refused else {
    panic!("the bytes are there and they are not a u16")
};
```
<!-- /shown -->

A report has two parts, and only the first is an enum:

- **a chain of contexts** - what failed, at each level that knew it was failing;
- **attachments** - the particulars, carried as types rather than as sentences.

Pulling one out of a set is a `match` like any other:

<!-- shown: getting at the report a set carries -->
```rust
fn what_the_store_said(why: LoadMap) -> StorageResult<()> {
    match why {
        LoadMap::EntryWillNotRead { why, .. } => Err(why.into_report()),
        other => panic!("an entry was expected to be at fault: {other}"),
    }
}
```
<!-- /shown -->

### The top names the operation

<!-- shown: what a failure says it is -->
```rust
let refused = store.kv().map::<String, u64>("labels").unwrap_err();
let report = what_the_store_said(refused).unwrap_err();

let context = report.current_context();
let sentence = report.to_string();
```
<!-- /shown -->

`context` is `StorageError::Codec`, and `sentence` is *the value could not be
encoded or decoded*.

`current_context()` is the outermost context. `StorageError`'s variants name
**the operation that failed**, not the thing that failed it: `Write`, `Scan`,
`Codec`. Two engines failing to write are the same context, told apart by the
frames underneath - which is deliberate, because a caller deciding what to do
next cares whether a write landed, not whether redb or `serde_json` was the one
to say no.

`StorageError` **is** `#[non_exhaustive]`, unlike the sets. It is the disk's own
list, it grows with the engines, and it is meant to be read rather than
dispatched on arm by arm.

### The particulars are attached as types

Each fact is its own type, so nothing is parsed back out of a message. They live
in `amethystate::errors::facts`: a key, the prefix a scan was under, the entry
it stopped on, the file it was reading, how large the value was. Each is a
newtype over the thing it holds, and its label lives on its `Display`.

`facts::all::<T, _>` hands back every fact of one type, innermost first:

<!-- shown: reaching the entry that failed -->
```rust
let refused = store.kv().map::<String, u64>("ports").unwrap_err();
let report = what_the_store_said(refused).unwrap_err();

let entries: Vec<&Entry> = facts::all::<Entry, _>(&report).collect();
let prefixes: Vec<&Prefix> = facts::all::<Prefix, _>(&report).collect();
```
<!-- /shown -->

`entries` holds one `Entry("http")` and `prefixes` one `Prefix("ports")`. A map
that will not open over one bad entry says which entry, and that is the part a
`{}` print discards.

Asking for a fact the report does not carry hands back nothing:

<!-- shown: asking for a fact the report does not carry -->
```rust
let refused = store.kv().map::<String, u64>("ports").unwrap_err();
let report = what_the_store_said(refused).unwrap_err();

let key = facts::all::<Key, _>(&report).next();
```
<!-- /shown -->

`key` is `None`: this report was never about a single key.

Which facts a report carries depends on who was on the stack: they are attached
by whoever knew them, and a `Key` is not attached by code that only ever saw a
prefix. Read them as evidence that is there when it is there, not as a schema.
Attaching is lazy - nothing is built on the path that succeeds.

This is the layer the sets exist to keep you out of. Reach for it when you are
writing a log line or a bug report, not when you are deciding what to do next.

## Printing one

A set's `{}` is one line, written for the person who has to fix it. `{:?}` on
the report underneath gives the whole thing - every context in the chain, with
the facts under the frame that attached them.

<!-- shown: an entry that will not decode -->
```rust
store.set(["ports", "http"], &"text".to_string())?;

let undecodable = store.kv().map::<String, u64>("ports").unwrap_err();
```
<!-- /shown -->

<!-- printed: an entry that will not decode from book_errors -->
```
the entry at ports.http will not read back

the value could not be encoded or decoded
├╴as: u64
├╴value bytes: 5
├╴prefix: ports
├╴entry: http
│
├─▶ Erased codec error: wrong msgpack marker FixStr(4)
│
╰─▶ wrong msgpack marker FixStr(4)
```
<!-- /printed -->

<!-- shown: an entry whose name is not the map's key type -->
```rust
let wrong_key = store.kv().map::<Id<u16>, String>("ports").unwrap_err();
```
<!-- /shown -->

<!-- printed: an entry whose name is not the map's key type from book_errors -->
```
`http` under ports will not read as a Id<u16>
```
<!-- /printed -->

That one has no report under it, and needs none: the variant already names the
map, the entry and the type it would not read as.

<!-- shown: a path that names nothing -->
```rust
let names_nothing = store.set(Vec::<String>::new(), &1u32).unwrap_err();
```
<!-- /shown -->

<!-- printed: a path that names nothing from book_errors -->
```
the write was given no path to land at: a path of no levels names the whole store - say `StorePath::root()` to mean it
```
<!-- /printed -->

<!-- shown: a path past the cap it was given -->
```rust
let shallow = StoreBuilder::new(settings)
    .limits(|l| l.key_depth(4))
    .build()?;

let too_deep = shallow.set(["a", "b", "c", "d", "e"], &1u32).unwrap_err();
```
<!-- /shown -->

<!-- printed: a path past the cap it was given from book_errors -->
```
a.b.c.d.e is deeper than this store reads back

deeper than this store reads back
├╴key: a.b.c.d.e
├╴levels: 5, and the limit is 4
├╴set by: limits(|l| l.key_depth(..))
╰╴what is stored here spends the same budget - this store reads 512 levels in all
```
<!-- /printed -->

The last one is the shape to aim for when a refusal is yours to write: the
sentence names what was refused, and the facts under it answer the question the
reader is about to ask - whose limit that was, and what it cost.

So `{:?}` on the report is what belongs in a log, and the set's `{}` is the one
line a person reads.

## When a bound asks for `std::error::Error`

A `Report` does not implement that trait, so it does not satisfy `E: Error` -
which is what `anyhow::Error::new` asks for, and `thiserror` through `#[source]`,
and any wrapper of your own over somebody else's error. The sets satisfy it, so
at the boundary the question does not come up; it comes up once a report has
been pulled out and has to cross on its own. `into_error` is the way, and it
loses nothing:

<!-- shown: turning a report into a std error -->
```rust
let std_error = report.into_error();

let sentence = std_error.to_string();
let whole = format!("{std_error:?}");
```
<!-- /shown -->

`sentence` is the same *the value could not be encoded or decoded*, and `whole`
still holds `entry: http`. The wrapper keeps the report behind it rather than
flattening it. `as_error` is the borrowing twin, for handing one out without
giving it up.

`error_stack` is re-exported as `amethystate::error_stack`, and `Report` also
sits in `amethystate::errors`, so naming one in your own signature costs no
dependency of your own and cannot drift out of version with this crate.
