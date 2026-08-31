use crate::SubscriptionKind;
use crate::store::durable::PersistHealth;
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::Facts;
use crate::store::debouncer::Debouncer;
use crate::store::{StoreEvent, SubscriptionEntry};
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use parking_lot::RwLock;
use std::path::Path;

#[cfg(any(feature = "redb", feature = "sqlite"))]
pub use buffered::*;

pub trait Attempted: ResultExt {
    fn doing(self, what: StorageError, file: &Path) -> StorageResult<Self::Ok>;
}

impl<R: ResultExt> Attempted for R {
    fn doing(self, what: StorageError, file: &Path) -> StorageResult<Self::Ok> {
        self.change_context(what).attach_store_file(file)
    }
}

/// Refuses a write while the background flush is not landing, and once the
/// store has been closed.
///
/// A stopped debouncer is what closing leaves behind, and a write reaching a
/// stopped one would be buffered by a store that has nothing left to write it
/// with. Refusing here is what makes that a `Closed` rather than a value that
/// was accepted and never appeared.
pub fn check_debouncer(health: &PersistHealth, debouncer: &Debouncer) -> StorageResult<()> {
    if debouncer.is_stopped() {
        return Err(error_stack::Report::new(StorageError::Closed));
    }
    if let Some(reason) = health.failure() {
        return Err(error_stack::Report::new(StorageError::CommitFailed)
            .attach(format!("the background flush is not landing: {reason:#}"))
            .attach("what is already buffered is still being retried, and reads are unaffected"));
    }
    if debouncer.is_poisoned() {
        panic!("debouncer thread is dead — store integrity cannot be guaranteed");
    }
    Ok(())
}

/// Reports what a store's closing flush did, from the `Drop` where nothing
/// else can.
///
/// That flush is the one a short-lived process depends on, and the one whose
/// failure nobody is in a position to see: a locked file, a full disk, a
/// permission error on the way out, and the process ends reporting success
/// with the data not written. `Drop` cannot return an error and cannot be
/// given a caller to hand one to, so a log line is the whole of what the loss
/// can leave behind - which is why it is at `error` rather than `warn`. A
/// caller that would rather find out while it can still act calls `save_now`
/// or `close` and reads the result.
pub fn report_closing_flush(outcome: StorageResult<()>, file: &Path) {
    if let Err(report) = outcome {
        tracing::error!(
            target: "amethystate",
            file = %file.display(),
            error = ?report,
            "the store's closing flush failed: what it still held is not on disk",
        );
    }
}

/// Lays the write buffer over what the engine holds, both already sorted: a
/// buffered write replaces the committed value at its key, a buffered delete
/// leaves nothing there, and the order is the engine's.
#[cfg(any(feature = "redb", feature = "sqlite"))]
pub fn merge_buffered(
    committed: Vec<(StorePath, Vec<u8>)>,
    buffered: Vec<(StorePath, Option<Vec<u8>>)>,
) -> Vec<(StorePath, Vec<u8>)> {
    if buffered.is_empty() {
        return committed;
    }

    let mut out = Vec::with_capacity(committed.len() + buffered.len());
    let mut left = committed.into_iter().peekable();
    let mut right = buffered.into_iter().peekable();

    loop {
        let take_left = match (left.peek(), right.peek()) {
            (Some((a, _)), Some((b, _))) => a <= b,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };

        if take_left {
            let (key, value) = left.next().expect("peeked");
            if right.peek().is_some_and(|(b, _)| *b == key) {
                continue;
            }
            out.push((key, value));
        } else {
            let (key, value) = right.next().expect("peeked");
            if let Some(value) = value {
                out.push((key, value));
            }
        }
    }

    out
}

