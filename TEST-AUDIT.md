# What the tests do not test

Four passes over the suite, one per area, each asked the same question: if the
behaviour under test were broken, would this test say so?

The standing criterion, and the reason most entries here exist:

> A test is only as good as what it would catch if the code were wrong. Where an
> entry says a test cannot fail, it names the line to change - **mutate it and
> watch the test stay green.** An entry without that is an opinion, not a
> finding.

What was expected going in was loose error matching: `is_err()` on a
`Report<C>` whose context and attachments say exactly what happened. That is
here, and it is real, but it is not the largest part. The largest part is
**tests that never run, or that cannot fail**. A rough count: 52 `is_err()` in
`tests/` and 13 in `src/`, against roughly two dozen tests below that assert
nothing the code could contradict.

Marked `[verified]` where the claim was checked against the source or a run
rather than taken from a report.

## Fixed so far

The entries below stay as written, because the reasoning is what makes each one
checkable. This is what has been done about them.

- **The scan boundary now has named tests.**
  `tests/scan_stops_at_the_level_boundary.rs` - every engine by name, the
  sibling characters listed rather than generated, and a flush before the scan
  so it reaches the committed path. redb, json, toml and ron pass; the sqlite
  module is `#[ignore]`d with the finding. The flush in conformance properties
  6-10 and the fix in `sqlite/mod.rs` are still to do, in that order.
- **`parallel_reads.rs`** names `Backend::Redb` and the whole file is gated on
  that feature, since no other engine implements the setting. `opened_with`
  asserts the store reports the setting it was asked for, so the builder
  mutation named below now goes red - confirmed by making it.
- **`reentrancy.rs`** - all three tests now assert the callback ran and its
  write landed. An empty `notify` no longer passes.
- **`primitives/signal.rs` `subscription_location_captured`** asserts the
  captured line against `line!()`, so removing `#[track_caller]` goes red.
- **`reactive/field.rs:930`** is `== 1` rather than `>= 1`.
- **`reactive/map.rs:1282`** asserts `current_context() == Codec`, and the
  substring is `entry: ...` rather than a bare `123` that a line number could
  supply. Either entry name is accepted, because which is met first is the
  scan's order - that part of the original `||` was right.
- **`atomic_write.rs:35`** asserts the variant and drops the unreachable arm.
- **`atomic_write.rs:66`** is replaced by
  `a_write_that_landed_leaves_no_temporary_behind`, in a directory of its own,
  which catches a deliberately leaked temporary - confirmed by leaking one.
- **Two flushes no longer cross.** `StoreFile::persist` holds one lock across
  rendering the document *and* replacing the file; the guard it used to take
  covered the read alone and was gone before the replacement, so A rendered, B
  rendered, B replaced, A replaced, and the file kept what A saw.
  `writers_racing_each_other_all_land` failed twice in ten runs and now passes
  twenty out of twenty.
- **A struct on redb is stored by name.** `with_struct_map` and `to_vec_named`
  at the three write sites; the reader took either form already. Reordering two
  same-typed fields no longer swaps every stored value, a renamed field is no
  longer invisible, and a `skip_serializing_if` in the middle no longer moves
  the field after it into its place - the last being the silent one, where the
  write said `Ok`, the read said `Ok`, and the value was different.
- **The sqlite scan boundary is fixed**, and the tests that were waiting for it
  are no longer `#[ignore]`d. `scan_prefix` and `scan_keys` run their committed
  rows through `is_under`, as redb always did, and `key_range`'s upper bound is
  the separator's byte successor rather than `prefix.\u{10FFFF}` - which a child
  named `\u{10FFFF}z` sorted above, so it was invisible to every scan of the
  level above it and survived a delete of its own subtree.
- **Both migration adapters** filter with `is_under` rather than
  `key.starts_with` / `GLOB '{prefix}*'`. A map named `routes` was picking up
  `routes_v2`'s entries and then refusing them, failing the migration for good.
  The redb probe that recorded that now records its absence.
