---
title: Defining Structs
sidebar:
  order: 4
---

## The `#[amethystate]` macro

The `#[amethystate]` macro transforms a plain Rust struct into a persistent state container.

### Struct attributes

```rust
#[amethystate(prefix = "network", version = 1, mode = "reactive")]
pub struct NetworkState { ... }
```

| Attribute | Type | Description |
|-----------|------|-------------|
| `prefix` | `String` | The place in the store these fields hang under. A root struct needs this or `as_root`. |
| `version` | `u32` | Schema version for migrations. Defaults to `0`. |
| `rename_all` | `&str` | How every field's own name is spelled where it is stored: `camelCase`, `kebab-case`, `PascalCase`, and the rest serde knows. A field with a `path` of its own is not touched by it. |
| `mode` | `String` | Code generation mode: `"reactive"` (default), `"persistent"`, or `"both"`. |
|`as_root`| `flag` | Fields sit at the top of the store, with no name above them. Written **instead of** `prefix` — the two say different things about the same place, so writing both is a compile error. |
| `on_unreadable` | variant | What opening does about a stored value that will not decode. `Refuse` (the default) or `UseDefault`. |
| `on_delete` | variant | What a field does when its key is deleted under it. `Keep` (the default) or `UseDefault`. |
| `unreadable_entries` | variant | What a `ReactiveMap` does with an entry it cannot read. `Refuse` (the default) or `Skip`. Fields that are not maps have no entries and ignore it. |
| `check` | `fn` | A rule about the whole struct, run once every field is built. |

A struct that says neither `prefix` nor `as_root` is a component: it has no
place of its own, and it goes inside another one through `nested`. Its place is
the field that holds it.

A `prefix` claims nothing by itself - the fields under it do, each its own path.
So two structs over the same prefix live together happily until two of their
paths meet, and the second one to open over a taken path is refused. A map is
the exception and owns everything beneath it, because its keys are made while
the program runs. That is a whole subject of its own, and the one that decides
how a `prefix` and a dotted stored name interact:
[Who owns which place](/amethystate/concepts/claims/).

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
| `path` | `&str` | Where the field is stored, when that is not its own name. A dot in it is a level, so a field can be put anywhere under the prefix and not only renamed. |
| `default` | `Expr` | Initial value on first run. If omitted, uses `Default::default()`. |
| `nested` | flag | Marks field as an embedded `#[amethystate]` struct. |
| `flatten` | flag | On a `nested` field: its fields sit at this level, with no segment named after it. |
| `volatile` | flag | In-memory only. Never read from or written to the store. Resets to default on every restart. |
| `with`, `serialize_with`, `deserialize_with` | path | The functions this field is written and read through, when its own type is not what writes it. |
| `on_unreadable` | variant | This field's answer, overriding the struct's. A map has no default to stand in for one entry, so writing it there is a compile error naming `unreadable_entries` instead. |
| `on_delete` | variant | The same for a deleted key. On a map it is the level this is about and not the entries - removing an entry is a removal under either answer. When the level itself goes, `UseDefault` puts the declared entries back and `Keep` (the default) leaves the map empty. |
| `unreadable_entries` | variant | On a map, and only on a map: `Skip` leaves out an entry it cannot read and builds the rest, `Refuse` (the default) fails and names the entry. |
| `check` | `fn` | A rule every value coming in from the store has to pass. |

They can be written in one `#[amestate(..)]` or spread over several, whichever
reads better:

```rust
#[amestate(path = "panels.left.visible")]
#[amestate(default = true, on_delete = Keep)]
pub left_panel_visible: bool,
```

Saying one of them twice is a compile error naming it: the second would win and
the first would look like it had been read.

#### Where a field goes

`path` names the place a field is stored at, and `rename_all` on the struct says
it once for all of them. A dot in a `path` is a level, so a field can be put
anywhere under the prefix and not only renamed:

<!-- shown: a struct that says where its fields go -->
```rust
#[amethystate(prefix = "net", rename_all = "camelCase")]
pub struct NetState {
    #[amestate(default = 8080u16)]
    pub listen_port: u16,

    #[amestate(path = "tls.enabled", default = false)]
    pub tls: bool,
}
```
<!-- /shown -->

That writes `net.listenPort` and `net.tls.enabled`.

