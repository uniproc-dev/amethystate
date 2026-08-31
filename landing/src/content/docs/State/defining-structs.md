---
title: Defining Structs
sidebar:
  order: 4
---

## The `#[amethystate]` macro

The `#[amethystate]` macro transforms a plain Rust struct into a persistent state container.

### Struct attributes

```rust
#[amethystate(prefix = "network", version = 1, mode = "reactive", as_root)]
pub struct NetworkState { ... }
```

| Attribute | Type | Description |
|-----------|------|-------------|
| `prefix` | `String` | Namespace path in the store. Required for root structs. |
| `version` | `u32` | Schema version for migrations. Defaults to `0`. |
| `mode` | `String` | Code generation mode: `"reactive"` (default), `"persistent"`, or `"both"`. |
|`as_root`| `flag` | If specified, fields are written directly to the store root without a namespace. |
| `on_unreadable` | variant | What opening does about a stored value that will not decode. `Refuse` (the default) or `UseDefault`. |
| `on_delete` | variant | What a field does when its key is deleted under it. `Keep` (the default) or `UseDefault`. |
| `check` | `fn` | A rule about the whole struct, run once every field is built. |

Structs without `prefix` are nested components, intended to be embedded in other structs via `nested`.

A `prefix` claims the place it names and everything under it, so two structs
cannot be declared over the same place - the second one to open is refused. That
is a whole subject of its own, and the one that decides how a `prefix` and a
dotted `key` interact: [Who owns which place](/amethystate/concepts/claims/).

### Field attributes

Field attributes are optional. A field with no `#[amestate]` annotation uses `Default::default()` as its value and the field name as its storage key.

```rust
#[amethystate(prefix = "app")]
pub struct AppState {
    pub counter: u32, // no annotation — uses Default::default(), stored as "app.counter"

    #[amestate(default = 8080)]
    pub port: u16,
}
```

| Attribute | Type | Description |
|-----------|------|-------------|
| `default` | `Expr` | Initial value on first run. If omitted, uses `Default::default()`. |
| `key` | `String` | Overrides the storage key. Defaults to the field name. |
| `nested` | flag | Marks field as an embedded `#[amethystate]` struct. |
| `volatile` | flag | In-memory only. Never read from or written to the store. Resets to default on every restart. |
| `on_unreadable` | variant | This field's answer, overriding the struct's. |
| `on_delete` | variant | The same for a deleted key. |
| `check` | `fn` | A rule every value coming in from the store has to pass. |

Those seven are the whole set; anything else is a compile error naming the seven.

### What a value going wrong does

Three moments, each with its own answer.

**Opening.** A declared path holding something that will not decode into the
field's type refuses construction and names the path. That is `Refuse`, the
default. `UseDefault` is for the application that has to start anyway: the field
takes its declared default, the stored value stays on disk for somebody to fix,
and [`try_get`](/amethystate/primitives/field/) answers `Err` from construction
until a change decodes.

<!-- shown: a struct that opens over a value it cannot read -->
```rust
#[amethystate(prefix = "mixed", on_unreadable = UseDefault)]
pub struct Mixed {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "".to_string(), on_unreadable = Refuse)]
    pub licence: String,
}
```
<!-- /shown -->

**A field may tighten what its struct wrote.** Above, the settings open with a
broken `port`, and a `licence` that will not read stops the whole thing.
`Refuse` on the struct with `UseDefault` on a field is a compile error naming
the field. A `nested` struct inherits its holder's answer, tightens it the same
way, and is checked against the holder while it compiles.

**A key deleted under a live field.** The field goes on reporting what it last
held: that is what was on screen a moment ago, and the declared default is a
compile-time guess. `UseDefault` asks for the guess:

<!-- shown: a field that wants the default back when its key goes -->
```rust
#[amethystate(prefix = "mixed_delete")]
pub struct MixedDelete {
    #[amestate(default = 800u32)]
    pub width: u32,

    #[amestate(default = 600u32, on_delete = UseDefault)]
    pub height: u32,
}
```
<!-- /shown -->

**A live change that will not decode.** The field keeps the last value the store
agreed with and no subscriber is called. `try_get` reports it, and clears itself
as soon as a change decodes. There is nothing to declare here.

### A value that decodes and is nonsense

Everything above is about bytes that will not read. A window position of
-32000, a font size of zero and the name of a theme nobody installed all decode
perfectly, and a `check` is where the application says it will not have them.

A field's check is a bare `fn` taking the value and a context, and it answers
with a reason rather than a `bool` - the reason is what `try_get` reports and
what a refused open carries, so it is written for whoever has to fix the file.

<!-- shown: a check on a field, and the world it is judged against -->
```rust
fn a_size_that_renders(size: &u8, _cx: &CheckContext) -> Result<(), Invalid> {
    if *size >= 6 {
        Ok(())
    } else {
        Err(Invalid::new("a font size below 6 renders nothing"))
    }
}

fn a_theme_that_is_installed(theme: &String, cx: &CheckContext) -> Result<(), Invalid> {
    let installed = cx.require::<InstalledThemes>()?;

    if installed.0.contains(&theme.as_str()) {
        Ok(())
    } else {
        Err(Invalid::new(format!("no theme called {theme} is installed")))
    }
}

#[amethystate(prefix = "checked_lenient", on_unreadable = UseDefault)]
pub struct LenientUi {
    #[amestate(default = 14u8, check = a_size_that_renders)]
    pub font_size: u8,

    #[amestate(default = "dark".to_string(), check = a_theme_that_is_installed)]
    pub theme: String,
}
```
<!-- /shown -->

The context is the answer to a check being a bare `fn`: it captures nothing, so
the world it judges a value against - which monitors exist, which themes are
installed - is handed to the store when it opens.