- **An empty path is refused.** `StorePathError::EmptyPath`, from
  `try_from_segments`, which is what `IntoStorePath` and so every write from a
  caller's strings goes through. `from_segments` still answers with the root,
  because a written-out empty list is a statement - an `as_root` struct's path
  is exactly that - while a computed one that filtered down to nothing is not.
- **`scan_keys` of a leaf: the finding was wrong, and the test with it.** A scan
  names the subtree *including its root*, so a leaf answering with itself is the
  contract, agreed by all five engines - `is_under` admits `key == prefix`, and
  conformance property 7 pins `scan_keys` and `scan_prefix` saying the same
  thing. Changing it broke that agreement on the three text engines, and
  property 7 caught it. Reverted; the test now asserts the real contract on
  every engine, and the non-termination it warned about is a caller's trap
  rather than a defect, named in the code where the answer is produced.

- **`two_stores_reactivity.rs:70`** is renamed to what it proves, its doc
  corrected, and a second test added that writes to the store whose binding is
  gone. On one point the audit overstated: a change on any source re-runs
  `refresh`, which reads every source, so the assertion was not reading a stale
  cache. `[verified]`

---

## Does not run at all

The worst kind, because a suite that reports green counts these as coverage.

**`tests/expand/nested_under_a_dotted_prefix.rs` has no `.expanded.rs`, and
`macrotest` writes one rather than failing.** `macrotest::expand()` runs in
`RegenerateFiles` mode: a missing golden is written and reported as *refreshed*,
which is not counted as a failure. On a fresh checkout it manufactures its own
answer every run. The case exists specifically to cover a dotted prefix
splitting, and nothing checks what it expands to. Either add the golden or
switch `tests/root.rs` to `expand_without_refresh`. `[verified: the file is
absent]`

**`crates/adapters/amethystate-reactor` is not in `[workspace].members`.** 324
lines of tests that `cargo test --workspace` never builds - and it depends on
`amethystate = "0.10.0"` from crates.io rather than by path, with a
`[patch.crates-io]` pointing at `../../../../windows-rs`, so it resolves on one
machine. Tests that only run on one laptop read as coverage and are not. Put it
in the workspace or delete it. `[verified: absent from members]`

**Nine of fifteen `error_reports` snapshots are never compared.**
`common::per_engine` names a snapshot after `default_backend().extension()`;
CI produces only `_redb` and `_json`. The `_db`, `_toml`, `_ron` variants sit
in `tests/snapshots/` with nothing to compare them to.

**No unit tests in any macro crate.** `amethystate-macros`,
`amethystate-macros-core`, `amethystate-macros-arena` have no `#[cfg(test)]` at
all. The validators whose diagnostics `fails/*.stderr` pins are reachable only
through a full `trybuild` compile, which runs in 2 of 6 CI legs -
`compile_tests.rs` requires `golden` + `redb` + none of json/toml/ron/sqlite, so
`--all-features` excludes it. `tests/root.rs` narrows `macrotest` further to
Windows.

**No CI cell makes toml or ron the text backend.** `common::text_backend()`
yields Json whenever `json` is on. The `toml =` and `ron =` arms of the `doc!`
macro, duplicated across five tamper files, are never selected: a typo in a
`ron` fixture would not be caught.

---

## Cannot fail

Each entry names the mutation. None was applied.

**`tests/reentrancy.rs:36, :56, :73`** - three tests register a subscriber, do
one write, and assert nothing beyond "the thread finished within 5s". Make
`ReactiveMapCore::notify` (`primitives/map_core.rs:305`) an empty body: all
three go green, because the callback that was meant to deadlock never runs.
Neither of the first two checks that the write it triggers landed.

Same file, `within()` (:16) treats `recv_timeout(..).is_err()` as *deadlocked* -
but a panic in the body drops the sender and yields `Disconnected` immediately.
The moment someone adds an assertion, a failure will be reported as a hang.

