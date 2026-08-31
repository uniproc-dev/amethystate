---
title: What the engines will not do
sidebar:
  label: Overview
  order: 21
---

The rest of the book says what works. This section is the other half: what a
store cannot hold, cannot tell apart, or hands back changed - and which engine
it happens on.

Almost none of it is this library's doing. A store keeps values in a document,
and a document format decides what a value may be, what may name it, and what
survives a round trip. TOML has no null; JSON names a member with a string; a
key with nothing in it is a key nothing can address. Those are decisions made
before `amethystate` sees the value, and no setting here changes them.

That is why the section exists separately. A limit the library imposes is
configuration and lives in [What a store refuses to
hold](/amethystate/store/limits/). A limit the format imposes is a fact about
the engine you picked, and the only useful thing to say about it is what
actually happens.

## Every page here is a run

None of these pages is written from reading the code. Each one is a test in
`crates/main/amethystate/tests`, and `cargo xtask docs` runs it and builds the
page out of what it printed: the code that was executed, then the documents the
engines wrote, verbatim. Editing a page by hand is pointless - the next run
replaces it.

So a page cannot describe behaviour the code stopped having. If a claim here is
wrong, the run that produced it was wrong, and it fails in the same place every
other test does.

## What earns a page

Something has to be genuinely lost or refused. A value spelled differently in
one engine's file and read back as the type that went in belongs somewhere else:
a page here is read as a warning, and every page here should be one.

Each page carries `Tags:` at the top - `toml`, `keys`, `codec`, `choosing an
engine` - and the search matches on those words, so a question shaped like
*what does TOML do about* finds the pages about TOML.

## Reading a page

Start at the bottom. The prose says what the section is about; the run says what
happened. Where the two disagree the run is right, and the prose is a bug.

If you are choosing an engine, read the runs across all of them and pick the one
whose losses you can live with. If you have already chosen, only your column
matters.
