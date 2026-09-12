---
title: What serde says here
sidebar:
  order: 5
---

The library stores whatever serde will carry, so a stored type is written in two
vocabularies at once. This page is about where the line between them falls, and
it falls in one place: whether the type is serialized at all.

**A struct whose fields are paths** - one with `#[amethystate]`, or embedded
with `nested` - is not serialized, ever. Not "nothing gets round to it": no
`Serialize` is written for it. It is not a value, it is a description of where
other values live, and each of its fields goes to the store on its own, at its
own path, when its own turn comes.

So a `#[serde(..)]` written on it, or on one of its fields, is a compile error -
and rustc's own, not ours:

```
error: cannot find attribute `serde` in this scope
  = note: `serde` is an attribute that can be used by the derive macros
          `Deserialize` and `Serialize`, you might be missing a `derive` attribute
```

Which is the whole of it. The macro does not read serde, refuse it clause by
clause, or explain it away: it carries every attribute it does not understand
onto the field it generates, and the attribute is judged by whoever understands
it. Where there is no derive, there is no attribute.

**A leaf** - anything stored as one value at one path - is the opposite. The
store hands it to the codec and takes back bytes, and never looks inside. Every
serde attribute means there exactly what it means anywhere else.

## What says it instead

Four things people reach for serde to say have somewhere to be said here. The
spellings are kept apart because the operations are not the same one:

| in serde | here |
| --- | --- |
| `rename = ".."` | `#[amestate(path = "..")]` - and a dot in it is a level, where serde's is one name with a dot in it |
| `rename_all = ".."` | `rename_all` on the `#[amethystate(..)]` above |
| `default` on a field | `#[amestate(default = ..)]`, taking an expression rather than the path to a function |
| `flatten` | `#[amestate(flatten)]` on a `nested` field - and it moves segments on disk, where serde's merges a map into the map holding it |
| `with`, `serialize_with`, `deserialize_with` | `#[amestate(with = ..)]` and its halves, taking the same functions |

The rest of serde's vocabulary has nothing to be here. `deny_unknown_fields`,
`tag`, `untagged`, `transparent`, `from`, `remote`, `skip`, `alias`, `getter`,
`borrow` all describe one encoded value, and there is none: the fields are
separate paths, present or absent on their own. What answers a path that will
not read is `on_unreadable`; what a field kept out of the store entirely is, is
`#[amestate(volatile)]`; what moves data written under an older name is a
`#[migrate]` step with `#[rename(old => new)]`, which runs once at the open and
converges, where an alias would be kept for good.

All of it is written up where the vocabulary lives:
[Defining structs](/amethystate/state/defining-structs/).

## Inside a leaf

Nothing above applies. The store writes the whole value at one path and reads it
back, and what it is told in between is between you and serde:

<!-- shown: a leaf, where serde answers to nobody here -->
```rust
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Endpoint {
    pub host_name: String,

    #[serde(default)]
    pub port: u16,
}

#[amethystate(prefix = "svc")]
pub struct Service {
    #[amestate(default = Endpoint { host_name: "localhost".to_string(), port: 443 })]
    pub upstream: Endpoint,
}
```
<!-- /shown -->

`Endpoint` lands whole at `svc.upstream`, kebab-cased and strict about unknown
fields, and the store has no opinion about any of it.

Two things still cost something, and it is worth knowing which:

**A rename inside a leaf is a migration nothing here will notice.** The bytes
move and the Rust name does not. A store written by the old build has the old
field name inside the value, and the new build will not find it.

**`deny_unknown_fields` turns every field a later version adds into a hard read
failure for an older build.** proto3 dropped unknown fields in its first release
and reversed it in 3.5.0 for the same reason. It is your value and your call.

And two habits worth keeping, for the same reason any format keeps them:

**`#[serde(default)]` on every field added after the first release.** Serde
already ignores a field it does not know, so a value written by a newer build
reads on an older one for free. The other direction is not free: a field with no
default fails the whole value when it is missing, so a leaf stored before the
field existed stops reading.

**Named fields rather than a tuple.** A tuple struct serialises positionally, so
a field added in the middle silently reassigns every field after it.

## What a format will carry is a different list

None of this is about what an engine can hold. That is measured, and lives in
[Choosing an engine](/amethystate/choosing/): ron will not carry an enum, json and
sqlite will not carry a `NaN`, toml has no room past `i64`. Those refuse a
*value*; this page is about a *type* saying something the store cannot record.

A type can pass one list and fail the other.
