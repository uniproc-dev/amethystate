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
TRYBUILD=overwrite cargo test -p amethystate --test compile_tests
```

Regenerate them only after reading the diff — they are the record of the errors
users see, so a change there is a change to the public surface.

What the macro expands *into* is not pinned. Every test file in the suite
declares a struct through it, so a change that matters shows up as a failure
somewhere that means something; a snapshot of the emitted tokens only records
that the tokens changed.

The trybuild goldens quote rustc diagnostics, which qualify type paths
differently depending on what else is in scope, so they are checked in the
single-backend configuration only.

## Releasing

The version lives once, in `[workspace.package]`, and the internal path
dependencies pin it too — both move together. Pushing a `v*` tag is what
publishes; CI must be green first, because a published version cannot be taken
back.
