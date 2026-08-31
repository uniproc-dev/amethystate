---
title: What a failure carries
sidebar:
  label: Errors
  order: 20
---

Every fallible call in this library answers with a `Report`, from
[`error_stack`](https://docs.rs/error-stack). It has two parts, and only the
first one is an enum:

- **a chain of contexts** - what failed, at each level that knew it was failing;
- **attachments** - the particulars, carried as types rather than as sentences.

The consequence to know before anything else: a caller that prints the error
with `{}` sees the top sentence and throws the rest away.

## The top names the operation

A map opened over an entry the store cannot decode as its value type refuses to
open at all. That is the failure the rest of this page reads:

<!-- shown: what a failure says it is -->
```rust
let refused = store.kv().map::<String, u64>("labels").unwrap_err();

let context = refused.current_context();
let sentence = refused.to_string();
```
<!-- /shown -->

`context` is `WriteError::Storage`, and `sentence` is *the store could not carry
out the write*.

`current_context()` is the outermost context. The variants name **the operation
that failed**, not the thing that failed it: `StorageError::Write`,
`StorageError::Scan`, `StorageError::Codec`. Two engines failing to write are
the same context, told apart by the frames underneath.

That is deliberate. A caller deciding what to do next cares whether a write
landed, not whether redb or `serde_json` was the one to say no - and matching
on the engine would tie the caller to which engine is configured.

A variant carries data only where that data **is** the failure rather than the
circumstances of it: `WriteError::KeyNotFound(key)` names the key because
nothing else in the report could, and `WriteError::SchemaOwned { path, declared }`
names both places because the collision is between them.

Both context types compare, so `==` is enough to test which one you have.
`matches!` is for the variants carrying data, when the variant is the question
and the payload is not.

## The particulars are attached as types

Each fact is its own type, so nothing is parsed back out of a message. They live
in `amethystate::errors::facts`: a key, the prefix a scan was under, the entry it
stopped on, the file it was reading, how large the value was. Each is a newtype
over the thing it holds, and its label lives on its `Display`.

`facts::all::<T, _>` hands back every fact of one type, innermost first:

<!-- shown: reaching the entry that failed -->
```rust
let refused = store.kv().map::<u16, u64>("ports").unwrap_err();

let entries: Vec<&Entry> = facts::all::<Entry, _>(&refused).collect();
let prefixes: Vec<&Prefix> = facts::all::<Prefix, _>(&refused).collect();
```
<!-- /shown -->

`entries` holds one `Entry("http")` and `prefixes` one `Prefix("ports")`. A map
that will not open over one bad entry says which entry, and that is the part a
`{}` print discards.

Asking for a fact the report does not carry hands back nothing:

<!-- shown: asking for a fact the report does not carry -->
```rust
let refused = store.kv().map::<u16, u64>("ports").unwrap_err();

let key = facts::all::<Key, _>(&refused).next();
```
<!-- /shown -->

`key` is `None`: this report was never about a single key.

Which facts a report carries depends on who was on the stack: they are attached
by whoever knew them, and a `Key` is not attached by code that only ever saw a
prefix. Read them as evidence that is there when it is there, not as a schema.

Attaching is lazy. Nothing is built on the path that succeeds.

## Printing one

`{}` gives the top context and nothing else. `{:?}` gives the whole thing - every
context in the chain, with the facts under the frame that attached them. The
three below are real, printed by the run that fills this page.

<!-- shown: an entry that will not decode -->
```rust
store.set(["ports", "http"], &1u64)?;

let undecodable = store.kv().map::<u16, u64>("ports").unwrap_err();
```
<!-- /shown -->

<!-- printed: an entry that will not decode from book_errors -->
```
the store could not carry out the write
├╴prefix: ports
│
╰─▶ the value could not be encoded or decoded
    ├╴prefix: ports
    ├╴entry: http
    ╰╴key type: u16
```
<!-- /printed -->

<!-- shown: a name that cannot be a level -->
```rust
let empty_level = store.set([""], &1u32).unwrap_err();
```
<!-- /shown -->

<!-- printed: a name that cannot be a level from book_errors -->
```
a name that cannot be a level
│
╰─▶ level 0 of the path has no name
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

So `{:?}` is what belongs in a log, and `{}` is for the one line a person reads.
A report reaching a user through `{}` alone has been stripped of the part that
says where to look.

## Handing one to code that wants a `std` error

<!-- shown: handing a report to something that wants a std error -->
```rust
fn writing(store: &amethystate::Store) -> Result<(), Box<dyn Error + Send + Sync>> {
    store.set(["ui", "width"], &800u32)?;
    Ok(())
}
```
<!-- /shown -->

`?` carries a report into `Box<dyn Error + Send + Sync>`, which is what makes it
usable from a test or a `main` without a shim.

A `Report` is not itself a `std::error::Error`, so it does not satisfy a bound
that asks for that trait - `anyhow::Error` asks for one to wrap, and so does
plenty of code that predates this style. `into_error` is the way across, and it
loses nothing:

<!-- shown: turning a report into a std error -->
```rust
let std_error = refused.into_error();

let sentence = std_error.to_string();
let whole = format!("{std_error:?}");
```
<!-- /shown -->

`sentence` is the same *the store could not carry out the write*, and `whole`
still holds `entry: http`. The wrapper keeps the report behind it rather than
flattening it, so nothing is lost by crossing over. `as_error` is the borrowing
twin, for handing one out without giving it up.

`error_stack` is re-exported as `amethystate::error_stack`, and `Report` also
sits in `amethystate::errors`, so naming one in your own signature costs no
dependency of your own and cannot drift out of version with this crate.
