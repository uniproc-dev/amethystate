//! What the store does when redb stops touching the disk.
//!
//! One real I/O error sets a flag in redb's own `CachedFile` that nothing in
//! the crate ever clears, and every call after it - reads included - is
//! answered with `PreviousIo` without the disk being touched at all. So a
//! retry against that handle can never land, however healthy the disk has
//! become, and the only recovery redb documents is to close the database and
//! open it again. This module is that: the swappable handle, the predicate
//! that decides when the handle is spent, and the trade itself.

use super::error;
use crate::StorageResult;
use crate::store::backend::utils::Attempted;
use crate::store::error::StorageError;
use error_stack::ResultExt;
use redb::Database;
use std::path::Path;
use std::sync::Arc;
use tracing::warn;

#[cfg(test)]
use std::io;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

/// The open database, or nothing while it is being replaced.
///
/// redb holds an exclusive lock on the file for as long as a `Database` is
/// alive, so a reopen has to drop the old handle before `Database::create` can
/// take the lock back. `None` is that gap, and it is short - a create, holding
/// the write lock.
///
/// Which is also why every holder reaches the database through this swap: a
/// clone kept anywhere - a background thread, a primitive - would hold the file
/// lock for its own lifetime and leave no moment when a reopen could happen.
pub(super) type OpenDatabase = Arc<arc_swap::ArcSwapOption<Database>>;

/// The one store file whose disk can be made to fail, named by path so the
/// breakage reaches that store alone while other tests open theirs in
/// parallel.
///
/// Armed before opening, since a `StorageBackend` can only be installed as the
/// database is built. A real `PreviousIo` can only come from redb's own I/O -
/// the flag lives in its `CachedFile` - so the hook lives here, beside the
/// open.
#[cfg(test)]
static FAILING_DISK: parking_lot::Mutex<Option<std::path::PathBuf>> = parking_lot::Mutex::new(None);

/// How many more writes that disk accepts, read live on every one - so a test
/// can take the disk away and give it back while the store is running, which
/// is the whole scenario.
#[cfg(test)]
pub(super) static WRITES_LEFT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Holds the failing disk armed, and puts it away on the way out - including
/// the way out through a panicking assertion, which a plain store-false at the
/// end of a test does not cover.
#[cfg(test)]
pub(super) struct ArmedDisk;

#[cfg(test)]
impl Drop for ArmedDisk {
    fn drop(&mut self) {
        *FAILING_DISK.lock() = None;
        WRITES_LEFT.store(usize::MAX, Ordering::SeqCst);
    }
}

/// Arms the disk under `path` to be breakable through `WRITES_LEFT`.
#[cfg(test)]
pub(super) fn arm_failing_disk(path: &Path) -> ArmedDisk {
    WRITES_LEFT.store(usize::MAX, Ordering::SeqCst);
    *FAILING_DISK.lock() = Some(path.to_path_buf());
    ArmedDisk
}

/// A disk that stops taking writes partway through, so a test can reach the
/// state redb never recovers from on its own.
#[cfg(test)]
#[derive(Debug)]
struct FailingBackend {
    inner: redb::backends::FileBackend,
}

#[cfg(test)]
impl FailingBackend {
    fn spend(&self) -> Result<(), io::Error> {
        if WRITES_LEFT
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_err()
        {
            return Err(io::Error::other("simulated disk failure"));
        }
        Ok(())
    }
}

#[cfg(test)]
impl redb::StorageBackend for FailingBackend {
    fn len(&self) -> Result<u64, io::Error> {
        self.inner.len()
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
        self.inner.read(offset, out)
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        self.inner.set_len(len)
    }

    fn sync_data(&self) -> Result<(), io::Error> {
        self.spend()?;
        self.inner.sync_data()
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        self.spend()?;
        self.inner.write(offset, data)
    }
}

/// Opens the database, through a failing disk when a test has armed this path.
pub(super) fn create_database(path: &Path) -> Result<Database, redb::DatabaseError> {
    #[cfg(test)]
    if FAILING_DISK.lock().as_deref() == Some(path) {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| redb::DatabaseError::Storage(redb::StorageError::Io(e)))?;

        return Database::builder().create_with_backend(FailingBackend {
            inner: redb::backends::FileBackend::new(file)?,
        });
    }

    Database::create(path)
}