/// A key read back out of storage, as the path it claims to be.
///
/// Every key a scan hands back is one this library could have written, so this
/// fails only where something else did the writing - an older build, or a hand
/// edit. Failing names the key rather than dropping it, since a key nothing can
/// address is worse unsaid.
pub fn stored_path(key: &str) -> StorageResult<StorePath> {
    StorePath::parse_joined(key)
        .change_context(StorageError::Scan)
        .attach_raw_key(key)
        .attach("the store holds a key this library could not have written")
}

/// The key a namespace's initialization marker is stored under, in the same
/// table as data - redb and sqlite keep no table of their own for it.
#[cfg(any(feature = "redb", feature = "sqlite"))]
pub fn init_key(namespace: &str) -> String {
    format!("__init::{namespace}")
}

pub fn emit_events(subs_lock: &RwLock<Vec<SubscriptionEntry>>, event: StoreEvent) {
    let callbacks = {
        let guard = subs_lock.read();
        guard
            .iter()
            .filter(|s| matches_kind(&s.kind, &event.path))
            .map(|s| s.callback.clone())
            .collect::<Vec<_>>()
    };
    for cb in callbacks {
        cb(&event);
    }
}

fn matches_kind(kind: &SubscriptionKind, path: &StorePath) -> bool {
    match kind {
        SubscriptionKind::Any => true,
        SubscriptionKind::ExactPath(p) => p == path,
        SubscriptionKind::Prefix(prefix) => prefix.subtree().contains(path.as_str()),
    }
}

#[cfg(any(feature = "redb", feature = "sqlite"))]
mod buffered {
    use super::{SubscriptionEntry, emit_events};
    use crate::store::StoreEvent;
    use crate::store::debouncer::Debouncer;
    use crate::{StorageResult, StoreOp};
    use amethystate_core::path::StorePath;
    use parking_lot::{Mutex, RwLock};

    /// One buffered write, waiting for the next flush.
    ///
    /// `Init` targets the metadata table rather than the data one; keeping it
    /// in the same buffer is what makes a namespace flag land in the same
    /// transaction as the values it vouches for.
    ///
    /// It carries the flag rather than there being one variant per direction,
    /// so setting and clearing it stay one branch wherever it is handled - and
    /// there are four of those, two per flat engine.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum PendingOp {
        Set(Vec<u8>),
        Delete,
        Init(bool),
    }

    impl PendingOp {
        /// The value a reader should see, or `None` where the key is gone.
        pub fn value(&self) -> Option<&[u8]> {
            match self {
                Self::Set(bytes) => Some(bytes),
                Self::Delete | Self::Init(_) => None,
            }
        }

        pub fn is_data(&self) -> bool {
            matches!(self, Self::Set(_) | Self::Delete)
        }
    }

    pub type Pending = std::collections::HashMap<StorePath, PendingOp>;

    /// Everything buffered under `prefix`, left in place.
    ///
    /// The buffer is only cleared once the write has actually landed, by
    /// [`clear_committed`]. Taking entries out first meant any error below lost
    /// them: not on disk, not in memory, and nothing left to retry.
    pub fn pending_prefix(pending: &Pending, prefix: &StorePath) -> Pending {
        if pending.is_empty() {
            return Pending::new();
        }

        if prefix.is_root() {
            return pending.clone();
        }

        let subtree = prefix.subtree();
        pending
            .iter()
            .filter(|(key, _)| subtree.contains(key.as_str()))
            .map(|(key, op)| (key.clone(), op.clone()))
            .collect()
    }

    /// Drops from the buffer exactly what was committed.
    ///
    /// A key whose buffered value has changed since is a write that landed while
    /// the commit was in flight; it is not on disk, so it stays for the next one.
    pub fn clear_committed(pending: &mut Pending, committed: &Pending) {
        for (key, value) in committed {
            if pending.get(key) == Some(value) {
                pending.remove(key);
            }
        }
    }

    pub fn set_raw_pending(
        pending: &Mutex<Pending>,
        subscriptions: &RwLock<Vec<SubscriptionEntry>>,
        debouncer: &Debouncer,
        key: &StorePath,
        value: &[u8],
    ) -> StorageResult<()> {
        let old_bytes = {
            let lock = pending.lock();
            lock.get(key).and_then(|op| op.value().map(Vec::from))
        };
        {
            let mut lock = pending.lock();
            lock.insert(key.clone(), PendingOp::Set(value.to_vec()));
        }
        emit_events(
            subscriptions,
            StoreEvent {
                path: key.clone(),
                op: StoreOp::Set,
                old: old_bytes,
                new: Some(value.to_vec()),
                source: None,
            },
        );
        debouncer.schedule();
        Ok(())
    }
}