```rust
let store = StoreBuilder::new(settings)
    .context(InstalledThemes(installed))
    .build()?;
```

One value per type, asked for with `cx.get::<T>()` or `cx.require::<T>()`.
`require` refuses the value when nothing was given, because a check that cannot
reach its world cannot say the value is good.

A refused value is the situation `on_unreadable` already describes, and it is
answered the same way: `Refuse` fails construction naming the path and the
reason, `UseDefault` takes the declared default, leaves the stored value on disk
and answers `try_get` with `Err` until a change passes.

### A rule about the struct, not the value

A field's check sees one value. It cannot see its siblings - fields are built
one at a time, and the others do not exist yet - so an invariant between two of
them goes on the struct, which is handed the whole thing once every field is
built.

<!-- shown: a check on the struct, for what one field cannot see -->
```rust
fn the_window_can_be_drawn(window: &LenientWindow, _cx: &CheckContext) -> Result<(), Invalid> {
    if window.min().get() <= window.max().get() {
        Ok(())
    } else {
        Err(Invalid::new("the smallest window is wider than the largest")
            .at(&["min", "max"]))
    }
}

#[amethystate(
    prefix = "window_lenient",
    on_unreadable = UseDefault,
    check = the_window_can_be_drawn
)]
pub struct LenientWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,

    #[amestate(default = "amethystate".to_string())]
    pub title: String,
}
```
<!-- /shown -->

`at` names the fields the verdict is about, and only those report it: asking an
unrelated `title` still answers what it holds. A verdict that names none is
about all of them.

Under `UseDefault` a refused struct **keeps what was stored** rather than
resetting to the defaults. There is a declared default for a value and none for
a relationship, and what is in the fields is still what the file says - the
complaint arrives through `try_get` on the named fields, not by the values
changing under the reader.

A nested struct is settled before the struct holding it is built, so a parent's
check sees children that have already had their own.

### Where a check runs, and where it does not

| a value arrives | a field's check | a struct's check |
| --- | --- | --- |
| the struct is built | runs | runs |
| an edit from outside the process | runs; a refusal keeps the last good value and wakes nobody | does not run |
| `load_with`, under `mode = "persistent"` | runs | cannot be declared |
| a write this process made itself | does not run | does not run |
| a migration step | does not run | does not run |

Two of those rows are worth reading twice.

**Your own `field.set(nonsense)` does not go through the check** and lands on
disk; the refusal arrives at the next open. The door for a value this process
is writing is an
[interceptor](/amethystate/concepts/subscriptions/), which can refuse a write
before it happens - a check cannot refuse what is already stored.

**Under `mode = "persistent"` there is no `Field`, so there is no `try_get`.**
A refused value under `UseDefault` takes the declared default and says so in the
log, and that is the only place it is said. `Refuse` - the default - fails the
load instead, which is the answer to reach for when a loaded struct has to be
trustworthy.

A struct's check is refused at compile time under `mode = "persistent"`, since
there is no struct to hand it. So are checks on `volatile` fields, which nothing
arrives at; on `nested` fields, which want the check on the nested struct
itself; and on maps, whose entries are data rather than declared paths.

## #[derive(AmeType)]

`#[derive(AmeType)]` is what lets a plain Rust struct be used as the value of an
`#[amethystate]` field. It computes a compile-time `TYPE_HASH` from the type's
shape, and that number is what the migration pass compares to notice that a
declaration has changed since the data was written.

The hash is a summary, not an identity: distinct shapes can land on the same
number, and where they do, a change goes unnoticed and no drift is reported.
Bumping `version` when a shape changes is the thing that does not depend on it.

```rust
#[derive(Debug, AmeType)]
pub struct CustomEndpoint {
    pub host: String,
    pub port: u16,
}
```

## Volatile fields

Volatile fields live in memory only and reset to their default on every restart. Useful for transient UI state that should not persist.

```rust
#[amethystate(prefix = "app")]
pub struct AppState {
    #[amestate(default = 8080)]
    pub port: u16,

    #[amestate(default = false, volatile)]
    pub loading: bool, // always starts as false, never written to disk
}
```

## Nested structs

Structs without a `prefix` are components — they have no storage namespace of their own and are embedded into a parent struct via `nested`. The parent's prefix is prepended to all nested fields.

```rust
#[amethystate]
pub struct DatabaseConfig {
    #[amestate(default = "localhost".to_string())]
    pub host: String,
}

#[amethystate(prefix = "sys")]
pub struct SystemSettings {
    #[amestate(nested)]
    pub db: DatabaseConfig, // stored as "sys.db.host"
}
```

## Sharing one place between two structs

Two structs cannot both declare the same place - the second to open is refused.
Where one value has to be reachable from two sides, address it by path from the
one that did not declare it: [Kv](/amethystate/primitives/kv/) reads and writes
anywhere no struct has claimed, and
[Who owns which place](/amethystate/concepts/claims/) is what decides where that
line falls.

## Root-level storage (`as_root`)

By default, all fields are stored under the struct's `prefix`. With `as_root`, fields are written directly to the store root with no namespace.

```rust
#[amethystate(mode = "persistent", as_root)]
pub struct AppConfig {
    #[amestate(default = "acme".to_string())]
    pub name: String,

    #[amestate(default = false)]
    pub verbose: bool,
}
```

This produces a file like:

```toml
name = "acme"
verbose = false
```

That is the shape to ask for when the file is read by something other than this
crate — a config somebody edits by hand, or one whose keys another program
already expects at the top level. Root fields are claimed like any others, so
two structs reaching for the same key still collide:
[Who owns which place](/amethystate/concepts/claims/).