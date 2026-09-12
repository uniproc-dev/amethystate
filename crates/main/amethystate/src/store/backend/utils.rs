use crate::SubscriptionKind;
use crate::store::debouncer::Debouncer;
use crate::store::durable::PersistHealth;
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::Facts;
use crate::store::{StoreEvent, SubscriptionEntry};
#[cfg(any(feature = "redb", feature = "sqlite"))]
use amethystate_core::path::Key;
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
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
/// What the closing flush did, kept for the closes that come after it.
///
/// Only the first close flushes; every one after finds the thread stopped and
/// has nothing left to do. Answering `Ok` there says the buffer landed, which
/// is a lie wherever it did not - and `Drop` calls close after the caller
/// already has, so the lie is the ordinary case rather than a corner.
#[derive(Default)]
pub struct Closed(parking_lot::Mutex<Option<(StorageError, String)>>);

impl Closed {
    /// Records what the closing flush did and hands it back unchanged.
    pub fn settled(&self, outcome: StorageResult<()>) -> StorageResult<()> {
        if let Err(why) = &outcome {
            *self.0.lock() = Some((
                *why.current_context(),
                amethystate_core::failure::one_line(why),
            ));
        }

        outcome
    }

    /// The same answer again, for a close that found the store already closed.
    pub fn again(&self, file: &Path) -> StorageResult<()> {
        match &*self.0.lock() {
            None => Ok(()),
            Some((kind, said)) => Err(Report::new(*kind)
                .attach(crate::store::facts::StoreFile(file.to_path_buf()))
                .attach(format!("the closing flush had already failed: {said}"))),
        }
    }
}

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

/// Takes the store's files away, where the open that just failed was told to
/// start fresh, and says whether it is worth reading again.
///
/// Called by an engine at the point it finds out its own files will not read,
/// and nowhere else: everything before that point is the store as the last run
/// left it, and everything after has this run's writing in it.
///
/// `false` where nothing was asked of it, and where a file would not go - the
/// caller then fails with what it already had, which is the more useful of the
/// two failures.
pub(crate) fn start_fresh(
    config: &crate::store::config::StoreConfig,
    backend: crate::store::builder::Backend,
    why: &Report<StorageError>,
) -> bool {
    use crate::store::{StoreLayout, WillNotOpen};

    if config.will_not_open != WillNotOpen::StartFresh {
        return false;
    }

    let going = StoreLayout::of(&config.path, backend).present();

    tracing::warn!(
        files = ?going,
        reason = %crate::store::one_line(why),
        "the store would not open and was told to start fresh, so what is there is being \
         taken away and an empty store opened in its place"
    );

    for file in going {
        if let Err(io) = std::fs::remove_file(&file) {
            tracing::error!(
                file = %file.display(),
                error = %io,
                "starting fresh was asked for and this file would not go, so the store is \
                 refused as it would have been"
            );
            return false;
        }
    }

    true
}

/// A key read back out of storage, as the path it claims to be.
///
/// Every key a scan hands back is one this library could have written, so this
/// fails only where something else did the writing - an older build, or a hand
/// edit. Failing names the key rather than dropping it, since a key nothing can
/// address is worse unsaid.
#[cfg(any(feature = "redb", feature = "sqlite"))]
pub fn stored_path(key: &[u8]) -> StorageResult<StorePath> {
    Key::from_bytes(key)
        .path()
        .change_context(StorageError::Scan)
        .attach_raw_key(&String::from_utf8_lossy(key))
        .attach("the store holds a key this library could not have written")
}

/// Where a row of kind `kind` about `path` sits in the bookkeeping table.
///
/// The kind is a level of its own and always the first, drawn from a closed
/// set - `meta`, `init`, `format`. That is what keeps the kinds apart: two rows
/// meet only where the kind *and* the rest of the path are the same, so no name
/// a caller writes can reach another kind's row. Key a row by the bare path
/// instead, as this did for `meta`, and a component declared at `init.foo`
/// lands exactly where the marker for the namespace `foo` lives.
///
/// The same shape the text engines lay their sidecar out with, and for the same
/// reason.
#[cfg(any(feature = "redb", feature = "sqlite"))]
pub fn bookkeeping_at(kind: &str, path: &StorePath) -> StorePath {
    StorePath::segment(kind).join(path)
}

/// [`bookkeeping_at`] encoded, for the engines that address by bytes.
#[cfg(any(feature = "redb", feature = "sqlite"))]
pub fn bookkeeping_key(kind: &str, path: &StorePath) -> Key {
    bookkeeping_at(kind, path).key()
}

/// The key a namespace's initialization marker is stored under.
#[cfg(any(feature = "redb", feature = "sqlite"))]
pub fn init_key(namespace: &StorePath) -> Key {
    bookkeeping_key("init", namespace)
}

/// Turns down a close asked for from inside `on_persist_failure`.
///
/// That callback runs on the thread a close waits for, so going ahead would
/// wait for the caller to return - which it cannot do until the close it is
/// waiting on comes back.
pub fn refuse_closing_from_a_flush() -> StorageResult<()> {
    if crate::store::debouncer::Saving::here() {
        return Err(Report::new(StorageError::Reentrant).attach(
            "`on_persist_failure` runs on the thread that saves, and closing waits for that \
             thread. Report the failure from the callback and close the store from wherever it \
             is held",
        ));
    }
    Ok(())
}