**`tests/old_value.rs` (all three, :17, :50, :77)** - they claim to pin where
`old_value` comes from, and the doc names the regression: "came from the write
buffer alone, so once a flush had emptied it the next write reported no old
value". The change a subscriber receives is built at
`store/primitives_factory.rs:379` with `.or_else(|| core.cache.get(&k))`, and
the cache is updated *after* the notify - so it still holds the previous value.
Replace `committed_or_buffered` (`backend/redb/mod.rs:565`) with a buffer-only
read - the exact regression - and all three stay green. What they pin is the
fallback. They also name no backend, though the doc's whole point is that this
"worked on the text backends and quietly did not on redb and sqlite".

**`tests/parallel_reads.rs:23, :65`** - `StoreBackend::parallel_reads()` is
implemented only on redb; every other engine takes the trait default `false`.
The file names no backend, so in CI's `json` cell both halves take the identical
sequential branch and the test compares a code path with itself - the failure
the module doc says it was written to avoid. And on redb: change
`store/builder.rs` `parallel_reads` to assign `false` and both tests still pass.

**`tests/reactive_map_defaults.rs:28` `test_map_defaults_applied_only_on_first_init`**
- both blocks assert the same two values at their defaults. If defaults were
re-seeded on every construction, which is the failure the name describes,
re-seeding writes exactly those values and the test passes. Delete the
`if !seeded_before` guard at `primitives_factory.rs:296` and it stays green. The
property is genuinely covered at :106 and in `map_defaults.rs:54`.

**`src/migration/engine.rs:1080` `test_drift_automatic_warning_log`** - carries
`#[traced_test]` and never calls `logs_contain`. `MigrationEngine::run` does not
log; the warnings live in `MigrationReport::log_to_tracing`
(`migration/mod.rs:92-104`), which the test never calls. Delete that whole block
and the test stays green.

**`tests/failed_migration_snapshot.rs:37`** - the doc says a snapshot must not
be written for a prefix whose migration failed, or drift compares the new schema
against itself. The test asserts only that the report has a failure and names
the prefix. Delete `migration/engine.rs:51-53` - the exact guard - and it stays
green. To earn its name it must reopen and read the snapshot back.

**`tests/persistent_only.rs:14`** - named for persistence, never reopens the
store. Every assertion reads back through the live handle, which answers from
its buffer. If nothing reached disk, the test passes. `init_flag_buffering.rs:18`
does the reopen properly.

**`primitives/signal.rs:217` `subscription_location_captured`** -
`assert!(!sub.location.file().is_empty())`. `Location::caller().file()` is never
empty. Remove `#[track_caller]` from `Signal::subscribe` and it stays green: the
location would point inside `signal.rs`, which is where the test lives.
`tests/panicking_builders.rs` does this correctly with `line!() + 2`.
`[verified]`

**`tests/observability_tracing.rs`** - delete the whole `impl Drop for
InstanceGuard` (`observability/mod.rs:55-59`) and every test in the file stays
green, while a `Field` outliving its state struct silently starts tracing
`source = "external"`.

**`tests/atomic_write.rs:66` `a_leftover_temp_file_does_not_disturb_the_next_open`**
- litters with `{stem}.tmp`, `.tmp{stem}`, `{stem}~`; the real temporaries are
`NamedTempFile::new_in`, i.e. `.tmpXXXXXX`. It does not matter: **no engine
enumerates its directory**, it opens `config.path` and nothing else. The test
would pass against a stub. Written this session.

**`tests/shape_on_disk.rs:50` `the_recorded_shape_survives_this_engines_codec`**
- opens, constructs, saves, reopens. If shape recording were removed entirely -
`ensure_snapshots` made a no-op - the reopen trivially succeeds. It catches only
"the codec chokes on a shape that *was* written". The three tests that pin a
shape is written are all `#[cfg(feature = "redb")]`. Written this session.

**`tests/two_stores_reactivity.rs:70` `dropping_one_store_leaves_the_other_half_alive`**
- nothing is dropped. The tuple `IntoPipeline` (`primitives/pipeline.rs:306`)
holds `$source.clone()` in its `refresh` closure and collects `keepalive`
besides, so the pipeline keeps the slow store alive by construction; only the
local binding goes. And `Pipeline::get()` reads its own cached signal without
re-reading sources, so the assertion holds even with the slow half completely
dead - "busy" was written before the block ended and nothing writes afterwards.
What it proves is the keepalive, which `pipe_keeps_its_source.rs` already pins.
Written this session. `[verified: the clone and the keepalive are both there]`

