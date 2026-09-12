# Working on amethystate

Persistent reactive state for Rust GUI apps. This file is the map; it does not
repeat what the sources or the book already say.

## Where the documentation lives

**Start with the book, not the sources.** It carries the overview that the API
reference cannot: what the pieces are for, how they fit, and why the design is
shaped the way it is. Read it from the repository — it is markdown, and the
checkout is already here:

[`landing/src/content/docs/introduction.md`](https://github.com/uniproc-dev/amethystate/blob/master/landing/src/content/docs/introduction.md)

Everything else sits beside it under `landing/src/content/docs/`:

| Section | Read it for |
| --- | --- |
| `Getting-started/` | installation, first store, the smallest working app |
| `Concepts/` | fields and subscriptions, reactive cells, `Kv`, durability, observability |
| `Migrations/` | defining steps, custom and manual migrations |
| `Integrations/` | one page per GUI framework, plus an overview of the execution models |

`Integrations/overview.md` is the fastest way to understand why each adapter is
shaped differently — the frameworks disagree about who owns state, and that
disagreement drives everything else.

Per-item reference is rustdoc: <https://docs.rs/amethystate>, or `cargo doc
--open` for the working tree.

## Layout

```
crates/core/amethystate-core      backend-agnostic primitives (Signal, FieldCore, map ops)
crates/core/amethystate-macros-core   parsing types shared by the macro crates
crates/main/amethystate           the crate users depend on: store, backends, reactive types
crates/main/amethystate-macros    #[amethystate], #[migrate], derive(AmeType)
crates/main/amethystate-arena     Copy handles for frameworks that want them
crates/adapters/*                 one crate per GUI framework
crates/tauri/*                    plugin and TypeScript binding generation
landing/                          the book (Astro + Starlight)
examples/                         runnable apps, one per framework
```

## Checks

`ci.ps1` in the root is the authoritative set. Run it rather than assembling
commands by hand:

```powershell
./ci.ps1
```

It is stricter than the GitHub workflow: fmt, then clippy with `-D warnings`
and the test suite over **each backend separately** — redb, json, toml, ron,
sqlite — and once more with `--all-features`. Running only the default features
hides plenty; a change can pass under one engine and fail under another.

It pins `INSTA_UPDATE=no`, so snapshot tests report a mismatch instead of
quietly rewriting the snapshot.

One difference from the GitHub workflow worth knowing: that one excludes
`amethystate-gpui`, which needs a toolchain the hosted runners lack.

`sqlite` compiles SQLite in from source, so building it needs a C toolchain.

## Working on the book

The book is two locales over the same tree: English at
`landing/src/content/docs/`, Russian under `ru/` beside it. A page added to one
belongs in the other, and the sidebar is generated from the directory, so the
file landing there is what puts it in the nav.

**Code in the book is not written in the book.** A page asks for a block by
name and `cargo xtask book` fills it:

```markdown
<!-- shown: a struct that says where its fields go -->
<!-- /shown -->
```

What lands between the two comes from a test that marked the same name:

```rust
//@show a struct that says where its fields go
...
//@show-end
```

Marks are read from `crates/main/amethystate/tests/*.rs` — the top level only,
not `expand/` or `fails/`. Both locales draw from one pool of names, so the
same name gives byte-identical code on both pages: the prose is translated, the
code cannot drift apart.

`<!-- printed: <field> from <test> -->` is the other half. That one runs the
named test and captures what it prints, so a page quoting an error message
quotes the message the code actually produces.

```bash
cargo xtask book           # fill every block
cargo xtask book --check   # fail instead of writing, for CI
```

It fails rather than writing when a page asks for a name no test marks, or for
output no run produced. It also checks the identifiers the prose names against
the sources, so a method renamed in the code is caught in the pages that still
name the old one.

`cargo xtask docs` is separate: it regenerates `Limitations/` wholesale from the
probe tests that measure each limit. Those pages carry a header saying so — edit
the probe, not the page.

**So the workflow for changing an example is: change the test, run the test,
run `cargo xtask book`.** Editing the fenced block in the page is wasted work;
the next run overwrites it.

## Documentation examples

Most rustdoc examples are real doctests with assertions, and they build their
store through `amethystate_core::test_utils::TempPath`, which cleans up after
itself.

Examples involving `#[amethystate]` or `#[migrate]` are marked `ignore`, and
this is not laziness. The macro resolves the crate to `crate`, and a doctest
compiles as a separate crate where that means the doctest itself - so the
generated code does not resolve. Anything reachable without the macro uses
`store::field_with_path` or `Kv` instead and stays runnable.

None of that belongs in the rustdoc - a reader has no use for why our test
harness cannot run something. The mapping lives here instead: the migration
examples are lifted from `tests/migration.rs` and `tests/migration_builder.rs`,
and both sides need updating together.

## Tests worth knowing about

`tests/fails/` holds `trybuild` cases with the compiler output each one is
expected to produce, and `tests/expand/` holds declarations that must keep
compiling. Both run from `compile_tests.rs`. The `.stderr` files are
regenerated rather than edited by hand:

```bash
TRYBUILD=overwrite cargo test -p amethystate --all-features --features golden --test compile_tests
```

Regenerate them only after reading the diff — they are the record of the errors
users see, so a change there is a change to the public surface.

What the macro expands *into* is not pinned. Every test file in the suite
declares a struct through it, so a change that matters shows up as a failure
somewhere that means something; a snapshot of the emitted tokens only records
that the tokens changed.

The goldens are behind the `golden` feature, because trybuild compiles a crate
per case and that one test costs more than the rest of the suite together. They
run with every engine in scope, and have to: this crate's dev-dependency on
itself names them all, so a test target is built with the union whatever the
command line says.

## Releasing

The version lives once, in `[workspace.package]`, and the internal path
dependencies pin it too — both move together. Pushing a `v*` tag is what
publishes; CI must be green first, because a published version cannot be taken
back.