/// Whether this failure is redb refusing to touch the disk at all.
///
/// Only that warrants trading the handle in; an ordinary write failure is
/// worth retrying as it is.
///
/// Which type the latch arrives as depends on the call that met it, and most
/// of them are redb's own rather than this crate's wrapper: `begin_write`
/// fails with a `TransactionError` and `commit` with a `CommitError`, both of
/// which reach a report through `change_context` without being wrapped. Every
/// one of them is the same dead handle.
pub(super) fn is_previous_io(report: &error_stack::Report<StorageError>) -> bool {
    use redb::StorageError::PreviousIo;

    report.frames().any(|frame| {
        frame
            .downcast_ref::<redb::StorageError>()
            .is_some_and(|e| matches!(e, PreviousIo))
            || frame
                .downcast_ref::<redb::TransactionError>()
                .is_some_and(|e| matches!(e, redb::TransactionError::Storage(PreviousIo)))
            || frame
                .downcast_ref::<redb::CommitError>()
                .is_some_and(|e| matches!(e, redb::CommitError::Storage(PreviousIo)))
            || frame
                .downcast_ref::<redb::TableError>()
                .is_some_and(|e| matches!(e, redb::TableError::Storage(PreviousIo)))
            || frame
                .downcast_ref::<error::RedbStoreError>()
                .is_some_and(|e| {
                    matches!(
                        e,
                        error::RedbStoreError::Storage(PreviousIo)
                            | error::RedbStoreError::Transaction(redb::TransactionError::Storage(
                                PreviousIo
                            ))
                            | error::RedbStoreError::Commit(redb::CommitError::Storage(PreviousIo))
                            | error::RedbStoreError::Table(redb::TableError::Storage(PreviousIo))
                    )
                })
    })
}

/// Drops the database and opens it again.
///
/// redb holds the file lock for as long as a `Database` lives, so the old
/// handle goes first and `Database::create` takes the lock back after it. That
/// is what the `None` is for, and why the caller holds `write_lock` across it:
/// a commit arriving in the gap waits for the new handle.
///
/// The buffer is untouched, so whatever was written and not yet committed goes
/// to the new handle.
pub(super) fn reopen(db: &OpenDatabase, path: &Path) -> StorageResult<()> {
    db.store(None);

    let fresh = create_database(path)
        .doing(StorageError::Open, path)
        .attach("reopening the database after an I/O failure")?;

    db.store(Some(Arc::new(fresh)));

    warn!(
        target: "amethystate",
        file = %path.display(),
        "reopened the database after an I/O failure",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreBackend;
    use crate::migration::set::MigrationSet;
    use crate::store::StoreExt;
    use crate::store::config::StoreConfig;
    use crate::stores::RedbStore;
    use amethystate_core::path::StorePath;
    use amethystate_core::test_utils::unique_path;
    use serial_test::serial;
    use std::time::Duration;

    #[test]
    #[serial]
    fn the_database_can_be_traded_for_a_fresh_one_under_a_live_store() {
        let path = unique_path("redb_reopen");
        let (store, _) = RedbStore::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();

        store
            .set(StorePath::from_segments(["before"]), &1u32)
            .unwrap();
        store.save_now().unwrap();

        let _guard = store.inner.write_lock.lock();
        reopen(&store.inner.db, &store.inner.path)
            .expect("the old handle should have released the file lock");
        drop(_guard);

        assert_eq!(
            store
                .get::<u32>(StorePath::from_segments(["before"]))
                .unwrap(),
            Some(1),
            "what was committed before the reopen is still there"
        );

        store
            .set(StorePath::from_segments(["after"]), &2u32)
            .unwrap();
        store.save_now().unwrap();
        assert_eq!(
            store
                .get::<u32>(StorePath::from_segments(["after"]))
                .unwrap(),
            Some(2),
            "and the store writes through the new handle"
        );
    }

    #[test]
    #[serial]
    fn a_disk_that_fails_for_real_is_recovered_by_trading_the_handle() {
        let path = unique_path("redb_previous_io");
        let _disk = arm_failing_disk(&path);

        let (store, _) = RedbStore::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();

        store
            .set(StorePath::from_segments(["survivor"]), &42u32)
            .unwrap();

        WRITES_LEFT.store(0, Ordering::SeqCst);

        let _ = store.inner.flush_locked(&StorePath::root());
        let latched = store.inner.flush_locked(&StorePath::root()).unwrap_err();
        assert!(
            is_previous_io(&latched),
            "redb latched but the store did not recognise it: {latched:?}"
        );

        WRITES_LEFT.store(usize::MAX, Ordering::SeqCst);

        store
            .save_now()
            .expect("the store should trade the latched handle in and land the write");

        assert_eq!(
            store
                .get::<u32>(StorePath::from_segments(["survivor"]))
                .unwrap(),
            Some(42),
            "the write buffered before the failure is on disk after the recovery"
        );
    }

    #[test]
    #[serial]
    fn a_buffered_write_survives_the_reopen() {
        let path = unique_path("redb_reopen_buffered");
        let mut config = StoreConfig::new(&path);
        config.save_debounce = Duration::from_secs(60);

        let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();
        store
            .set(StorePath::from_segments(["pending"]), &7u32)
            .unwrap();

        {
            let _guard = store.inner.write_lock.lock();
            reopen(&store.inner.db, &store.inner.path).unwrap();
        }

        store.save_now().unwrap();
        assert_eq!(
            store
                .get::<u32>(StorePath::from_segments(["pending"]))
                .unwrap(),
            Some(7),
            "the write was buffered before the reopen and belongs to the store"
        );
    }
}