#[cfg(all(test, any(feature = "redb", feature = "sqlite")))]
mod tests {
    use super::*;

    fn path(joined: &str) -> StorePath {
        StorePath::parse_joined(joined).expect("a key the tests wrote themselves")
    }

    fn buffer(entries: &[(&str, Option<&[u8]>)]) -> Pending {
        entries
            .iter()
            .map(|(k, v)| {
                (
                    path(k),
                    match v {
                        Some(b) => PendingOp::Set(b.to_vec()),
                        None => PendingOp::Delete,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn collecting_leaves_the_buffer_alone() {
        let pending = buffer(&[("a.x", Some(b"1")), ("a.y", Some(b"2"))]);

        let taken = pending_prefix(&pending, &path("a"));

        assert_eq!(taken.len(), 2);
        assert_eq!(
            pending.len(),
            2,
            "entries must survive until the write lands, or a failure below \
             loses them from memory and disk both"
        );
    }

    #[test]
    fn the_root_means_everything() {
        let pending = buffer(&[("a.x", Some(b"1")), ("b.y", Some(b"2"))]);
        assert_eq!(pending_prefix(&pending, &StorePath::root()).len(), 2);
    }

    #[test]
    fn a_prefix_matches_its_own_key_and_its_children() {
        let pending = buffer(&[
            ("a", Some(b"root")),
            ("a.x", Some(b"child")),
            ("ab", Some(b"sibling")),
        ]);

        let taken = pending_prefix(&pending, &path("a"));

        assert!(taken.contains_key("a"));
        assert!(taken.contains_key("a.x"));
        assert!(!taken.contains_key("ab"), "a prefix is not a substring");
    }

    #[test]
    fn committed_entries_are_dropped() {
        let mut pending = buffer(&[("a.x", Some(b"1")), ("a.y", Some(b"2"))]);
        let committed = pending.clone();

        clear_committed(&mut pending, &committed);

        assert!(pending.is_empty());
    }

    #[test]
    fn a_value_that_changed_during_the_commit_survives() {
        let committed = buffer(&[("a.x", Some(b"old"))]);
        let mut pending = buffer(&[("a.x", Some(b"new"))]);

        clear_committed(&mut pending, &committed);

        assert_eq!(
            pending.get("a.x"),
            Some(&PendingOp::Set(b"new".to_vec())),
            "the newer write is not on disk, so dropping it would lose it"
        );
    }

    #[test]
    fn a_key_written_after_the_commit_survives() {
        let committed = buffer(&[("a.x", Some(b"1"))]);
        let mut pending = buffer(&[("a.x", Some(b"1")), ("a.z", Some(b"9"))]);

        clear_committed(&mut pending, &committed);

        assert!(!pending.contains_key("a.x"));
        assert!(pending.contains_key("a.z"), "it was never committed");
    }

    #[test]
    fn a_pending_delete_is_committed_like_any_other_entry() {
        let mut pending = buffer(&[("a.x", None)]);
        let committed = pending.clone();

        clear_committed(&mut pending, &committed);

        assert!(pending.is_empty());
    }

    fn stored(entries: &[(&str, &[u8])]) -> Vec<(StorePath, Vec<u8>)> {
        entries
            .iter()
            .map(|(k, v)| (path(k), v.to_vec()))
            .collect()
    }

    fn pending(entries: &[(&str, Option<&[u8]>)]) -> Vec<(StorePath, Option<Vec<u8>>)> {
        entries
            .iter()
            .map(|(k, v)| (path(k), v.map(<[u8]>::to_vec)))
            .collect()
    }

    fn names(merged: &[(StorePath, Vec<u8>)]) -> Vec<String> {
        merged.iter().map(|(k, _)| k.as_str().to_string()).collect()
    }

    #[test]
    fn an_empty_buffer_gives_back_what_the_engine_holds() {
        let committed = stored(&[("a", b"1"), ("b", b"2")]);
        assert_eq!(merge_buffered(committed.clone(), Vec::new()), committed);
    }

    #[test]
    fn an_empty_engine_side_gives_back_the_buffer_without_its_deletes() {
        let merged = merge_buffered(
            Vec::new(),
            pending(&[("a", Some(b"1")), ("b", None), ("c", Some(b"3"))]),
        );
        assert_eq!(names(&merged), ["a", "c"]);
    }

    #[test]
    fn the_two_lists_interleave_by_key() {
        let merged = merge_buffered(
            stored(&[("a", b"1"), ("c", b"3"), ("e", b"5")]),
            pending(&[("b", Some(b"2")), ("d", Some(b"4"))]),
        );
        assert_eq!(names(&merged), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn a_buffered_write_replaces_the_committed_value_at_that_key() {
        let merged = merge_buffered(
            stored(&[("a", b"old"), ("b", b"kept")]),
            pending(&[("a", Some(b"new"))]),
        );
        assert_eq!(
            merged,
            vec![
                (path("a"), b"new".to_vec()),
                (path("b"), b"kept".to_vec()),
            ]
        );
    }

    #[test]
    fn a_buffered_delete_takes_the_committed_value_with_it() {
        let merged = merge_buffered(
            stored(&[("a", b"1"), ("b", b"2"), ("c", b"3")]),
            pending(&[("b", None)]),
        );
        assert_eq!(names(&merged), ["a", "c"]);
    }

    #[test]
    fn a_delete_of_a_key_the_engine_never_had_adds_nothing() {
        let merged = merge_buffered(stored(&[("a", b"1")]), pending(&[("z", None)]));
        assert_eq!(names(&merged), ["a"]);
    }

    #[test]
    fn deleting_everything_leaves_nothing() {
        let merged = merge_buffered(
            stored(&[("a", b"1"), ("b", b"2")]),
            pending(&[("a", None), ("b", None)]),
        );
        assert!(merged.is_empty());
    }

    proptest::proptest! {
        #[test]
        fn it_answers_what_a_map_of_the_two_would(
            entries in proptest::collection::vec(
                (0u8..12, proptest::option::of(proptest::option::of(0u8..4))),
                0..24,
            ),
        ) {
            use std::collections::BTreeMap;

            let mut committed = BTreeMap::new();
            let mut buffered = BTreeMap::new();
            for (key, op) in entries {
                let key = format!("k{key:02}");
                match op {
                    None => { committed.insert(key, vec![0u8]); }
                    Some(value) => { buffered.insert(key, value.map(|v| vec![v])); }
                }
            }

            let mut expected: BTreeMap<String, Vec<u8>> = committed.clone();
            for (key, op) in &buffered {
                match op {
                    Some(value) => { expected.insert(key.clone(), value.clone()); }
                    None => { expected.remove(key); }
                }
            }

            let merged = merge_buffered(
                committed.into_iter().map(|(k, v)| (path(&k), v)).collect(),
                buffered.into_iter().map(|(k, v)| (path(&k), v)).collect(),
            );

            let got: Vec<(String, Vec<u8>)> = merged
                .into_iter()
                .map(|(k, v)| (k.as_str().to_string(), v))
                .collect();
            let want: Vec<(String, Vec<u8>)> = expected.into_iter().collect();
            proptest::prop_assert_eq!(got, want);
        }
    }
}
