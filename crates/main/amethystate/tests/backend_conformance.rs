//! What a store is, said once and asked of every engine.
//!
//! Each engine has its own unit tests, written when it was written, and they
//! overlap by accident. This file is the other thing: one set of statements
//! that hold of a store whichever engine is underneath, run against whichever
//! engine the build enabled. A statement that fails on one engine and passes on
//! another is the finding - that is the whole point of the file, so nothing
//! here is written down to what the engines currently do.
//!
//! # What is deliberately not stated here
//!
//! - **Which writes a commit takes with it.** A durable write commits one key
//!   on redb and sqlite and the whole document on the text engines. That is a
//!   real difference and it is genuinely engine-specific - a document is
//!   rewritten whole because it is a document - so it belongs in
//!   `durability_crash.rs`, which pins it per engine, and not here.
//! - **What a scan's bytes are.** The value bytes are the engine's own format,
//!   so only their stability within one engine is asserted, by
//!   `writing_then_deleting_leaves_the_store_as_it_was`, never their content
//!   across engines.
//! - **Where the metadata lives.** Two files on the text engines, one
//!   transaction on the flat ones. A statement about a store cannot see this
//!   without reaching for the file, which is what `tamper_meta.rs` is for.
//! - **Ordering of a map's `keys` against the key type's own `Ord`.** That is a
//!   statement about `ReactiveMap`, not about a store, and `map_order.rs` has
//!   it.
//!
//! # Two deliberate narrowings of the generators, and why
//!
//! A segment consisting of nothing but the separator is excluded from the
//! generated names, and `a_level_named_dot_is_an_ordinary_level` takes it
//! instead. `["."]` is mapped to the whole document by the text engines, so
//! leaving it in the general strategy made every property fail there for one
//! already-known reason and hid everything else in the space.
//!
//! The generated key sets are antichains - no generated path is an ancestor of
//! another - everywhere except `a_leaf_and_a_branch_coexist_at_one_name`, which
//! is the test about ancestry. Mixing the two made every scan property fail for
//! that one's reason instead of its own.

use amethystate::Store;
use amethystate::errors::WriteError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{
    Occupied, StorageError, StoreEvent, StoreOp, StorePath, StorePathError, SubscriptionKind,
};
use amethystate_core::test_utils::TempPath;
use proptest::prelude::*;

mod common;
use common::once_per_engine;

/// A namespace named the way a key on disk spells it.
fn ns(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).expect("a namespace the test wrote itself")
}

use std::sync::{Arc, Mutex};
use std::time::Duration;


/// A store on `backend`, with the debouncer and the watcher pushed far enough
/// out that nothing lands except when a property asks for it.
fn open(backend: Backend, file: &TempPath) -> Store {
    StoreBuilder::new(file.path())
        .backend(backend)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .expect("the store opened")
}

/// Few cases, because each one opens a file.
///
/// Counterexamples are kept, in the `.proptest-regressions` file beside this
/// one, and replayed before the new draws on every later run. Without that a
/// property fails only on the runs whose draw happens to reach it: the failing
/// set moved between runs of the same tree - json 2 and toml 4 one time, json 3
/// and toml 3 the next - and a real regression was indistinguishable from a
/// different roll of the dice.
///
/// Keeping them costs a file the suite writes and a reader has to commit. That
/// is the trade, and it is worth taking: a counterexample found once and lost
/// by the next run is the worst of both worlds, since the search paid for it
/// and nothing kept it.
fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 24,
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    }
}

/// Weighted towards what breaks a key rather than towards what a name usually
/// is: the separator and the escape are what a flat engine has to encode, the
/// bracket and the star are what a pattern language would read as something
/// other than itself, and the tiny letter range makes different names collide
/// often enough to matter. `any::<char>()` is in at low weight because a name
/// is bytes and an engine that quotes its keys has to survive whatever is in
/// them.
fn segment() -> impl Strategy<Value = String> {
    amethystate_core::strategies::segment().prop_filter(
        "a level named `.` has its own test",
        |name| name != ".",
    )
}

fn path() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(segment(), 1..4)
}

fn path_set() -> impl Strategy<Value = Vec<Vec<String>>> {
    prop::collection::vec(path(), 1..6)
}

/// The generated paths with every ancestry relation dropped, so what is left is
/// a set of leaves - which is what a document engine and a flat engine can both
/// represent. Duplicates go with them, since a path starts with itself.
fn leaves(raw: Vec<Vec<String>>) -> Vec<StorePath> {
    let mut kept: Vec<StorePath> = Vec::new();

    for segments in raw {
        let candidate = StorePath::from_segments(&segments);
        let related = kept
            .iter()
            .any(|k| k.starts_with(&candidate) || candidate.starts_with(k));
        if !related {
            kept.push(candidate);
        }
    }

    kept
}

/// The prefixes worth scanning for a given key set: the root, every written
/// path, every ancestor of one, and one path from elsewhere. Random prefixes
/// alone almost never land on a node that exists, which is where a scan can be
/// wrong.
fn probe_prefixes(written: &[StorePath], elsewhere: &StorePath) -> Vec<StorePath> {
    let mut out = vec![StorePath::root(), elsewhere.clone()];

    for path in written {
        out.push(path.clone());
        let mut current = path.clone();
        while let Some(parent) = current.parent() {
            out.push(parent.clone());
            current = parent;
        }
    }

    out
}