---

## Passes for the wrong reason

**`tests/backend_conformance.rs` properties 6-10 (:386, :429, :456, :478, :499)
never flush, so they never touch a committed row.** `open()` sets
`debounce(60s)` and none of them calls `save_now`. On redb and sqlite every
`set` goes to the in-memory buffer, and the scan then merges a buffer that *is*
filtered by `utils::is_under`. So five properties test `pending_prefix` and
`merge_buffered` rather than the engine's own range query.

That is exactly where sqlite is wrong. `sqlite/mod.rs` `scan_prefix` (:303) and
`scan_keys` (:355) run `WHERE key >= ? AND key < ?` from `utils::key_range`,
whose upper bound is `prefix + "." + \u{10FFFF}`, and **never apply `is_under`
to the rows that come back**; redb does, at `backend/redb/mod.rs:633`. So any
committed sibling whose next character sorts below `.` - space, `!`, `"`, `#`,
`$`, `%`, `&`, `'`, `(`, `)`, `*`, `+`, `,`, `-` - is returned as being under the
prefix, and `delete_prefix` is built on that scan, so it **destroys those
siblings**. `[verified: the filter is present in redb and absent in sqlite; six
characters confirmed destroyed by running the sqlite probe]`

The conformance run is green on sqlite - 29/29 - with the defect live. The
generator is not the problem: property 6 already produces a sub-`.` sibling in
roughly 88% of cases. The missing flush is.

Three separate things follow, and they are not one:

1. **Named deterministic tests** naming the character class, as
   `#[ignore]` findings. These are already written, in `tests/probe_sqlite.rs` -
   `a_scan_of_a_prefix_lists_only_what_is_under_it`,
   `delete_prefix_takes_only_the_subtree`,
   `a_flush_does_not_change_what_a_scan_lists`,
   `a_child_above_the_range_bound_is_still_scanned` - and they fail. They need
   lifting out of a probe file into a test file.
2. **A flush in properties 6-10**, so the conformance suite stops being blind to
   the committed path on every engine, not only for this defect.
3. **The fix itself** - `is_under` over the SQL rows, and an upper bound that is
   the byte-successor of `prefix + '.'` rather than a literal `U+10FFFF`.

Third without first means nothing shows that it was fixed.

**`tests/watcher_race.rs:56` `a_write_during_a_persist_still_reaches_the_file`**
- claims the generation is read before serializing so a later write stays
pending. The test writes 1..200, sleeps, **closes the block**, reopens, and
asserts 200. `impl Drop for TextStoreInner` calls `save_now()` unconditionally,
which writes the whole document regardless of any generation bookkeeping.
Replace the generation store with a constant and it stays green; it would also
pass with the debouncer thread never running. To pin the claim it must assert
*before* the drop.

**`tests/debounce_loss.rs:22` `a_write_during_a_commit_is_not_dropped`** - the
second write never lands during a commit. Debounce is 25ms and every `Schedule`
restarts the quiet period, so `set(first)` at t=0 and `set(second)` at t=20 are
covered by one `op()` at t≈45, with the thread parked in `recv_timeout` the
whole time. Revert `utils::clear_committed` to an unconditional
`pending.remove(key)` - the original bug - and it stays green, because the
single commit has landed `second` by assert time. The file also names no
backend, and in the `json` cell it runs the text engine, which has no pending
buffer at all - so the doc describes a path that is not running.

**`tests/store_tests.rs:149` `test_component_atomic_rollback`** (duplicated at
`backend/redb/mod.rs:1133`) - asserts `report.has_failures()` and that the value
is unchanged. Both hold if the step being "rolled back" never ran. It should
assert that one component succeeded then rolled back and the other failed.

**`tests/atomic_write.rs:35` `a_path_that_cannot_be_written_is_reported`** - the
real claim ("must not leave the caller thinking the value landed") is in the
`Ok(store)` branch, which is unreachable: with a directory on the store's path
every engine fails at `build()`. Only the `Err` branch runs, and its whole check
is that the path string appears somewhere in `{:?}`. Written this session.