#### A field whose paths sit at its holder's level

`flatten` on a `nested` field says its fields are stored here, without a segment
named after it:

<!-- shown: a nested struct whose fields sit at their holder's level -->
```rust
#[amethystate(prefix = "editor")]
pub struct Editor {
    #[amestate(nested, flatten)]
    pub window: Window,
}
```
<!-- /shown -->

That writes `editor.width`, not `editor.window.width`.

Two flattened children that spell a field the same way are a compile error
naming both, since each stores its fields at this level and the two would write
over each other. So is a flattened child whose field is spelled the same as one
written beside it.

**Both `path` and `flatten` decide where data lands, so changing either on
something already shipped is a migration.** The data stays where the old build
wrote it while the new build looks somewhere else, and what a person sees is
their settings gone back to defaults.

#### A field stored some other way

When a type's own encoding is not the one you want on disk, the field is written
and read through a pair of functions of your own:

```rust
#[amestate(with = since_the_epoch)]
pub opened: SystemTime,
```

`with = m` is `m::serialize`, which writes, and `m::deserialize`, which reads.
Either half can be named on its own, as `serialize_with` or `deserialize_with`.
Then the type does the other half, and the value goes to disk one way and comes
back another. That is usually a mistake, so write both unless you want exactly
that difference.

Nothing else touches the value. What lies at the path is what the first function
wrote, and only the second turns it back. So the type needs no encoding of its
own — and a field can hold a type from another crate, which has none to give.

The macro checks what it is given and names what it accepts, so a misspelling
is a compile error rather than an attribute that does nothing.

#### Attributes that are not this macro's

Everything else written on a field is carried onto the field the macro
generates, and onto its getter. A doc comment arrives where it was aimed;
`#[allow]` and `#[deprecated]` do what they say; and an attribute nobody here
understands is judged by whoever does — which is how `#[serde(..)]` becomes
rustc's own error about an attribute that is not in scope, rather than something
this macro has an opinion about.

`#[cfg]` is the exception, and it is refused. A field appears in a dozen places
in what is generated — the struct, its constructor, the snapshot, the schema
written to disk — and some of those are `const` arrays, where an element cannot
be conditional. Carried to the places that allow it and not the rest, a field
compiled out would be missing from the struct and present in the schema, and
nothing would say so. Put the whole struct behind the `cfg`, or keep the field
and decide at runtime what it holds.

Where a field is *stored* is serde's vocabulary rather than this one:
`#[serde(rename)]` names the place, `#[serde(rename_all)]` says it once for a
whole struct, and `#[serde(flatten)]` on a `nested` field puts its fields at
this level. What serde says that a struct of paths cannot honour is refused
where it is written. All of it, and what a leaf may say instead:
[What serde says here](/amethystate/state/serde/).

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

**Where nothing said, the store does.** A field's rule wins over its struct's,
and a struct's over the one the store was opened with — so an application says
once what it wants of everything that had no opinion, and every declaration
above stands untouched:
[Opening a store](/amethystate/store/opening/).

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

It takes the value by `&mut`, so a check that knows what the value should have
been may put it right and answer `Ok` instead. What it corrects is held in
memory: the field starts on the corrected value, the file still holds what it
held, and the next ordinary write settles it. Nothing says a repair happened -
a value that passes is a value that passes, and there is no third state between
*accepted* and *refused*.

