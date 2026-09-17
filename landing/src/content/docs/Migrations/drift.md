---
title: Schema drift
sidebar:
  order: 23
---

Rename a field, move it under a different level, turn a value into a map — and leave the version alone — and no step runs, because none was asked for. `amethystate` notices anyway.

## What is compared

Where things sit. Each struct writes down the places it owns beside the data, and on a startup that finds the stored version equal to the declared one, the recorded places are compared against the declared ones.

A *place* is a path a value or a map level actually stands at. A nested struct is not one — nothing is stored at it — so a wrapper that gains or loses its segment is reported as what it does, which is move everything beneath it:

```text
  declared                    owned
  ------------------------------------------------
  ui            nested        -           a nested struct is not a place
    theme       field         ui.theme

  ui            flattened     -           and lends no segment either
    theme       field         theme
```

Paths are compared, not field names. One layout can be spelled two ways — a `theme` field inside a nested `ui` and a field with `path = "ui.theme"` sit in the same place under different names — and two fields called `theme` in different structs have nothing to do with each other. Only the path a field lands at counts.

## The verdicts

| What moved | Verdict | Why |
|---|---|---|
| a place that was declared and is not now | **breaks** | what was written there is under no declaration any more, and somebody has to say what happens to it |
| the same place holding a different kind of thing — one value where a level of entries stood, or the reverse | **breaks** | the two are not the same thing on disk |
| a place nothing declared before | **look at the ground** | nothing at all over empty ground, an annexation over occupied ground |
| a place that may now hold nothing, or may no longer | **harmless** | the declaration still owns it |

Drift is raised when at least one difference breaks. The rest are carried with it, because a change is easier to recognise whole than through the one part of it that failed.

A rename reads as a release beside a claim, and is not to be told from the two happening at once — which is what [`#[rename(old => new)]`](/amethystate/migrations/defining-steps/) exists to say.

## It does not block startup, and it does not go away

The store opens, the application runs. What is reported is reported again on the next startup, and the one after: drift nobody answered leaves the recorded shape where it is, so the comparison still has something to compare against.

Two things stop it. Bump the version and write a step for what moved, which is the answer when the change was meant. Or put the released places back, which is the answer when it was not.

## Reading it

`MigrationReport::has_drift` answers whether there was any. What it was is on `component.nagging` — one `NaggingRecord` per line, carrying the prefix and the `id`, every place that moved with its verdict, and a by-name diff of the fields added and removed.

```rust
let (store, report) = StoreBuilder::new("./app.redb").migrate()?;

for record in report.components.iter().flat_map(|c| &c.nagging) {
    for moved in &record.moved {
        println!("{:?} {moved}", moved.verdict());
    }
}
```

A place is named under its prefix there — `token`, not `app.token` — and the prefix is on the record beside it.

## Rendering it

Both `build` and `migrate` already write the report through `tracing`, so nothing has to be printed by hand. With the `diagnostics` feature on, drift is laid out the way a compiler lays out a warning instead of a line per field:

```toml
amethystate = { version = "0.21", features = ["diagnostics"] }
```

What follows is one struct edited and its `version` left alone. The build that wrote the data declared this:

```rust
#[amethystate(prefix = "app", version = 2)]
pub struct Settings {
    #[amestate(default = None)]
    pub host: Option<String>,

    #[amestate(default = {})]
    pub port: ReactiveMap<String, u16>,

    #[amestate(default = String::new())]
    pub token: String,
}
```

and the build reading it declares this:

```rust
#[amethystate(prefix = "app", version = 2)]
pub struct Settings {
    #[amestate(default = "localhost".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}
```

`token` is gone, `port` stopped being a map, `host` stopped being optional, and `version` is still `2`:

```
amethystate::drift

  ⚠ `app` holds a shape this build does not declare
  help: raise the struct's `version` and write a step for it, or declare the
        released places again

Warning:
  ⚠ `app.token` was declared before and is not now
  help: what an earlier build wrote there is under no declaration now

Warning:
  ⚠ `app.port` was written as Map and is declared as Field
  help: one value and a level of entries are not the same thing on disk

Advice:
  ☞ `app.host` may no longer hold nothing
  help: what was written there as nothing reads back as the declared default
```

What breaks is read first, then what is worth a look, then the rest — the order the record carries them in is the order the comparison walked, which is true to the code and no use to somebody deciding whether to act.

The feature is off by default, and it is a presentation rather than a check: the comparison runs either way and `has_drift` answers either way. Turn it on where a person reads the output — a debug build, a development profile — and `MigrationReport::drift` hands you the same diagnostics to render yourself.

## What it does not compare

Types. A leaf's type is yours, and a value that stops decoding is answered where it is read rather than here — by `on_unreadable`, and listed by `disagreements()`. See [Errors](/amethystate/concepts/errors/).

So `u16` becoming `u32` is not drift: the place is the same, and what sits there is the codec's business. If the new type will not read the old value, you find out at the read rather than at startup.

## What has to be running for any of this

Nothing. Drift is looked at on every open, `build` and [`migrate`](/amethystate/store/opening/) alike, and a binary that declares structs and contains no `#[migrate]` at all is checked the same way.

Running the *steps* is the part that needs `migrate`. A store opened with `build` runs none, so a binary full of `#[migrate]` opened that way migrates nothing — and then reports the drift, which is the shape of that mistake.
