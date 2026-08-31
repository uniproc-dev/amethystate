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
    #[amestate(key = "panels.left.visible", default = true)]
    pub left_panel_visible: bool,
}

#[amethystate(prefix = "ui.panels", version = 1)]
pub struct Panels {
    #[amestate(key = "left.visible", default = true)]
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

let refused = Panels::new_with(&store)
    .expect_err("`ui.panels.left.visible` is spelled by both of them");

assert_eq!(refused.current_context(), &StorageError::Claimed);

for claim in all::<Claimed, _>(&refused) {
    println!("{} claims {}", claim.by, claim.path);
}
```
<!-- /shown -->

The failure is `StorageError::Claimed`, and the report carries a `Claimed` fact
for each side: the path, and the schema that wanted it. Reading them out of the
report is by type rather than by searching the rendered text.

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

<!-- shown: two structs that do not meet -->
```rust
let _ui = Ui::new_with(&store)?;
let _editor = Editor::new_with(&store)?;
```
<!-- /shown -->

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
let owner = store.owners().declared_by(&field);

println!("{owner:?}");
```
<!-- /shown -->

The lookup is by exact path, and the paths recorded are the ones actually
claimed - a field's own path, not the prefix it sits under. So `Ui` above is
found at `ui.panels.left.visible` and not at `ui`.

The name it answers with is the schema's, which is what the refusal prints.