fn write_leaves(store: &Store, written: &[StorePath]) {
    for (i, path) in written.iter().enumerate() {
        store.set(path, &(i as u32)).expect("the write landed");
    }
}

fn a_value_reads_back_where_it_was_written(backend: Backend) {
    proptest!(config(), |(raw in path_set())| {
        let file = TempPath::new("conf_read_back");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        for (i, path) in written.iter().enumerate() {
            prop_assert_eq!(
                store.get::<u32>(path).unwrap(),
                Some(i as u32),
                "at {}", path
            );
        }
    });
}

/// An ancestor of a written path is excluded, and asked about separately by
/// `an_ancestor_is_not_a_value` - a tree engine has a node there where a flat
/// one has no key, and the two answer differently without either being wrong.
fn a_write_leaves_every_other_path_alone(backend: Backend) {
    proptest!(config(), |(raw in path_set(), probe in path())| {
        let file = TempPath::new("conf_no_other_path");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        let probe = StorePath::from_segments(&probe);
        let is_ancestor = written.iter().any(|path| path.starts_with(&probe));

        if !written.contains(&probe) && !is_ancestor {
            prop_assert_eq!(
                store.get::<u32>(&probe).ok(),
                Some(None),
                "nothing was written at {}", probe
            );
        }
    });
}

/// A path with values under it and none of its own never reads back as a value.
///
/// Where the engines are allowed to differ: the flat ones hold no key there and
/// answer `Ok(None)`, the document ones hold a node and answer with a decode
/// failure. What none of them may do is hand back something that reads as a
/// value, because the only value in reach is one that belongs to a path
/// underneath - and a caller cannot tell that apart from a real reading.
fn an_ancestor_is_not_a_value(backend: Backend) {
    proptest!(config(), |(head in path(), child in segment())| {
        let file = TempPath::new("conf_ancestor");
        let store = open(backend, &file);

        let parent = StorePath::from_segments(&head);
        let under = parent.push(&child);
        store.set(&under, &7u32).unwrap();

        let read = store.get::<u32>(&parent);
        prop_assert!(
            matches!(read, Err(_) | Ok(None)),
            "reading {} handed back a value, and the only one in reach is at {}",
            parent, under
        );
    });
}

fn the_last_write_is_the_one_that_reads_back(backend: Backend) {
    proptest!(config(), |(target in path(), values in prop::collection::vec(any::<u32>(), 1..8))| {
        let file = TempPath::new("conf_last_write");
        let store = open(backend, &file);
        let target = StorePath::from_segments(&target);

        for value in &values {
            store.set(&target, value).unwrap();
        }

        prop_assert_eq!(
            store.get::<u32>(&target).unwrap(),
            values.last().copied(),
            "at {}", target
        );
    });
}

fn writing_then_deleting_leaves_the_store_as_it_was(backend: Backend) {
    proptest!(config(), |(raw in path_set(), extra in path())| {
        let file = TempPath::new("conf_write_delete");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        let extra = StorePath::from_segments(&extra);
        prop_assume!(
            written
                .iter()
                .all(|p| !p.starts_with(&extra) && !extra.starts_with(p))
        );

        let before = store.scan_prefix(StorePath::root()).unwrap();

        store.set(&extra, &4242u32).unwrap();
        store.delete(&extra).unwrap();

        prop_assert_eq!(
            store.scan_prefix(StorePath::root()).unwrap(),
            before,
            "writing and deleting {} was not a round trip", extra
        );
    });
}

/// An ancestor of a written path is deliberately not asked about here, though
/// it is exactly where `delete` and `delete_prefix` could be confused. A
/// document engine stores a map-valued field as the same node a level with
/// values under it is, and `delete` is handed a path and nothing else, so it
/// cannot tell the two apart: refusing to remove a node with children would
/// refuse to delete a field whose value is a struct. The flat engines have no
/// key at an ancestor and take nothing; the document engines take the subtree.
/// Recorded rather than demanded - see `an_ancestor_is_not_a_value` below.
fn deleting_what_is_not_there_changes_nothing(backend: Backend) {
    proptest!(config(), |(raw in path_set(), absent in path())| {
        let file = TempPath::new("conf_delete_absent");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        let absent = StorePath::from_segments(&absent);
        prop_assume!(!written.iter().any(|p| p.starts_with(&absent)));

        let before = store.scan_prefix(StorePath::root()).unwrap();
        store.delete(&absent).unwrap();

        prop_assert_eq!(
            store.scan_prefix(StorePath::root()).unwrap(),
            before.clone(),
            "deleting {}, which holds nothing", absent
        );

    });
}

