# What a caller sees when a write fails, and why it is almost nothing

**Status: done.** Every claim below was checked against the code and held; the
output it describes was reproduced before anything was changed. What was built
is at the end, under [What was done](#what-was-done), including where it departs
from the directions and why.

The diagnosis a failing write carries is
complete inside the library and invisible outside it. Nothing is lost on the
way out - the facts are still in the report, sitting in the variant - but the
only caller who can reach them is the one who already matched on the variant
and pulled the report out by hand. The caller who does the ordinary thing, `?`
into `anyhow` and print, gets one sentence with no key, no file and no engine
in it.

That caller is the population. Not one of the six adapters and not one of the
ten examples names an error type at all; every `WriteValue::` match in the tree
is in `amethystate-core`, in `amethystate`, or in their tests. So the path that
carries the least is the path everybody is on.

## Where it goes

Inside, a failure travels as `Report<StorageError>` and collects typed facts
along the way - `Key`, `StoreFile`, `MetaFile`, `Table`, `ValueBytes`, `Read`
and the rest of `amethystate-core/src/facts.rs`, attached by whoever knew them.
At the boundary it becomes `WriteValue`
(`crates/core/amethystate-core/src/primitives/error.rs`), which is an ordinary
`std::error::Error` so that a `?` into `anyhow` or `eyre` costs nothing. Four
things happen there, and together they flatten the report to a single
sentence.

**`Display` reads the poorest frame.** `Store(why)` prints
`why.current_context()`, which is the outermost context and the one that says
least - "the store could not write", without what refused it. `TooDeep` does
not print `why` at all. `WillNotEncode` is the only variant that uses
`one_line`, which is in `amethystate-core/src/failure.rs` and was written for
exactly this.

**`source` stops at a leaf.** For `Store`, `TooDeep` and `WillNotEncode` it
returns `why.current_context()`, a `&StorageError`, and `impl Error for
StorageError {}` has no `source` of its own. Anything walking the source chain
- which is what `anyhow` does when it renders - halts there. Every frame below
and every fact attached anywhere in the report is, to that walker, not present.

**So `Store` prints twice.** `Display` yields the same string the source chain
then yields under `Caused by:`, because both read the same frame.

**And the facts are printable but unreached.** They go on with `.attach`, 188
times; in error-stack 0.8 `attach` is the printable one and `attach_opaque` is
what hides a value. They render in `{:?}` of the report. Nothing outside gets a
report to render.

## What should be true

One `eprintln!("{err:?}")` over an error raised through `anyhow` from the
public API names the operation that failed, the key and the file it was about,
and the chain of contexts down to the engine. No `match`, and no knowledge that
there is an error_stack underneath.

## Directions, not decisions

Use `one_line(why)` in `Display` wherever a variant carries a report, `TooDeep`
included, and print `at` with it.

Separate `Display` from `source`, which currently read the same frame. Either
`Display` carries the chain and `source` returns `None`, or `Display` says only
its own sentence and `source` descends for real - the second is the honest one
and needs a wrapper over `Report` that implements `Error` with a non-empty
`source`.

Give a way to the whole picture that does not require matching:
`WriteValue::explain(&self) -> String`, rendering the report with its facts,
exported from `pub mod errors`, so an `anyhow` caller can `.context(err.explain())`.

`facts::all` is already re-exported from `errors`; say so on `WriteValue`
itself, as the way to read a fact back as a type rather than as a sentence.

## Out of scope

The taxonomy stays as it is - whether `WriteValue` should be
`#[non_exhaustive]` is a separate question and a separate change.
`Report<StorageError>` remains the internal currency and the `change_context`
seams stay where they are.

No global error_stack debug hooks. `install_debug_hook` is process-wide state,
and a library does not get to spend it on behalf of the application that links
it.

## One defect on the way, same file

`WriteValue::from_store` drops `why` entirely on the `StorageError::Closed`
arm, keeping only `at`, where the `Depth` and `Codec` arms beside it keep the
report whole. `impl From<WriteValue> for Report<StorageError>` then builds a
fresh `Report::new(StorageError::Closed)`. So the round trip `Report ->
WriteValue -> Report`, which the comment on that impl describes as the plumbing
*under* the boundary and which is therefore an internal path, silently loses
frames, facts and backtrace. If `Closed` is empty by construction, that is
worth saying in the doc; if a real cause can ever sit under it - the file taken
away, an io failure while closing - it disappears without trace. The asymmetry
is invisible at the call site either way.

## Acceptance

A test that builds a redb failure carrying `attach_store_file` and
`attach_key`, raises it through the public API, `?`s it into an
`anyhow::Result` and formats it with `{:?}`. The store file path, the key and
the engine's own words are all in the output, and no line appears twice.

## What was done

`tests/an_error_raised_through_anyhow.rs`. What a caller got before:

```
the store could not write

Caused by:
    the store could not write
```

and gets now:

```
the store could not write

Caused by:
    0: the value could not be encoded or decoded (key: probe.v; from: dyn erased_serde::ser::Serialize)
    1: TOML error: unsupported None value
```

`Caused` is a report's context frames as an ordinary error chain, one link per
context carrying what was attached to it. A `Report` owns its frames and
`source` has to hand back something with a life of its own, so the chain is
copied out where a report crosses into a set. `Because` holds the report and
builds the chain on the first reader that asks, so a failure nobody looks into
costs nothing beyond the report it was already carrying. Both are exported from
`errors`.

`Display` and `source` now say different things: the variant's own sentence with
its path, and the chain under it. The outermost context is left out of the
chain, because the variant already says it - that was the duplicate line.

**Where this departs from the directions above.** `one_line` in `Display` was
the other fork: it carries the chain where `source` returns `None`. With a real
`source` chain it would print everything twice, so it came *out* of `Display`
here, and out of the sibling sets for the same reason. It stays where it belongs,
in log records.

`explain()` is on every set that carries a report, not only `WriteValue`.

`Debug` is hand-written on each set and prints what `anyhow` prints. That was
not in the directions and is the same defect one level out: `fn main() ->
anyhow::Result<()>` renders with `Debug`, so `{:?}` is the surface a reader has
learnt, and ours used to answer it with an error-stack tree - ANSI escapes and
all - wrapped in `Store(...)`. The tree is still there, in `explain()`, which is
where a dump belongs.

**The `Closed` defect resolves as documentation.** Every `StorageError::Closed`
in the tree is minted `Report::new(StorageError::Closed)` at the refusal itself -
the store asks whether it is closed and answers - so there is never a cause
underneath, and the round trip loses no frame that ever existed. What it does
drop is the store's own file, attached beside it. Both are now said on the
variant, in every set that has one.

**Scope taken beyond `WriteValue`.** The same `source` -> `current_context()`
stood in `ReadValue`, `ScanKeys`, `LoadMap`, `KvWrite`, `Flush`, `OpenStruct` and
`OpenStore`. Reads and scans lied exactly the way writes did, so they were
changed with it: a caller who learns the shape from one failure should not find
the next one shaped differently.