/// Tells everyone subscribed to `event`, and hands back what they said.
///
/// Every subscriber is told, whatever the ones before it answered: they are
/// separate readers of the same change, and stopping at the first would leave
/// the rest holding a value nobody replaced. What comes back is the first
/// refusal, carrying how many there were.
pub fn emit_events(
    subs_lock: &RwLock<Vec<SubscriptionEntry>>,
    event: StoreEvent,
) -> StorageResult<()> {
    let callbacks = {
        let guard = subs_lock.read();
        guard
            .iter()
            .filter(|s| matches_kind(&s.kind, &event.path))
            .map(|s| s.callback.clone())
            .collect::<Vec<_>>()
    };

    let mut refused: Option<Report<StorageError>> = None;
    let mut also = 0usize;

    for cb in callbacks {
        if let Err(why) = cb(&event) {
            match &refused {
                None => refused = Some(why),
                Some(_) => also += 1,
            }
        }
    }

    match refused {
        None => Ok(()),
        Some(first) if also == 0 => Err(first),
        Some(first) => Err(first.attach(format!("{also} other subscribers refused it too"))),
    }
}

fn matches_kind(kind: &SubscriptionKind, path: &StorePath) -> bool {
    match kind {
        SubscriptionKind::Any => true,
        SubscriptionKind::ExactPath(p) => p == path,
        SubscriptionKind::Prefix(prefix) => path.starts_with(prefix),
    }
}