/// The sibling is built rather than generated: `uix` is a string away from `ui`
/// and a whole level away from it, and a generator that had to stumble on that
/// pair would not.
fn a_scan_lists_exactly_what_is_under_the_prefix(backend: Backend) {
    proptest!(config(), |(
        head in path(),
        tails in prop::collection::vec(path(), 1..4),
        extra in segment(),
        elsewhere in path(),
    )| {
        let file = TempPath::new("conf_scan_exact");
        let store = open(backend, &file);

        let prefix = StorePath::from_segments(&head);

        let mut sibling_head = head.clone();
        sibling_head.last_mut().unwrap().push_str(&extra);
        let sibling = StorePath::from_segments(&sibling_head);

        let elsewhere = StorePath::from_segments(&elsewhere);
        prop_assume!(!elsewhere.starts_with(&prefix) && !prefix.starts_with(&elsewhere));
        prop_assume!(!elsewhere.starts_with(&sibling) && !sibling.starts_with(&elsewhere));

        let under = leaves(
            tails
                .iter()
                .map(|tail| head.iter().chain(tail).cloned().collect())
                .collect(),
        );

        write_leaves(&store, &under);
        for tail in &tails {
            let beside = sibling.join(&StorePath::from_segments(tail));
            store.set(&beside, &9000u32).unwrap();
        }
        store.set(&elsewhere, &1u32).unwrap();

        let mut expected: Vec<StorePath> = under.to_vec();
        expected.sort();

        let mut listed = store.scan_keys(&prefix).unwrap();
        listed.sort();

        prop_assert_eq!(listed, expected, "scanning {}", prefix);
    });
}

fn scan_keys_and_scan_prefix_agree(backend: Backend) {
    proptest!(config(), |(raw in path_set(), elsewhere in path())| {
        let file = TempPath::new("conf_scan_agree");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        let elsewhere = StorePath::from_segments(&elsewhere);
        for prefix in probe_prefixes(&written, &elsewhere) {
            let keys = store.scan_keys(&prefix).unwrap();
            let entries: Vec<StorePath> = store
                .scan_prefix(&prefix)
                .unwrap()
                .into_iter()
                .map(|(k, _)| k)
                .collect();

            prop_assert_eq!(keys, entries, "scanning {}", prefix);
        }
    });
}

fn a_scan_comes_back_sorted(backend: Backend) {
    proptest!(config(), |(raw in path_set(), elsewhere in path())| {
        let file = TempPath::new("conf_scan_sorted");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        let elsewhere = StorePath::from_segments(&elsewhere);
        for prefix in probe_prefixes(&written, &elsewhere) {
            let keys = store.scan_keys(&prefix).unwrap();
            for pair in keys.windows(2) {
                prop_assert!(
                    pair[0] < pair[1],
                    "scanning {} gave {:?} before {:?}", prefix, pair[0], pair[1]
                );
            }
        }
    });
}

fn every_key_a_scan_returns_is_a_path_under_the_prefix(backend: Backend) {
    proptest!(config(), |(raw in path_set(), elsewhere in path())| {
        let file = TempPath::new("conf_scan_shape");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);

        let elsewhere = StorePath::from_segments(&elsewhere);
        for prefix in probe_prefixes(&written, &elsewhere) {
            for key in store.scan_keys(&prefix).unwrap() {
                prop_assert!(
                    key.starts_with(&prefix),
                    "scanning {} gave {:?}, which is not under it", prefix, key
                );
            }
        }
    });
}

fn delete_prefix_takes_the_subtree_and_nothing_beside_it(backend: Backend) {
    proptest!(config(), |(
        head in path(),
        tails in prop::collection::vec(path(), 1..4),
        extra in segment(),
        elsewhere in path(),
    )| {
        let file = TempPath::new("conf_delete_prefix");
        let store = open(backend, &file);

        let prefix = StorePath::from_segments(&head);

        let mut sibling_head = head.clone();
        sibling_head.last_mut().unwrap().push_str(&extra);
        let sibling = StorePath::from_segments(&sibling_head);

        let elsewhere = StorePath::from_segments(&elsewhere);
        prop_assume!(!elsewhere.starts_with(&prefix) && !prefix.starts_with(&elsewhere));
        prop_assume!(!elsewhere.starts_with(&sibling) && !sibling.starts_with(&elsewhere));

        let under = leaves(
            tails
                .iter()
                .map(|tail| head.iter().chain(tail).cloned().collect())
                .collect(),
        );
        let beside: Vec<StorePath> = under
            .iter()
            .filter_map(|p| p.strip_prefix(&prefix))
            .map(|rest| sibling.join(&rest))
            .collect();

        write_leaves(&store, &under);
        for path in &beside {
            store.set(path, &9000u32).unwrap();
        }
        store.set(&elsewhere, &1u32).unwrap();

        store.delete_prefix(&prefix).unwrap();

        for path in &under {
            prop_assert_eq!(
                store.get::<u32>(path).ok(),
                Some(None),
                "{} was under the deleted prefix {}", path, prefix
            );
        }
        for path in &beside {
            prop_assert_eq!(
                store.get::<u32>(path).unwrap(),
                Some(9000),
                "{} is beside {}, not under it", path, prefix
            );
        }
        prop_assert_eq!(
            store.get::<u32>(&elsewhere).unwrap(),
            Some(1),
            "{} is nowhere near {}", elsewhere, prefix
        );
    });
}