**`tests/keyed_field_rename.rs:151` `dropping_a_plain_field_removes_its_stored_value`**
- the control for two `#[ignore]`d siblings, and the one that lost its
pre-condition. Both siblings assert the seeded value landed before migrating;
this one writes and goes straight to asserting `None` afterwards. If the write
went elsewhere, the post-assert is `None` anyway.
`migration_cleanup_composite.rs:114` has the pre-check.

**`tests/entry_cell.rs:229` `a_forgotten_cell_does_not_hold_the_store_open`** -
its own doc says the proof is that redb refuses a second open of a live file.
It never names redb, so under any other engine `reopened.is_ok()` is vacuous.
The second assertion survives, so the test is not worthless - but the
file-handle property is untested on four of five engines.

**`crates/adapters/amethystate-dioxus/tests/tests.rs:325`** -
`assert!(result.is_err())` on a `catch_unwind`, in a test whose only
`get_pipeline` call is the one inside it. `catch_unwind` catches any panic.
Make `get_pipeline` `panic!("unimplemented")` on its first line and the test
stays green. The siblings at :699 and :738 are protected by a positive
assertion; this one is not.

**`crates/adapters/amethystate-leptos/tests/tests.rs:241, :289`** -
`let _ = arena.set_map_entry(..)` then assert the count did not change. The
`let _` discards the `Result`: if the write failed for any reason the count is
unchanged and the test passes without testing unsubscription at all. Assert the
absence *after* unwrapping the write.

**`crates/adapters/amethystate-leptos/tests/tests.rs:190** -
`assert!(catch_unwind(|| { arena.get_pipeline(h); }).is_ok())` - the semicolon
drops the value, so the assertion says only "did not panic". Nothing checks the
pipeline still reads what it should.

---

## Weaker than its own name

**`tests/watcher_race.rs:24` `a_write_is_never_rolled_back_by_the_watcher`** -
asserts `seen >= n` and never counts a watcher pass. With no events delivered it
is 400 iterations of "a write I just made reads back". `atomicity_stress.rs`'s
`assert_saw_enough` is the pattern it needs.

**`src/reactive/field.rs:930`** - `assert!(*call_count >= 1)` where the property
is exactly one. A double-fire, which is the bug `entry_cell.rs:107` and
`cell.rs:448` exist to catch, passes here. Same at `src/reactive/map.rs:1100`
(`res.len() >= 3`), where a spurious event after index 2 is invisible; the
`sleep(100ms)` above it is superstition, since `notify` is synchronous on the
calling thread.

**`tests/reactive_map_defaults.rs:80` `test_new_defaults_applied_on_version_upgrade`**
- the name says new defaults *are* applied on a version upgrade; the body
asserts they are not, and the body is right. Map seeding
(`primitives_factory.rs:294`) is gated on the map's own path with no version in
it, so the `version = 1` / `version = 2` distinction is inert. Anyone grepping
for the answer finds a name that says the opposite.

**`tests/observability_tracing.rs:153` `forked_write_traces_with_source`** -
checks `logs_contain("field write")` and the path, never the `source` field.
Change `reactive/field.rs:468` to a bare `"external"` and it stays green; only
`field_set_trace_contains_source_name` (:75) catches that. The fork-specific
claim is untested.

**`tests/observability_tracing.rs:16` `instance_registered_on_new`** - named for
the instance registry, asserts only on the schema registry. Its assertions are a
strict subset of `fields_registered_in_schema_registry` (:32).

**`tests/set_durable.rs:183`** - `contents(&path).contains("42")` matches
`"answer": 142`, `420`, or `42` anywhere in the metadata. Every sibling in the
module asserts `"key": value`.

**`tests/interceptors.rs:58`** - the store assertion is decorative: the
recursion unwinds outermost-last, so the final store value is `1` whatever
happened at depth 10. The subscriber assertion at :53 is the load-bearing one
and does catch the mutation. Worth saying which is which before someone
"simplifies" them as equals.

**`crates/adapters/amethystate-slint/src/parser.rs:265` `returns_parse_errors`**
- `assert!(!err.message.is_empty())`. Return `"error"` for every input and it
passes. The other three tests in that module assert full structural equality.

**`tests/two_stores_reactivity.rs`, `tests/tamper_live.rs:68`** - `hits > 0`
without inspecting the delivered event; a spurious or stale notification
satisfies it.

**`tests/delete_prefix_dotted_keys.rs`, `tests/prefix_boundary.rs`** - both
promise "every engine" in their module docs and cover redb and json, because
neither names a backend. Neither ever flushes, so even where they run they test
the buffer. The engine where escaping-versus-comparison has teeth is sqlite.

---

## Loose error matching

`StorageError` has 11 coarse variants - `Open` covers a missing file, a
permission error, a parse failure, a broken `.meta` and a migration cycle alike
- and everything specific lives in attachments. So `is_err()` distinguishes
nothing, and the crate already has the precise forms:
`report.current_context()` matched against the variant, and
`tests/common/mod.rs::shape` + `insta` for the whole report shape.
`tests/error_reports.rs` is the house style working on the same failures one
directory over.

**The tamper suite never uses either.** All six text files import exactly
`use common::text_backend;`. Sixteen assertions reduce a `Report<StorageError>`
to a boolean:

- `tamper_broken_file.rs:79, :105, :124, :139, :163, :192, :230` - seven. Four
  are satisfied by serde_json returning any error at all. Make
  `StoreFile::load_or_empty` propagate a read error for an unreadable file and
  all seven stay green while the parse check they name is bypassed. `:124` and
  `:139` are load-bearing - they die if the root-must-be-a-table check goes -
  but even they cannot tell `RootMustBeObject` from `CodecError::Json`, which is
  the distinction their doc claims.
- `tamper_shapes.rs:77, :101, :126, :161` - four `built.is_err()` on
  `Cfg::new_with`. The three type tests are structurally identical and none says
  which error. Make `field_with_path` return `Err(StorageError::Read)` whenever
  `resolve_field` is `None` and all three stay green, having never reached the
  codec.
- `tamper_toml_inline.rs:124` - `#[ignore]`d, so it costs nothing today; worth
  pinning before it is un-ignored.