<!-- shown: a check on a field, and the world it is judged against -->
```rust
fn a_size_that_renders(size: &mut u8, _cx: &CheckContext) -> Result<(), Invalid> {
    if *size >= 6 {
        Ok(())
    } else {
        Err(Invalid::new("a font size below 6 renders nothing"))
    }
}

fn a_theme_that_is_installed(theme: &mut String, cx: &CheckContext) -> Result<(), Invalid> {
    let installed = cx.require::<InstalledThemes>()?;

    if installed.0.contains(&theme.as_str()) {
        Ok(())
    } else {
        Err(Invalid::new(format!(
            "no theme called {theme} is installed"
        )))
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

A construction that fails hands back an `OpenStruct` - the five ways a
constructor can fail and no others - so a caller tells one refusal from another
by its variant rather than by reading the sentence:

<!-- shown: telling one refused open from another -->
```rust
match StrictUi::new_with(&store) {
    Ok(_) => {}
    Err(OpenStruct::Refused { at, said }) => eprintln!("{at} will not do: {said}"),
    Err(OpenStruct::WillNotRead { at, why }) => eprintln!("{at} is unreadable: {why}"),
    Err(OpenStruct::Taken(taken)) => {
        eprintln!("{} already holds {}", taken.held_by, taken.at)
    }
    Err(other) => return Err(other.into()),
}
```
<!-- /shown -->

Under `UseDefault` nothing fails, and the same place and reason arrive through
the field instead - as a `Disagreement`, which is what the field and the store
do not agree about rather than a failure of the asking:

<!-- shown: asking a field what the store disagrees with -->
```rust
let held = match ui.font_size().try_get() {
    Ok(size) => size,
    Err(no) => {
        match no.reason {
            Reason::Refused(said) => eprintln!("running on the default: {said}"),
            Reason::WillNotRead(said) => eprintln!("{} will not decode: {said}", no.at),
            Reason::Occupied(said) => eprintln!("{} was already taken: {said}", no.at),
            _ => eprintln!("{} is not what the store has", no.at),
        }

        ui.font_size().get()
    }
};
```
<!-- /shown -->

`Reason::Refused` is there only when a declared check turned the value down;
bytes that would not decode arrive as `WillNotRead` with the codec's own
sentence, and a field that never got to write its default because the store
already held something arrives as `Occupied`. A field whose store has closed
answers `Reason::Closed`, which is none of those three and says the value is the
last one it heard.

`Reason` is `#[non_exhaustive]` - it is a list of what a field can be found
disagreeing about, and it grows - so a `match` over it keeps a `_` arm. The
sets do not, and [Errors](/amethystate/concepts/errors/) is why.

### A rule about the struct, not the value

A field's check sees one value. It cannot see its siblings - fields are built
one at a time, and the others do not exist yet - so an invariant between two of
them goes on the struct, which is handed the whole thing once every field is
built.

<!-- shown: a check on the struct, for what one field cannot see -->
```rust
fn the_window_can_be_drawn(
    window: &AmeData<LenientWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new("the smallest window is wider than the largest").at(&["min", "max"]))
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

What arrives is `AmeData<LenientWindow>` - the plain-data twin of the struct,
the same one a migration step is handed - so the check reads `window.min` and
not `window.min().get()`. That is deliberate: a rule about a relationship is
about values, and a struct's mode decides what the struct itself is made of
while the data twin is the same either way.

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
| `load_with` | runs | runs |
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
log, and that is the only place it is said. A refused *struct* under
`UseDefault` keeps what was stored, for the same reason it does anywhere else,
and also only says so in the log. `Refuse` - the default - fails the load
instead, which is the answer to reach for when a loaded struct has to be
trustworthy.

<!-- shown: the same rule, on a struct that is loaded rather than watched -->
```rust
#[amethystate(prefix = "kept_window", mode = "persistent", check = the_kept_window_can_be_drawn)]
pub struct KeptWindow {
    #[amestate(default = 400u32)]
    pub min: u32,

    #[amestate(default = 1600u32)]
    pub max: u32,
}

fn the_kept_window_can_be_drawn(
    window: &AmeData<KeptWindow>,
    _cx: &CheckContext,
) -> Result<(), Invalid> {
    if window.min <= window.max {
        Ok(())
    } else {
        Err(Invalid::new(
            "the smallest window is wider than the largest",
        ))
    }
}
```
<!-- /shown -->

Three fields will not take a check at all, and the macro says so while it
compiles:

- **A `volatile` field.** Nothing arrives at it from the store, so a check
  would never be asked anything. A rule about a value this process writes and
  never stores is an
  [interceptor](/amethystate/concepts/subscriptions/)'s job.
- **A `nested` field.** A field's check judges one value that arrived from the
  store at one path. A nested struct is not a value but a subtree of paths:
  nothing arrives at it, so there is nothing to call a check with. The rule you
  wanted goes on the nested struct itself, where
  `#[amethystate(check = ..)]` is handed all of its fields at once, once it is
  built.
- **A map.** A check is declared once against one value, and a map's entries
  are data - they come and go while the program runs, and there is no declared
  path to hang a rule on. What a map does with an entry it cannot read is
  [Kv](/amethystate/primitives/kv/)'s subject, not this one.

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