fn a_name_holding_the_separator_stays_one_level(backend: Backend) {
    proptest!(config(), |(outer in segment(), parts in amethystate_core::strategies::name_holding_the_separator())| {
        let file = TempPath::new("conf_dotted_name");

        let (left, right, dotted) = parts;
        let one_level = StorePath::from_segments([&outer, &dotted]);
        let taken_apart = StorePath::from_segments([&outer, &left, &right]);
        let parent = StorePath::segment(&outer);

        {
            let store = open(backend, &file);
            store.set(&one_level, &11u32).unwrap();

            prop_assert_eq!(store.get::<u32>(&one_level).unwrap(), Some(11));
            prop_assert_eq!(
                store.get::<u32>(&taken_apart).ok(),
                Some(None),
                "{} is not {}", taken_apart, one_level
            );
            prop_assert_eq!(
                store.scan_keys(&parent).unwrap(),
                vec![one_level.clone()]
            );

            store.flush_prefix(StorePath::root()).unwrap();
        }

        let store = open(backend, &file);
        prop_assert_eq!(
            store.get::<u32>(&one_level).unwrap(),
            Some(11),
            "{} did not survive the reopen", one_level
        );
        prop_assert_eq!(
            store.get::<u32>(&taken_apart).ok(),
            Some(None),
            "the reopen split {} into levels", one_level
        );
        prop_assert_eq!(store.scan_keys(&parent).unwrap(), vec![one_level.clone()]);
    });
}

/// 12. A value at a name and values under that name either coexist, or the
///     second write is refused and the first survives. Neither is destroyed.
///
/// The one place the suite does not ask for the same answer from everyone. A
/// document holds a value at a node or values under it, never both, so the flat
/// engines take the second write and the document engines refuse it - and a
/// refusal that leaves the first value alone is as good an answer as keeping
/// both. What stays universal is that nothing is lost without a word.
///
/// Both orders, because the order the two writes arrive in is what decides
/// which of the two an engine that walks a tree would have destroyed.
fn a_leaf_and_a_branch_coexist_at_one_name(backend: Backend) {
    proptest!(config(), |(head in path(), child in segment())| {
        let file = TempPath::new("conf_leaf_and_branch");
        let store = open(backend, &file);

        let node = StorePath::segment("leaf_first").join(&StorePath::from_segments(&head));
        let under = node.push(&child);
        store.set(&node, &1u32).unwrap();

        match store.set(&under, &2u32) {
            Ok(()) => {
                prop_assert_eq!(
                    store.get::<u32>(&node).ok(),
                    Some(Some(1)),
                    "writing under {} lost the value at it", node
                );
                prop_assert_eq!(store.get::<u32>(&under).ok(), Some(Some(2)));
            }
            Err(refused) => {
                prop_assert!(
                    refused.contains::<Occupied>(),
                    "refused for some other reason: {refused:?}"
                );
                prop_assert_eq!(
                    store.get::<u32>(&node).ok(),
                    Some(Some(1)),
                    "the write under {} was refused and took the value at it anyway", node
                );
            }
        }

        let node = StorePath::segment("branch_first").join(&StorePath::from_segments(&head));
        let under = node.push(&child);
        store.set(&under, &2u32).unwrap();

        match store.set(&node, &1u32) {
            Ok(()) => {
                prop_assert_eq!(store.get::<u32>(&node).ok(), Some(Some(1)));
                prop_assert_eq!(
                    store.get::<u32>(&under).ok(),
                    Some(Some(2)),
                    "writing at {} lost what was under it", node
                );
            }
            Err(refused) => {
                prop_assert!(
                    refused.contains::<Occupied>(),
                    "refused for some other reason: {refused:?}"
                );
                prop_assert_eq!(
                    store.get::<u32>(&under).ok(),
                    Some(Some(2)),
                    "the write at {} was refused and took what was under it anyway", node
                );
            }
        }
    });
}

/// 13. A reopen gives back everything that was committed and nothing else.
fn a_reopen_gives_back_what_was_committed(backend: Backend) {
    proptest!(config(), |(raw in path_set())| {
        let file = TempPath::new("conf_reopen");
        let written = leaves(raw);

        let before = {
            let store = open(backend, &file);
            write_leaves(&store, &written);
            store.flush_prefix(StorePath::root()).unwrap();
            store.scan_keys(StorePath::root()).unwrap()
        };

        let store = open(backend, &file);

        for (i, path) in written.iter().enumerate() {
            prop_assert_eq!(
                store.get::<u32>(path).unwrap(),
                Some(i as u32),
                "{} did not survive the reopen", path
            );
        }
        prop_assert_eq!(
            store.scan_keys(StorePath::root()).unwrap(),
            before,
            "the reopen changed which keys exist"
        );
    });
}