**`src/reactive/map.rs:1282`** - the sharpest single instance:

```rust
report.contains("not_int_key") || report.contains("123")
```

on `format!("{err:?}")` of a whole `Report` - source locations, attachments, and
a temp path carrying a 19-digit nanosecond stamp. `"123"` is a three-digit
substring that a line number or the path supplies on its own, and the `||` means
only one disjunct need hold. `current_context()` is never checked, though four
sibling tests in the same module check it. `[verified]`

**`src/reactive/cell.rs:427, :480`** - `is_err()` where `tests/entry_cell.rs:145`
does the same test with a snapshot.

**`tests/non_finite_float.rs:113, :167`** - the doc promises "`Field::unreadable`
says why"; the assertion is `try_get().is_err()`. The surrounding assertions do
the real work, so this is polish.

**`tests/kv.rs:151`** - the only bare `is_err()` in a file that otherwise
snapshots, and the one place where the distinction matters most: the type is
`u16`, the *correct* type, so the refusal must be by declared path rather than
by type mismatch, and a boolean cannot say that.

**`tests/atomic_write.rs:376`** - bare `is_err()` on `build()`; would pass on any
`Open` failure. The two at `:180` and `:337` are fine as written, each backed by
a second assertion that does the real work.

---

## Goldens that pin the wrong thing

**`fails/map_through_an_alias.stderr`** - 14 rustc errors over ~200 lines, of
which one is the diagnostic the case is named for. The other 13 are serde
trait-bound cascades quoting `$CARGO/serde_core-$VERSION/...`. Not a false pass -
trybuild diffs the whole file - but any serde or rustc bump rewrites the golden
for reasons unrelated to the macro.

