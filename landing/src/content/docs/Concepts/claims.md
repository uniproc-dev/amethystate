---
title: Who owns which place
sidebar:
  order: 16
---

A struct's `prefix` says where it sits, and its fields are paths under that
prefix. Nothing stops two structs from spelling the same place - a dotted key
reaches as deep as a dotted prefix does:

<!-- shown: two structs that want the same place -->
```rust
#[amethystate(prefix = "ui", version = 1)]
pub struct Ui {
    #[amestate(path = "panels.left.visible", default = true)]
    pub left_panel_visible: bool,
}

#[amethystate(prefix = "ui.panels", version = 1)]
pub struct Panels {
    #[amestate(path = "left.visible", default = true)]
    pub left_visible: bool,
}
```
<!-- /shown -->

Both of those address `ui.panels.left.visible`. One reaches it as a key with
dots in it, the other as a prefix one level down, and neither declaration
mentions the other.

## The second one is refused

The store keeps a table of what has been claimed, and the second struct to open
over the same place fails to build:

<!-- shown: what the refusal looks like -->
```rust
let _ui = Ui::new_with(&store)?;

let refused =
    Panels::new_with(&store).expect_err("`ui.panels.left.visible` is spelled by both of them");

let OpenStruct::Taken(taken) = &refused else {
    panic!("{refused}")
};

let Taken {
    at,
    wanted_by,
    held_at,
    held_by,
} = &**taken;

println!("{wanted_by} wants {at}, which {held_by} already holds at {held_at}");
```
<!-- /shown -->

The failure is `OpenStruct::Taken`, and it names both sides at both paths:
`at` and `wanted_by` for the declaration that was turned down, `held_at` and
`held_by` for the one standing. The two paths differ whenever one declaration
reaches the other through an ancestor - `root.b` holding `root.b.x` - and the
pair is the whole diagnosis.

Nothing has to be searched out of a rendered message, and nothing can be missed
by a `match` that had a `_` arm: the variant is one of five a constructor can
answer with, and [Errors](/amethystate/concepts/errors/) has the rest.

Refusing is the cheaper end of the same collision. Two structs writing the same
path is one struct silently overwriting the other's value on every save, and
finding that later means reading two declarations that never mention each other.
A refusal happens at the call that opens the struct, and names both.

## What counts as overlapping

Two places overlap when one holds the other. That is the whole rule, and it is
symmetric: it does not matter which is declared first, only that one subtree
starts the other.

| the two places | overlap |
| --- | --- |
| `ui` and `ui.panels` | yes - the second is inside the first |
| `ui.panels` and `ui` | yes - the same pair, the other way round |
| `ui.panels` and `ui.status` | no |
| `ui` and `ui!x` | no - `ui!x` is a different name that happens to start with the same letters |

A prefix may not land on another struct's field, either. A field is a place
like any other, so a struct whose prefix is `root.b` is refused when some other
struct already declares a `b` field under `root`.

Places that do not meet are left alone, however close they sit:

<!-- shown: a struct that sits right beside one and still opens -->
```rust
#[amethystate(prefix = "ui.panels.right", version = 1)]
pub struct RightPanel {
    #[amestate(default = true)]
    pub visible: bool,
}
```
<!-- /shown -->

That is as close as two can get: `Ui` holds `ui.panels.left.visible` and
`RightPanel` holds `ui.panels.right.visible`. They part on the last level they
share, neither path starts the other, and both open in the same store. `Panels`,
which aimed at `ui.panels`, was refused because its place started `Ui`'s.

## What each kind of place owns

Three laws, and everything above follows from them.

**A leaf owns its path and everything under it, and should have nothing under
it.** The subtree is not for holding things; owning it is what stops somebody
nesting under a scalar.

**A map owns its path and everything under it, and uses one level of it.** Its
keys are made while the program runs, so nobody can declare them - and if the
space beneath it were open, every entry it has not written yet would be
contestable.

**A struct owns nothing at its prefix. Its fields do.** All of its paths are
declared, so what is not declared is not its business, and the level it sits on
stays open.

That last one is the asymmetry worth holding on to. One declaration:

<!-- shown: a map and a leaf under one prefix -->
```rust
#[amethystate(prefix = "ui")]
pub struct Panels {
    #[amestate(default = {})]
    pub open: ReactiveMap<String, u32>,

    #[amestate(default = "dark".to_string())]
    pub theme: String,
}
```
<!-- /shown -->

and the space it leaves behind:

```
ui                       ·  nobody's - a prefix is not claimed
├─ theme                 ▪  Panels declared it; nothing may go under it
├─ open                  ▪  Panels declared it, as a map
│  ├─ left               ▪    an entry, named while the program ran
│  └─ right              ▪    an entry
└─ myplugin              ·  nobody's - an extension may write here
```

`ui.myplugin` is free because `ui` was never claimed. `ui.open.myplugin` is not,
because `ui.open` was:

```
under a struct's prefix            under a map's prefix
  ui                                 ui.open
  ├─ theme       declared            ├─ left       an entry
  ├─ open        declared            ├─ right      an entry
  └─ myplugin    allowed             └─ myplugin   refused
```

A map owning the whole subtree and using one level of it are two different
statements, and both are enforced. `ui.open.left` is an entry; `ui.open.left.px`
is inside the map's space and is not an entry, so it is refused as a place a map
does not reach rather than as a place nobody owns.

**A value's own depth is not the store's.** An entry holds whatever its type is,
encoded, at one path - so a structure nested five deep is one key and not five
levels:

```
tree.roots.one           ▪  one key, one stored value
                            { name: "a", children: [ { … }, { … } ] }
                            the depth in here belongs to the codec
```

## The claim outlives the handle

Dropping the struct does not release its place. The table belongs to the store
and lasts as long as it does.

That is deliberate: a claim released on drop would make the refusal depend on
when a value happened to go out of scope, so the same program would open
cleanly or fail depending on where a `let` binding ended.

## Asking who has a place

<!-- shown: asking who claimed a place -->
```rust
let field = StorePath::parse_joined("ui.panels.left.visible")?;
let owner = store.places().declared_by(&field);

println!("{owner:?}");
```
<!-- /shown -->

The lookup is by exact path, and the paths recorded are the ones actually
claimed - a field's own path, not the prefix it sits under. So `Ui` above is
found at `ui.panels.left.visible` and not at `ui`.

The name it answers with is the schema's, which is what the refusal prints.