/// 14. A map's `len`, `keys` and `entries` agree with each other and with a
///     scan of the map's own path.
fn a_map_agrees_with_itself_and_with_a_scan(backend: Backend) {
    proptest!(config(), |(keys in prop::collection::vec(segment(), 1..6))| {
        let file = TempPath::new("conf_map_agrees");
        let store = open(backend, &file);
        let at = StorePath::segment("m");
        let map = store.kv().map::<String, u32>("m").unwrap();

        for (i, key) in keys.iter().enumerate() {
            map.insert(key.clone(), &(i as u32)).unwrap();
        }

        let mut distinct = keys.clone();
        distinct.sort();
        distinct.dedup();

        prop_assert_eq!(map.len(), distinct.len(), "len");
        prop_assert_eq!(map.keys().count(), distinct.len(), "keys");
        prop_assert_eq!(map.entries().count(), distinct.len(), "entries");

        let from_scan: Vec<String> = store
            .scan_keys(&at)
            .unwrap()
            .iter()
            .filter_map(|k| k.strip_prefix(&at))
            .filter_map(|rest| rest.name().map(|name| name.into_owned()))
            .collect();

        let mut membership = from_scan.clone();
        membership.sort();
        prop_assert_eq!(&membership, &distinct, "the scan disagrees with the map");

        prop_assert_eq!(
            &map.keys().collect::<Vec<_>>(),
            &from_scan,
            "keys came back in an order the scan does not use"
        );
        prop_assert_eq!(
            &map.entries().map(|(k, _)| k).collect::<Vec<_>>(),
            &from_scan,
            "entries came back in an order the scan does not use"
        );
    });
}

/// 15. A level with no name is refused by every entry point with
///     `StorageError::Path` over a `StorePathError::EmptySegment`, and the
///     store is left untouched.
fn a_level_with_no_name_is_refused_and_changes_nothing(backend: Backend) {
    proptest!(config(), |(raw in path_set(), holed in path(), at in 0usize..8)| {
        let file = TempPath::new("conf_empty_level");
        let store = open(backend, &file);
        let written = leaves(raw);

        write_leaves(&store, &written);
        let before = store.scan_prefix(StorePath::root()).unwrap();

        let mut segments = holed.clone();
        let at = at % (segments.len() + 1);
        segments.insert(at, String::new());

        let refusals = [
            ("get", store.get::<u32>(segments.clone()).err()),
            ("set", store.set(segments.clone(), &1u32).err()),
            ("delete", store.delete(segments.clone()).err()),
            ("delete_prefix", store.delete_prefix(segments.clone()).err()),
            ("scan_keys", store.scan_keys(segments.clone()).err()),
            ("scan_prefix", store.scan_prefix(segments.clone()).err()),
        ];

        for (call, refusal) in refusals {
            prop_assert!(
                refusal.is_some(),
                "{} accepted a level with no name at index {}", call, at
            );
            let report = refusal.unwrap();
            prop_assert_eq!(report.current_context(), &StorageError::Path, "{}", call);
            prop_assert!(
                matches!(
                    report.downcast_ref::<StorePathError>(),
                    Some(StorePathError::EmptySegment { .. })
                ),
                "{} lost the reason underneath", call
            );
        }

        prop_assert_eq!(
            store.scan_prefix(StorePath::root()).unwrap(),
            before,
            "a refused path changed the store"
        );
    });
}

/// 16. Bytes that will not decode as the type asked for are a
///     `StorageError::Codec` failure, not an absent value.
///
/// The failure half of the read, and the half a caller acts on: handing back
/// `None` would be worse than an error, because a field would then seed its
/// default over data that is still on disk. Which operation the report names
/// outermost is the statement being pinned - two engines that disagree about
/// that cannot be told apart by a caller matching on the context.
fn bytes_that_will_not_decode_are_a_codec_failure(backend: Backend) {
    let file = TempPath::new("conf_codec");
    let store = open(backend, &file);

    store
        .set(["cfg", "name"], &"not a number".to_string())
        .unwrap();

    let refusal = store.get::<u32>(["cfg", "name"]);
    assert!(
        refusal.is_err(),
        "a string read as a number gave {:?}",
        refusal.unwrap()
    );
    let report = refusal.unwrap_err();
    assert_eq!(
        report.current_context(),
        &StorageError::Read,
        "the outermost context names what the caller asked for: {report:?}"
    );
    assert!(
        report.contains::<amethystate::errors::CodecError>(),
        "the codec's refusal is the cause underneath: {report:?}"
    );
}

/// 17. A path that holds nothing reads as `Ok(None)`: absence is not a failure.
fn an_absent_path_reads_as_nothing_rather_than_failing(backend: Backend) {
    proptest!(config(), |(probe in path())| {
        let file = TempPath::new("conf_absent");
        let store = open(backend, &file);
        let probe = StorePath::from_segments(&probe);

        let read = store.get::<u32>(&probe);
        prop_assert!(read.is_ok(), "reading {} failed: {:?}", probe, read.unwrap_err());
        prop_assert_eq!(read.unwrap(), None);
    });
}

/// 18. A map refuses `update` on a key it does not hold with
///     `WriteError::KeyNotFound`, and refuses a key that cannot be a level with
///     `WriteError::Path`; `insert` is the call that adds a key.
fn a_map_refuses_a_key_it_does_not_hold_and_a_key_that_is_not_a_name(backend: Backend) {
    let file = TempPath::new("conf_map_errors");
    let store = open(backend, &file);
    let map = store.kv().map::<String, u32>("m").unwrap();

    let refusal = map.update("absent", &1).unwrap_err();
    assert!(
        matches!(refusal.current_context(), WriteError::KeyNotFound(key) if key == "absent"),
        "{refusal:?}"
    );

    map.insert("absent".to_string(), &1).unwrap();
    map.update("absent", &2).unwrap();
    assert_eq!(map.get("absent"), Some(2));

    let refusal = map.insert(String::new(), &1).unwrap_err();
    assert!(
        matches!(refusal.current_context(), WriteError::Path),
        "{refusal:?}"
    );
    assert!(
        matches!(
            refusal.downcast_ref::<StorePathError>(),
            Some(StorePathError::EmptySegment { .. })
        ),
        "{refusal:?}"
    );
    assert_eq!(map.len(), 1, "the refused key was not added");
}

