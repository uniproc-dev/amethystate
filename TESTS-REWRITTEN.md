# Tests that were rewritten, and what they assert now

A rewritten test is a test whose *claim* changed, not one that was reformatted.
Each entry says what it used to assert, what it asserts instead, why the old
claim stopped being the right one, and how the new one was shown to still fail
when the thing it guards is broken.

The last column is the one that matters. A rewrite that nobody checked against
a broken build is a test that was quietly deleted.

## Rewritten because the test was measuring load, not behaviour

### `atomic_write.rs::a_policy_that_says_not_to_retry_is_obeyed`

**Was:** one failing save under `WriteAttempts::once()`, timed, asserted to
finish in under half the default policy's budget - 200 ms of a 400 ms budget.

**Now:** two failing saves in the same test, one under the default policy and
one under `once()`, and the no-retry one must be faster than the other by at
least half the budget the retries spend sleeping.

**Why:** the property is "no sleeping happened", and the old form asked it as
"less than 200 ms of wall clock". A single blocked `persist` under the whole
suite's load passes 200 ms without anything being wrong. The pauses the retries
add are `std::thread::sleep` - load cannot shorten them - so comparing two runs
measures the sleeping and nothing else.

**Shown to fail:** both sides given the retrying policy. Reported
`425.2836ms against 421.1016ms ... the 400ms of waiting the retries add did not
show`, which is the message a real regression would print.

### `atomicity_stress.rs` - all four loops

`a_reader_never_meets_a_half_written_file`,
`a_holder_coming_and_going_never_leaves_a_broken_file`,
`the_metadata_file_is_never_half_written_either`, and the writer loop they share
a shape with.

**Was:** run for a fixed 5 s, then assert that enough happened inside it - 20
writes, 20 whole reads, 5 holds, a save that met the holder.

**Now:** run *until* those counts are met, with a 120 s ceiling that means the
machine is stuck rather than busy. The floors are unchanged; only the exit
condition moved.

**Why:** the counts are what make the test meaningful, and the deadline was a
guess about how long reaching them takes on an idle machine. Under the full
suite the same real work happened, there was just less of it in five seconds -
`only 18 writes in 5s` is a report about the host, not about atomicity.

**Shown to fail:** unchanged - the assertions and their floors are the same
ones, so a build that cannot reach a floor within two minutes still fails, and
the torn-file panics inside the loops were not touched. Side effect worth
noting: the file now runs in 2.9 s instead of 6.0 s, because a loop stops when
it is done.

## Rewritten because the behaviour they pinned changed on purpose

The store's listing order used to be the joined key's, where a name is escaped;
it is the levels' order now. See TODO.md, *Where the diff stands*.

### `book_reactive_map.rs::the_order_is_the_stores`

**Was:** `["10", "9", "a1b", "a.b"]`. **Now:** `["10", "9", "a.b", "a1b"]`.

