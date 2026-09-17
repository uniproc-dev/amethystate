---
title: Migrating from a custom solution
sidebar:
  order: 24
---

What there is to do turns on one question: are the settings you have now a
TOML, JSON or RON file with your struct's fields at its top level?

## The file is your struct

A struct serde loads from a file and saves back:

<!-- shown: the struct serde was writing -->
```rust
#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    pub window: Window,
}
```
<!-- /shown -->

is declared again, with the same fields, at the top of the store:

<!-- shown: the same fields at the top of the store -->
```rust
#[amethystate(mode = "persistent", as_root)]
pub struct Settings {
    #[amestate(default = "light".to_string())]
    pub theme: String,

    #[amestate(default = Window { width: 800, height: 600 })]
    pub window: Window,
}
```
<!-- /shown -->

Turn on the engine for the file's format:

```toml
amethystate = { version = "0.21", features = ["toml"] }
```

and open the file the application already has:

<!-- shown: opening the file that was there -->
```rust
let store = StoreBuilder::new(file).backend(Backend::Toml).build()?;
let settings = Settings::load_with(&store)?;
```
<!-- /shown -->

Nothing is converted. `as_root` puts the fields where serde put them, so
`load_with` reads what the file holds, and a field the file lacks takes its
default. Beside the file the store keeps a `.meta` of its own:
[What is on disk](/amethystate/store/files/).

After a save, the old struct still reads a TOML or JSON file, so an older build
of the application opens it too. RON is different: the store writes it back as a
map, `{ "theme": .. }`, and serde reads a struct only from `(theme: ..)`.

Where the old struct renamed a field through serde, the same thing has its own
spelling here: [What serde says here](/amethystate/state/serde/).

## Everything else

A database of your own, a format other than those three, a file laid out
differently from the struct: there is nothing the store can open in place. The
values move in once, by your code, right after the store opens. Read the old
source with whatever reads it today, write the values through the struct, and
remove the old source:

<!-- shown: moving the values in once -->
```rust
let mut settings = Settings::load_with(&store)?;

if let Some(old) = read_old_settings(&old_file)? {
    settings.mutate(|now| {
        now.theme = old.theme;
        now.window = old.window;
    })?;
    std::fs::remove_file(&old_file)?;
}
```
<!-- /shown -->

`mutate` returns once the values are on disk, so the old file goes only after
they are safe. A crash between the two leaves both, and the next start moves the
same values again.

A migration step cannot do this, because it would have nothing to run over:
steps rewrite what a store already holds, and a new store runs none. See
[What happens on a clean install](/amethystate/migrations/overview/#what-happens-on-a-clean-install).