/// 19. Golden: one fixed set of names comes back in one exact order, the same
///     order on every engine.
///
/// Not a property - which order is arbitrary, and this pins the one chosen so
/// that two engines cannot drift apart on it. Every name is one level, so the
/// list is about ordering alone: `a.b` is a single name holding the separator,
/// and it has to land between `a` and `ab` because the escape sorts before an
/// ordinary letter.
fn the_order_keys_come_back_in(backend: Backend) {
    let file = TempPath::new("conf_order");
    let store = open(backend, &file);

    for name in ["ab", "\u{e9}", "9", "a", "B", "a.b", "10"] {
        store.set([name], &1u32).unwrap();
    }

    let listed = store.scan_keys(StorePath::root()).unwrap();
    assert_eq!(
        listed.iter().map(StorePath::as_str).collect::<Vec<_>>(),
        ["10", "9", "B", "a", "a\\.b", "ab", "\u{e9}"]
    );
}

/// 20. Degenerate: an empty store lists nothing at any prefix, and is still
///     empty after a reopen.
fn an_empty_store_lists_nothing(backend: Backend) {
    let file = TempPath::new("conf_empty");

    {
        let store = open(backend, &file);

        assert_eq!(
            store.scan_keys(StorePath::root()).unwrap(),
            Vec::<StorePath>::new()
        );
        assert_eq!(store.scan_prefix(StorePath::root()).unwrap(), Vec::new());
        assert_eq!(
            store.scan_keys(["nothing"]).unwrap(),
            Vec::<StorePath>::new()
        );
        assert_eq!(store.get::<u32>(["nothing"]).unwrap(), None);

        store.delete_prefix(StorePath::root()).unwrap();
        assert_eq!(
            store.scan_keys(StorePath::root()).unwrap(),
            Vec::<StorePath>::new()
        );

        store.flush_prefix(StorePath::root()).unwrap();
    }

    let store = open(backend, &file);
    assert_eq!(
        store.scan_keys(StorePath::root()).unwrap(),
        Vec::<StorePath>::new()
    );
}

/// 21. Degenerate: a level named `.` is an ordinary level - it addresses one
///     value, not the whole store.
///
/// The name that the general strategies leave out, asked here on its own so
/// that its answer is one failure rather than a failure in every property.
fn a_level_named_dot_is_an_ordinary_level(backend: Backend) {
    let file = TempPath::new("conf_dot");
    let store = open(backend, &file);

    store.set(["cfg", "width"], &1280u32).unwrap();

    let wrote = store.set(["."], &7u32);
    assert!(
        wrote.is_ok(),
        "a level named `.` is a name like any other: {:?}",
        wrote.unwrap_err()
    );

    assert_eq!(
        store.get::<u32>(["cfg", "width"]).ok(),
        Some(Some(1280)),
        "writing at a level named `.` destroyed an unrelated value"
    );
    assert_eq!(
        store.get::<u32>(["."]).ok(),
        Some(Some(7)),
        "a level named `.` holds what was written at it"
    );

    store.delete(["."]).unwrap();
    assert_eq!(
        store.get::<u32>(["."]).ok(),
        Some(None),
        "the level named `.` is still there after deleting it"
    );
    assert_eq!(store.get::<u32>(["cfg", "width"]).ok(), Some(Some(1280)));
}

/// A namespace is uninitialized until it is marked, and marking one says
/// nothing about any other.
///
/// This is what stops a map re-seeding its defaults over entries the user has
/// since removed, so it is a statement about a store rather than about the
/// engine that happens to keep the marker in a table or in a second file.
fn a_namespace_is_uninitialized_until_it_is_marked(backend: Backend) {
    let file = TempPath::new("conf_initialized");
    let store = open(backend, &file);

    assert!(!store.is_initialized(&ns("settings")).unwrap());
    assert!(!store.is_initialized(&ns("ui")).unwrap());

    store.mark_initialized(&ns("settings")).unwrap();

    assert!(store.is_initialized(&ns("settings")).unwrap());
    assert!(
        !store.is_initialized(&ns("ui")).unwrap(),
        "marking one namespace marked another"
    );
}

/// Each kind of subscription hears exactly what it asked for.
///
/// `Any` hears every write, `ExactPath` only its own, and `Prefix` only what is
/// under it - and a prefix is matched by level, so `ui.theme` does not hear
/// `ui.themes.dark`.
fn a_subscription_hears_what_its_kind_asked_for(backend: Backend) {
    let file = TempPath::new("conf_subscribe");
    let store = open(backend, &file);

    let heard = |kind: SubscriptionKind| {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        store.subscribe(
            kind,
            Arc::new(move |event| sink.lock().unwrap().push(event.path.to_string())),
        );
        seen
    };

    let any = heard(SubscriptionKind::Any);
    let exact = heard(SubscriptionKind::ExactPath(StorePath::from_segments([
        "ui", "theme", "dark",
    ])));
    let prefix = heard(SubscriptionKind::Prefix(StorePath::from_segments([
        "ui", "theme",
    ])));

    store.set(["ui", "theme", "dark"], &true).unwrap();
    store.set(["ui", "layout", "width"], &260u64).unwrap();
    store.set(["ui", "themes", "dark"], &true).unwrap();

    assert_eq!(any.lock().unwrap().len(), 3, "Any hears every write");
    assert_eq!(
        exact.lock().unwrap().as_slice(),
        ["ui.theme.dark"],
        "ExactPath hears only its own path"
    );
    assert_eq!(
        prefix.lock().unwrap().as_slice(),
        ["ui.theme.dark"],
        "Prefix is matched by level, so `ui.theme` does not hear `ui.themes.dark`"
    );
}