**`fails/local_scope_not_send.stderr`** - over-specified in the dangerous
direction: it pins *both* reasons `LocalScope` is `!Send`. Remove the
`PhantomData` marker and the type is still `!Send` - the invariant holds - but
the golden goes red. `assert_not_impl` would pin the claim instead.

---

## Timing

Good, and the pattern to copy: `atomic_write.rs:231` sleeps
`policy.replace.budget() / 2` and `:287` asserts elapsed against the configured
budget in both directions - every bound derived from the policy rather than
guessed. `atomicity_stress.rs:77` `assert_saw_enough` refuses to report success
on a run that observed too little to have exercised anything.

Guesses: `debounce_loss.rs:33` (20ms/60ms against a 25ms debounce, and see
above), `watcher_race.rs:28` (5ms/5ms, and nothing is a lower bound on a watcher
pass), `store/util/ticker.rs:102` (600ms at a 30ms interval asserting `n >= 3`,
expecting ~20 - would pass with the interval silently at 200ms).

Sound: `test_debouncer_persistence` in both binary engines checks the "not yet on
disk" half synchronously before any sleep, which is where the weight is;
`a_flush_that_keeps_failing...` and `a_disk_that_comes_back_heals...` use 200ms
against a 50ms budget.

`TODO.md` separately records two file-watch tests as load-sensitive.
`watcher_race` is the mirror problem - load-*insensitive*, because it checks
nothing the load would affect.

Seen since, which pins which two: a `--workspace --all-features --no-fail-fast`
run failed `json_store::store_tests::file_watch_emits_delete_for_external_removal`
and `ron_store::store_tests::file_watch_emits_set_for_external_change`, and the
whole lib suite passed 137/137 immediately after. The tell that it was load and
not a change: the same test body is generated for all three text engines by
`define_store_test_suite!`, and two of the six instances failed. A break in what
they test would have taken all six.

---

## Hygiene

- **`src/test_utils.rs` `unique_store` reproduces the bug `unique_path` was
  fixed for.** `core/test_utils.rs:6` carries the reason in a comment - Windows
  resolves `SystemTime::now` more coarsely than tests start, so parallel tests
  collided - and adds a counter and the pid. `unique_store` has nanos only, then
  `.unwrap()`s the open, and never deletes its file. Every dioxus and leptos
  adapter test uses it.
- **`core/test_utils.rs:57` `TempPath::drop` sweeps by the full file name**, so
  `store.db.bak` goes and `store.meta` and a neighbouring `store.bak` do not.
- **`#[cfg_attr(feature = "toml", ignore)]` is gated on the wrong condition** -
  `text_backend()` picks Json whenever `json` is on, so under `--all-features`
  four tests are skipped for a toml defect that is not in play:
  `tamper_broken_file.rs:92, :145`, `tamper_live.rs:188`,
  `tamper_shapes.rs:135`. It should be `all(feature = "toml", not(feature =
  "json"))`.
- **`amethystate-yew/src/hooks.rs:113, :168, :187, :206, :223`** - five
  optimistic-update rollbacks that discard the `Report` entirely. The rollback is
  right; a user whose write an interceptor refused sees the value snap back with
  nothing said anywhere. Yew has no tests.
- `amethystate-gpui` is `--exclude`d from every CI job, so it is not even
  compiled there. `amethystate-yew`, `amethystate-tauri`, and
  `amethystate-slint`'s `ir.rs`/`lib.rs` have no tests at all.
- Five identical copies of the `doc!` macro and `settle()` across the tamper
  files.
- **`define_store_test_suite!` outlived its reason.** It generates one body per
  text engine because a store used to be one per binary; there is no such
  constraint now, and `backend_conformance.rs` already says one statement to
  every engine and reports which engine it failed on. What the macro gives
  instead is triplication: one flaky watcher test is three flaky tests, and one
  fix to a shared body is invisible until three suites agree. What it holds that
  the conformance suite does not - the file-watch tests, which are about a text
  file changing under the store - is what should move, and the rest should go.
- `tests/map_delete_prefix_notify.rs:24` and `tests/map_order.rs:33` describe
  their bugs in the present tense while asserting the fixed behaviour, against
  this repo's own rule that documentation describes the present.