**Why:** `a.b` used to sort after `a1b` because the key it became began `a\.`,
and `\` is above `1`. The key holds the levels now, so the name sorts as
itself and `.` is below `1`.

**Shown to fail:** it did - this is one of the five engine cases that failed
when the ordering changed, which is how the change was found to reach here.

### `reactive/map.rs` - the `ReactiveMap::keys` doctest

Same change, same reason. The prose above it claimed the store sorts `a.b`
after `a1b` "because the key it writes begins `a\.`", which is no longer true of
any engine.

### `path.rs::a_name_sorting_between_a_path_and_its_child_does_not_end_the_walk`

**Was:** `pot!luck` sorts between `pot` and `pot.ato`, so a walk must not stop
at the first key the subtree does not contain.

**Now:** `a_subtree_is_one_run_with_nothing_of_anyone_elses_in_it` - sorting
`[potato, pot!luck, pot.ato, pot]` gives `[pot, pot.ato, pot!luck, potato]`.

**Why:** the old test pinned a defect of the joined spelling. Under level order
a subtree is contiguous, which is what let `Places::take` drop its
`may_still_reach` predicate; the new test is the property that replaced it.

### `path.rs` - two subtree property tests

`a_walk_stops_exactly_where_the_range_does` and
`a_walk_never_stops_before_what_the_subtree_holds` became
`the_range_a_key_gives_holds_exactly_the_subtree` and
`everything_grown_from_a_path_is_in_its_range`.

**Why:** both asked about `Subtree`, the string-range type, which no longer
exists. The properties they expressed are worth keeping, so they were moved onto
`Key` rather than deleted: the range holds exactly the subtree, and everything
grown from a path is inside it.

**Shown to fail:** the same properties, stated over `Key`, are what caught the
first encoding being wrong - `["ui\0x"]` read as a key under `["ui"]`. That was
found by `probe_redb::a_scan_stops_at_the_level_boundary`, and
`a_path_as_a_flat_engine_key.rs::a_name_holding_the_bytes_the_encoding_uses_is_not_under_the_prefix`
now pins it directly.

### `path.rs::a_path_hashes_like_its_key`

**Was:** `hash_of(&path) == hash_of(&"ui")` - a path hashes like the `&str` it
spells.

**Now:** `a_path_hashes_by_its_levels_however_it_arrived`, plus
`two_levels_do_not_hash_as_the_one_they_spell`.

**Why:** the old assertion pinned the `Borrow<str>` contract, which is gone.
`Hash` and `Eq` moved to the levels so that all three of `Ord`, `Eq` and `Hash`
answer the same question - before, `Ord` compared levels while the other two
compared the joined spelling, which is a hazard `std` relies on being absent.
Nothing hashes a path *as a string* any more, so the property to hold is that a
path hashes the same however it was built, and that two levels do not collide
with the one name they spell.

**Shown to fail:** the second test is the collision guard - `["a","b"]` against
`["ab"]` - and it fails if the level count is dropped from the hash.

### `path.rs::comparing_names_answers_what_comparing_their_keys_answers`

**Was:** `cmp_names(a, b)` equals comparing the joined keys the names become.
**Now:** `a.cmp(b)` equals comparing the encoded keys.

**Why:** `cmp_names` existed because the joined order was not the name's order.
It is now, so the function went and the property became the plain one.

## Un-ignored, because the code stopped failing them

### `probe_sqlite.rs::a_namespace_flag_and_a_value_at_its_name_coexist`

And `a_value_written_after_a_namespace_flag_survives` beside it. Both carried
`#[ignore = "a failure here is this file's finding"]` and both now run.

**Was the finding:** the write buffer was one `HashMap<StorePath, PendingOp>`
holding values and namespace markers alike. `set(["cfg"])` and marking `cfg`
were one entry, so whichever came second dropped the first - the value in one
order, the marker in the other. `PendingOp::is_data()` hid this from readers
without preventing it: the `Set` was gone from the map before any reader
filtered anything.

**What changed:** `Pending` is a struct holding the two apart by construction -
`at: HashMap<StorePath, PendingOp>` beside `marking: HashMap<StorePath, bool>` -
so no name can put a value where a marker goes. `PendingOp::Init` and
`is_data()` went with it. Keying the markers under a synthetic `init.<ns>` path
was the other candidate and is worse: it re-imports the collision `TODO.md`
records for the disk, where a prefix declared at `init.foo` lands on the marker
for `foo`.

**Shown to fail:** both tests failed against the previous commit with exactly
the messages they carry - `the value at 'cfg' was lost to the namespace flag of
the same name`, and the `is_initialized` assertion in the other order - and pass
now.

`tests/a_marker_and_a_value_share_a_name.rs` asks the same three questions of
every engine, including the unflushed case where the answer comes from the
buffer rather than the disk. redb held the identical defect with no test on it;
that file is the test.

## Removed rather than rewritten

### `migration_reactive_map.rs` - the hand-deleting loop

A migration step deleted `routes.{key}` in a loop before writing the new map.
That was a workaround for cleanup not taking a withdrawn declaration's entries,
which `MigrationContext::drop_withdrawn` now does.

**Shown to be dead:** removed, and all ten cases across five engines still pass.

### `an_error_raised_through_anyhow.rs` - the `raised` helper

`Err(why).map_err(anyhow::Error::from)` became `anyhow::Error::from(why)`. The
conversion under test is the same one; the `Result` around it was scaffolding
clippy could see through.