/// A dropped subscription stops hearing.
fn a_dropped_subscription_stops_hearing(backend: Backend) {
    let file = TempPath::new("conf_unsubscribe");
    let store = open(backend, &file);

    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let id = store.subscribe(
        SubscriptionKind::Any,
        Arc::new(move |event| sink.lock().unwrap().push(event.path.to_string())),
    );

    store.set(["ui", "theme"], &"dark".to_string()).unwrap();
    store.unsubscribe(id);
    store.set(["ui", "theme"], &"light".to_string()).unwrap();

    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Every event a store emits, in order.
fn events(store: &Store) -> Arc<Mutex<Vec<StoreEvent>>> {
    let seen: Arc<Mutex<Vec<StoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    store.subscribe(
        SubscriptionKind::Any,
        Arc::new(move |event| sink.lock().unwrap().push(event.clone())),
    );
    seen
}

/// What an event's bytes say, read in the engine's own format.
fn decoded<T: serde::de::DeserializeOwned>(store: &Store, bytes: &Option<Vec<u8>>) -> Option<T> {
    bytes
        .as_ref()
        .map(|raw| store.decode::<T>(raw).expect("the event's bytes decode"))
}

/// 26. A write says what changed: one `Set` at the path written, `new` holding
///     the value that landed and `old` the one it replaced - and nothing for
///     `old` where there was nothing to replace.
fn a_write_emits_one_set_carrying_both_values(backend: Backend) {
    let file = TempPath::new("conf_event_set");
    let store = open(backend, &file);
    let seen = events(&store);

    store.set(["ui", "width"], &10u32).unwrap();
    store.set(["ui", "width"], &20u32).unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "one event per write, got {seen:?}");

    assert_eq!(seen[0].op, StoreOp::Set);
    assert_eq!(seen[0].path.as_str(), "ui.width");
    assert!(seen[0].old.is_none(), "nothing was there to replace");
    assert_eq!(decoded::<u32>(&store, &seen[0].new), Some(10));

    assert_eq!(seen[1].op, StoreOp::Set);
    assert_eq!(
        decoded::<u32>(&store, &seen[1].old),
        Some(10),
        "the value the second write replaced"
    );
    assert_eq!(decoded::<u32>(&store, &seen[1].new), Some(20));
}

/// 27. A delete says what went: one `Delete` carrying the value that was there
///     and no new one. A delete that removed nothing says nothing at all - an
///     event for a removal that did not happen is a change subscribers act on.
fn a_delete_emits_one_delete_and_only_when_something_went(backend: Backend) {
    let file = TempPath::new("conf_event_delete");
    let store = open(backend, &file);

    store.set(["ui", "width"], &10u32).unwrap();

    let seen = events(&store);
    store.delete(["ui", "width"]).unwrap();
    store.delete(["ui", "height"]).unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        1,
        "only the delete that removed something speaks, got {seen:?}"
    );
    assert_eq!(seen[0].op, StoreOp::Delete);
    assert_eq!(seen[0].path.as_str(), "ui.width");
    assert_eq!(
        decoded::<u32>(&store, &seen[0].old),
        Some(10),
        "the value that went"
    );
    assert!(seen[0].new.is_none(), "a delete leaves nothing behind");
}

/// 28. `delete_prefix` is one operation, so it is one `DeletePrefix` at the
///     prefix rather than a `Delete` for each key underneath.
fn delete_prefix_emits_one_event_at_the_prefix(backend: Backend) {
    let file = TempPath::new("conf_event_delete_prefix");
    let store = open(backend, &file);

    store.set(["ui", "a"], &1u32).unwrap();
    store.set(["ui", "b"], &2u32).unwrap();
    store.set(["ui", "c", "d"], &3u32).unwrap();

    let seen = events(&store);
    store.delete_prefix(["ui"]).unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one operation, one event, got {seen:?}");
    assert_eq!(seen[0].op, StoreOp::DeletePrefix);
    assert_eq!(seen[0].path.as_str(), "ui");
}

/// The store's own bookkeeping is not data.
///
/// Marking a namespace initialized must not put anything a scan of that
/// namespace will return, whether the engine keeps the marker in a table, a
/// second file, or beside the data.
fn the_initialization_marker_is_not_listed_as_data(backend: Backend) {
    let file = TempPath::new("conf_init_marker");
    let store = open(backend, &file);

    store.mark_initialized(&ns("settings")).unwrap();
    store
        .set(["settings", "host"], &"localhost".to_string())
        .unwrap();

    let keys = store.scan_keys(["settings"]).unwrap();

    assert_eq!(
        keys.iter().map(StorePath::as_str).collect::<Vec<_>>(),
        ["settings.host"],
        "a scan returned bookkeeping"
    );
}

