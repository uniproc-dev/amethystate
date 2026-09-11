# What a text store loses when a write is interrupted

**Status: closed.** Two of the five were the library's to fix and are fixed. The
other three are one fact about editable files, ruled out of scope and written
down in the book instead of carried as ignored tests.

The pins live in `crates/main/amethystate/tests/torn_recovery.rs` and all of
them run.

## Fixed

**A killed write leaks a full copy of the document.** The temporary is written
and `sync_all`ed before the replace enters its retry loop, so a kill in the loop
leaves it on disk, and nothing collected it: one per crash, each a complete copy
of the store's contents. A temporary is now named
`<file>.<nonce:08x><sig:08x>.tmp`, where `sig` is `xxh3_64(nonce ++
WRITTEN_BY_US) as u32`, and `clean_backups` sweeps every one beside both files
that carries a mark verifying against its own nonce. The mark rules out
accidents - somebody else's `.tmp`, a name that happens to collide - and is not
meant to rule out anybody deliberate.

Pinned by `a_write_killed_between_the_temporary_and_the_target_leaves_no_temporary_behind`.

**One buffered write erases what another store committed.** Store A holds one
unflushed write; store B opens the same path, writes, flushes, drops; A's
`save_now()` rewrites the whole document from memory and B's key is gone. The
hole was `Standoff::holding`: with nothing recorded in `left`, a store read "I
have not written yet" as "the file is as I left it" and laid its document over
what it had never read. Nothing recorded is now the answer *yes, somebody may
have written* - a store that has never written knows nothing about how the file
stands.

Pinned by `one_buffered_write_does_not_erase_what_another_store_committed`.

**An open refused after reading left its copies behind.** The copies were taken
at the end of `load_and_back_up`, and `settle_for_codec` runs later still: a
store recording a deciding fact this build has no name for was refused with a
`.bak` beside it, which the next open reads as an unfinished run and recovers
onto. `StoreFiles::take_backups` now stands on its own and is called
immediately before `run_migrations`, so every way an open can still be refused
comes first.

Pinned by `an_open_refused_by_the_format_record_leaves_nothing_of_its_own_behind`.

## Ruled out of scope

Three findings were one thing seen from three sides: **from outside, a document
is whatever it parses as.**

- A torn TOML write parses as valid and wrong. Cut `[torn]\na = 11\nb = 22\nc =
  33\n` at every byte offset and 9 of 28 cuts open, three of them returning a
  *different number* for a committed key - `11` as `1`, `22` as `2`, `33` as
  `3`. json and ron refuse at every offset, so this is TOML's grammar.
- A torn write that still parses eats the backup that would repair it. The stump
  walks the success path, so `clean_backups` deletes the copy that held the
  whole document, unread.
- A leftover copy is trusted because it exists. `recover_from_backup` compares
  nothing before copying it over the data - not content, not age. mtime is not
  an answer; git carries its whole "racily clean" mechanism rather than trust
  one.

Telling a whole file from a stump means the store writing its own length or
checksum into it, and then it is no longer a file a person can edit - which is
the reason the text engines exist. The ruling is that a `.bak` is a fault report
rather than a restore point, and that an application which cannot afford this
wants redb or SQLite, whose write-ahead logs make a cut-off write either whole
or absent.

Written up in `landing/src/content/docs/{,ru/}Store/files.md` under *What the
copy cannot promise*.

## Still worth knowing

`restore_from_backups` does not fire for a migration step that returns an error.
`MigrationEngine::run` records the component as `Failed` and returns `Ok`, and
`TextProvider::atomic` has already rolled the in-memory documents back, so
`build()` opens and only logs. The on-disk copy is reached when the engine
itself errors - a prefix that will not parse as a path, or `ensure_snapshots`
failing - and when a process dies between the copy being taken and the open
finishing.