- `tests/map_order.rs:38, :50, :62` - three of five tests compare two sorted
  reads of the same `DashMap`. `keys()` and `entries()` read
  `core.cache` and sort with `cmp_names`; `scan_prefix` and the buffer merge the
  doc describes are not on that path.

---

## What holds

Not a courtesy list. These were checked the same way and a mutation could not be
found, so they are what the suite can be trusted on.

**Error assertions done right:** `tests/error_reports.rs` (all 15 - whole-report
snapshots, engine-aware naming); `tests/errors_are_not_swallowed.rs` (all four
assert the exact variant, and one also checks the store is unchanged);
`src/migration/engine.rs:441` (asserts `current_context()`, destructures
`MigrationError::Gap` and checks all three fields, *and* checks the meta did not
advance); `src/migration/set.rs` `test_cycle_error` (matches the variant with a
`panic!` in the else arm); `src/reactive/map.rs` `test_map_intercept_and_reject`
and `test_key_specific_logic`.

**Whole files:** `tests/durability_crash.rs` - the strongest in the tree: a real
`process::abort()`, every engine named, and a `Granularity` distinction so the
assertion differs per engine rather than being weakened to fit all.
`tests/type_identity.rs` and `tests/type_hash.rs` - const asserts with a
positive control that stops the file collapsing into "everything equals
everything". `tests/watch_builder.rs` (all 14, every one an exact sequence).
`tests/tamper_names.rs` and `tests/tamper_dot_sentinel.rs` - exact values and
exact key sets, no booleans. `tests/entry_cell.rs` - four snapshots of real
report shapes and a working double-fire regression test.
`tests/slice_subscription.rs`, `tests/clear_events.rs`, `tests/field_delete.rs`,
`tests/kv_clear.rs`, `tests/map_defaults.rs`, `tests/map_dotted_keys.rs`,
`tests/nested_reactive_map.rs`, `tests/init_flag_buffering.rs`,
`tests/instance_registry.rs`, `tests/migration_complex.rs` (514 lines that earn
them - ordering across prefixes, exact step counts, exact log lines, and
rollback proven positively). The test modules in `src/reactive/local.rs`,
`src/reactive/pipeline.rs`, `src/store/backend/redb/recovery.rs`, and
`src/store/backend/utils.rs`.

**Written this session and holding:** `tests/atomic_write.rs`'s four Windows
tests - elapsed time against the configured budget *and* `os error 5` in the
report, plus a control that exists solely to prove the configuration reaches the
write path. `tests/atomicity_stress.rs`. `tests/serializer_damage.rs` - four
independent assertions on one refusal, and a contrast test carrying its own
control so it cannot silently degenerate.

**Individual pieces worth copying:** `tests/panicking_builders.rs` uses
`line!() + 2` against a captured panic location, which is the correct way to
test `#[track_caller]`. `tests/pipe_keeps_its_source.rs:25` explains in a
comment why it writes through the raw store rather than a retained handle.
`primitives/signal.rs` `concurrent_writes_keep_value_and_source_together` - 8
threads by 500 writes checking that the value and its source arrive as a pair.
`tests/set_durable.rs` proves the negative first - a plain set leaves the old
value in the file - before proving the positive.
`amethystate-dioxus`'s `test_pipeline_lifecycle_and_tuple_pipe` - exact strings
through a tuple pipe across mount and unmount, plus a render-count check.

---

## Not checked

- No cargo run during three of the four passes: another agent was editing
  imports across the tree and a build would have been noise. Claims are from
  reading, with the mutation named so each is confirmable in one edit.
- `tests/snapshots/*.snap` contents were listed but not read, so nothing here
  says whether an individual snapshot is a *good* snapshot - only that nine of
  them are never compared.
- `primitives/{map_ops_async,field_ops,field_ops_async}.rs`, `store/kv.rs`,
  `reactive/watch.rs` beyond `register_with_source` and `filterable`.
- The five `probe_*.rs` files, deliberately: they print rather than assert and
  are known raw material.