/// The statements, once. Each one names the engine the module it lands in was
/// built for, so the same sentence is asked of whichever engine the build has.
macro_rules! conformance_suite {
    () => {
        #[test]
        fn a_value_reads_back_where_it_was_written() {
            super::a_value_reads_back_where_it_was_written(BACKEND);
        }

        #[test]
        fn a_write_leaves_every_other_path_alone() {
            super::a_write_leaves_every_other_path_alone(BACKEND);
        }

        #[test]
        fn an_ancestor_is_not_a_value() {
            super::an_ancestor_is_not_a_value(BACKEND);
        }

        #[test]
        fn the_last_write_is_the_one_that_reads_back() {
            super::the_last_write_is_the_one_that_reads_back(BACKEND);
        }

        #[test]
        fn writing_then_deleting_leaves_the_store_as_it_was() {
            super::writing_then_deleting_leaves_the_store_as_it_was(BACKEND);
        }

        #[test]
        fn deleting_what_is_not_there_changes_nothing() {
            super::deleting_what_is_not_there_changes_nothing(BACKEND);
        }

        #[test]
        fn a_scan_lists_exactly_what_is_under_the_prefix() {
            super::a_scan_lists_exactly_what_is_under_the_prefix(BACKEND);
        }

        #[test]
        fn scan_keys_and_scan_prefix_agree() {
            super::scan_keys_and_scan_prefix_agree(BACKEND);
        }

        #[test]
        fn a_scan_comes_back_sorted() {
            super::a_scan_comes_back_sorted(BACKEND);
        }

        #[test]
        fn every_key_a_scan_returns_is_a_path_under_the_prefix() {
            super::every_key_a_scan_returns_is_a_path_under_the_prefix(BACKEND);
        }

        #[test]
        fn delete_prefix_takes_the_subtree_and_nothing_beside_it() {
            super::delete_prefix_takes_the_subtree_and_nothing_beside_it(BACKEND);
        }

        #[test]
        fn a_name_holding_the_separator_stays_one_level() {
            super::a_name_holding_the_separator_stays_one_level(BACKEND);
        }

        #[test]
        fn a_leaf_and_a_branch_coexist_at_one_name() {
            super::a_leaf_and_a_branch_coexist_at_one_name(BACKEND);
        }

        #[test]
        fn a_reopen_gives_back_what_was_committed() {
            super::a_reopen_gives_back_what_was_committed(BACKEND);
        }

        #[test]
        fn a_map_agrees_with_itself_and_with_a_scan() {
            super::a_map_agrees_with_itself_and_with_a_scan(BACKEND);
        }

        #[test]
        fn a_level_with_no_name_is_refused_and_changes_nothing() {
            super::a_level_with_no_name_is_refused_and_changes_nothing(BACKEND);
        }

        #[test]
        fn bytes_that_will_not_decode_are_a_codec_failure() {
            super::bytes_that_will_not_decode_are_a_codec_failure(BACKEND);
        }

        #[test]
        fn an_absent_path_reads_as_nothing_rather_than_failing() {
            super::an_absent_path_reads_as_nothing_rather_than_failing(BACKEND);
        }

        #[test]
        fn a_map_refuses_a_key_it_does_not_hold_and_a_key_that_is_not_a_name() {
            super::a_map_refuses_a_key_it_does_not_hold_and_a_key_that_is_not_a_name(BACKEND);
        }

        #[test]
        fn the_order_keys_come_back_in() {
            super::the_order_keys_come_back_in(BACKEND);
        }

        #[test]
        fn an_empty_store_lists_nothing() {
            super::an_empty_store_lists_nothing(BACKEND);
        }

        #[test]
        fn a_level_named_dot_is_an_ordinary_level() {
            super::a_level_named_dot_is_an_ordinary_level(BACKEND);
        }

        #[test]
        fn a_namespace_is_uninitialized_until_it_is_marked() {
            super::a_namespace_is_uninitialized_until_it_is_marked(BACKEND);
        }

        #[test]
        fn a_subscription_hears_what_its_kind_asked_for() {
            super::a_subscription_hears_what_its_kind_asked_for(BACKEND);
        }

        #[test]
        fn a_dropped_subscription_stops_hearing() {
            super::a_dropped_subscription_stops_hearing(BACKEND);
        }

        #[test]
        fn the_initialization_marker_is_not_listed_as_data() {
            super::the_initialization_marker_is_not_listed_as_data(BACKEND);
        }

        #[test]
        fn a_write_emits_one_set_carrying_both_values() {
            super::a_write_emits_one_set_carrying_both_values(BACKEND);
        }

        #[test]
        fn a_delete_emits_one_delete_and_only_when_something_went() {
            super::a_delete_emits_one_delete_and_only_when_something_went(BACKEND);
        }

        #[test]
        fn delete_prefix_emits_one_event_at_the_prefix() {
            super::delete_prefix_emits_one_event_at_the_prefix(BACKEND);
        }
    };
}

once_per_engine! {
    conformance_suite!();
}