#[cfg(any(feature = "redb", feature = "sqlite"))]
mod buffered {
    use crate::StorageResult;
    use crate::store::config::StoreConfig;
    use crate::store::debouncer::{Debouncer, FlushPolicy};
    use crate::store::durable::{CommitSignal, PersistHealth};
    use amethystate_core::path::StorePath;
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// One buffered write, waiting for the next flush.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum PendingOp {
        Set(Vec<u8>),
        Delete,
    }

    impl PendingOp {
        /// The value a reader should see, or `None` where the key is gone.
        pub fn value(&self) -> Option<&[u8]> {
            match self {
                Self::Set(bytes) => Some(bytes),
                Self::Delete => None,
            }
        }
    }

    /// Everything written since the last flush landed: the values, and which
    /// namespaces have been seeded.
    ///
    /// The two are held apart by construction rather than by a name, which is
    /// the only way they can be held apart at all. A marker is about a
    /// namespace and a value is about a path, and any level name is legal in
    /// both - so the moment they share a key space, `set(["cfg"])` and marking
    /// `cfg` are one entry and whichever came last is the only one that
    /// reaches the disk.
    ///
    /// They stay in one buffer, though, because that is what makes a marker
    /// land in the same transaction as the values it vouches for.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Pending {
        at: std::collections::HashMap<StorePath, PendingOp>,
        marking: std::collections::HashMap<StorePath, bool>,
    }

    impl Pending {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn is_empty(&self) -> bool {
            self.at.is_empty() && self.marking.is_empty()
        }

        /// How much the buffer is holding, markers counted with the values -
        /// it is a size for a report, not an index into anything.
        pub fn len(&self) -> usize {
            self.at.len() + self.marking.len()
        }

        #[cfg(test)]
        pub fn holds(&self, path: &StorePath) -> bool {
            self.at.contains_key(path)
        }

        /// What is buffered for `path`, which is a value and never a marker.
        pub fn get(&self, path: &StorePath) -> Option<&PendingOp> {
            self.at.get(path)
        }

        pub fn insert(&mut self, path: StorePath, op: PendingOp) {
            self.at.insert(path, op);
        }

        /// Every path a value is buffered at, with what is buffered there.
        pub fn values(&self) -> impl Iterator<Item = (&StorePath, &PendingOp)> {
            self.at.iter()
        }

        /// Whether `namespace` has been marked since the last flush, and how.
        pub fn marked(&self, namespace: &StorePath) -> Option<bool> {
            self.marking.get(namespace).copied()
        }

        pub fn mark(&mut self, namespace: StorePath, seeded: bool) {
            self.marking.insert(namespace, seeded);
        }

        pub fn markings(&self) -> impl Iterator<Item = (&StorePath, bool)> {
            self.marking.iter().map(|(at, seeded)| (at, *seeded))
        }

        /// Everything written and not yet saved, as the caller named it.
        ///
        /// A marker names the namespace it is about, which is a path the
        /// caller wrote to as surely as a value's is.
        pub fn unsaved(&self) -> Vec<StorePath> {
            self.at.keys().chain(self.marking.keys()).cloned().collect()
        }
    }

    /// What a flush is to carry, or `None` where there is nothing to write.
    ///
    /// Copied rather than taken, for the reason [`clear_committed`] gives: the
    /// buffer is emptied of exactly what landed, once it has landed. The lock
    /// is released before the flush, which is what makes the copy necessary
    /// rather than merely convenient.
    pub fn buffered(pending: &Mutex<Pending>) -> Option<Pending> {
        let held = pending.lock();
        match held.is_empty() {
            true => None,
            false => Some(held.clone()),
        }
    }

    /// Everything the flush thread needs beyond the write itself, which is the
    /// same for every engine buffering into a [`Pending`].
    pub fn flushing(
        config: &StoreConfig,
        commits: &Arc<CommitSignal>,
        health: &Arc<PersistHealth>,
        pending: &Arc<Mutex<Pending>>,
    ) -> FlushPolicy {
        let held = pending.clone();

        FlushPolicy {
            retry: config.retry_policy.clone(),
            commits: commits.clone(),
            health: health.clone(),
            on_giveup: config.on_persist_failure.clone(),
            unsaved: Arc::new(move || held.lock().unsaved()),
        }
    }

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

        let mut found = Pending::new();

        for (key, op) in pending.values() {
            if key.starts_with(prefix) {
                found.insert(key.clone(), op.clone());
            }
        }

        for (namespace, seeded) in pending.markings() {
            if namespace.starts_with(prefix) {
                found.mark(namespace.clone(), seeded);
            }
        }

        found
    }

    /// Drops from the buffer exactly what was committed.
    ///
    /// A key whose buffered value has changed since is a write that landed while
    /// the commit was in flight; it is not on disk, so it stays for the next one.
    pub fn clear_committed(pending: &mut Pending, committed: &Pending) {
        for (key, value) in committed.values() {
            if pending.at.get(key) == Some(value) {
                pending.at.remove(key);
            }
        }

        for (namespace, seeded) in committed.markings() {
            if pending.marking.get(namespace) == Some(&seeded) {
                pending.marking.remove(namespace);
            }
        }
    }

    /// Buffers a write made through the inspector, which is how a migration
    /// step writes.
    ///
    /// Nobody is told. A migration runs inside `open`, before the reactive
    /// layer exists, so the subscriber list it would be told through is empty
    /// by construction.
    pub fn set_raw_pending(
        pending: &Mutex<Pending>,
        debouncer: &Debouncer,
        key: &StorePath,
        value: &[u8],
    ) -> StorageResult<()> {
        {
            let mut lock = pending.lock();
            lock.insert(key.clone(), PendingOp::Set(value.to_vec()));
        }
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
        let mut held = Pending::new();

        for (k, v) in entries {
            held.insert(
                path(k),
                match v {
                    Some(b) => PendingOp::Set(b.to_vec()),
                    None => PendingOp::Delete,
                },
            );
        }

        held
    }

    #[test]
    fn what_is_taken_carries_the_op_it_was_buffered_with() {
        let pending = buffer(&[("a.x", Some(b"1")), ("a.y", None)]);

        let taken = pending_prefix(&pending, &path("a"));

        assert_eq!(
            taken.get(&path("a.x")),
            Some(&PendingOp::Set(b"1".to_vec()))
        );
        assert_eq!(
            taken.get(&path("a.y")),
            Some(&PendingOp::Delete),
            "a buffered delete that came back as a write would resurrect the key"
        );
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

        assert!(taken.holds(&path("a")));
        assert!(taken.holds(&path("a.x")));
        assert!(!taken.holds(&path("ab")), "a prefix is not a substring");
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
            pending.get(&path("a.x")),
            Some(&PendingOp::Set(b"new".to_vec())),
            "the newer write is not on disk, so dropping it would lose it"
        );
    }

    #[test]
    fn a_key_written_after_the_commit_survives() {
        let committed = buffer(&[("a.x", Some(b"1"))]);
        let mut pending = buffer(&[("a.x", Some(b"1")), ("a.z", Some(b"9"))]);

        clear_committed(&mut pending, &committed);

        assert!(!pending.holds(&path("a.x")));
        assert!(pending.holds(&path("a.z")), "it was never committed");
    }

    #[test]
    fn a_pending_delete_is_committed_like_any_other_entry() {
        let mut pending = buffer(&[("a.x", None)]);
        let committed = pending.clone();

        clear_committed(&mut pending, &committed);

        assert!(pending.is_empty());
    }

    fn stored(entries: &[(&str, &[u8])]) -> Vec<(StorePath, Vec<u8>)> {
        entries.iter().map(|(k, v)| (path(k), v.to_vec())).collect()
    }

    fn pending(entries: &[(&str, Option<&[u8]>)]) -> Vec<(StorePath, Option<Vec<u8>>)> {
        entries
            .iter()
            .map(|(k, v)| (path(k), v.map(<[u8]>::to_vec)))
            .collect()
    }

    fn names(merged: &[(StorePath, Vec<u8>)]) -> Vec<String> {
        merged.iter().map(|(k, _)| k.to_string()).collect()
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
            vec![(path("a"), b"new".to_vec()), (path("b"), b"kept".to_vec()),]
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
                .map(|(k, v)| (k.to_string(), v))
                .collect();
            let want: Vec<(String, Vec<u8>)> = expected.into_iter().collect();
            proptest::prop_assert_eq!(got, want);
        }
    }
}
