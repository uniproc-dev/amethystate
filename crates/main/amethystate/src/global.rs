use crate::store::StorageResult;
use crate::{MigrationReport, Store, StoreBuilder};
use std::path::Path;
use std::sync::OnceLock;

/// The process-wide store.
///
/// Rust does not drop statics, so nothing here is closed on the way out: an
/// ordinary store writes what it has buffered from its `Drop`, and this one
/// never reaches one. [`shutdown`] is that step, said out loud.
static GLOBAL_STORE: OnceLock<Store> = OnceLock::new();

/// Closes the process-wide store when it goes out of scope.
///
/// A local is dropped and a static is not, which is the whole of why this
/// exists: held in `main`, it runs [`shutdown`] at the end of `main` - while
/// the logger, the threads and the allocator are all still up - rather than
/// leaving the last writes to a static that is never dropped.
///
/// Dropping it early closes the store early, and every read and write after
/// that answers [`StorageError::Closed`](crate::store::StorageError::Closed).
/// Binding it in `main` is what puts that at the end rather than in the middle.
#[must_use = "dropped here, the global store is closed here - bind it in `main` \
              (`let _ame = ...`) so the last writes are flushed on the way out"]
pub struct GlobalStoreGuard {
    _private: (),
}

impl Drop for GlobalStoreGuard {
    fn drop(&mut self) {
        if let Err(report) = shutdown() {
            tracing::error!(
                target: "amethystate",
                error = ?report,
                "the global store's closing flush failed: what it still held is not on disk",
            );
        }
    }
}

pub trait IntoGlobalStore: Sized {
    fn into_store_builder(self) -> StoreBuilder;

    /// Opens the process-wide store, once, and hands back the guard that
    /// closes it.
    ///
    /// Runs the migrations declared by hand and no others.
    /// [`IntoGlobalStore::init_global_with_migration`] is the one that also picks
    /// up every `#[migrate]` step in the binary, and says what the pass did -
    /// the same split as
    /// [`build`](crate::StoreBuilder::build) and
    /// [`build_with_migration`](crate::StoreBuilder::build_with_migration).
    #[must_use = "dropped here, the global store is closed here - bind it in `main` \
                  (`let _ame = ...`) so the last writes are flushed on the way out"]
    fn init_global(self) -> GlobalStoreGuard {
        let store = self.into_store_builder().build().unwrap_or_else(|err| {
            panic!(
                "amethystate: Failed to build global StoreBackend.\n\
                     Ensure the database path is writable and not locked by another process.\n\
                     Details: {err}"
            );
        });

        install(store)
    }

    /// [`IntoGlobalStore::init_global`], with every `#[migrate]` step in the
    /// binary collected as well, and what the pass did.
    fn init_global_with_migration(self) -> (MigrationReport, GlobalStoreGuard) {
        let (store, report) = self
            .into_store_builder()
            .build_with_migration()
            .unwrap_or_else(|err| {
                panic!(
                    "amethystate: Failed to build global StoreBackend.\n\
                     Ensure the database path is writable and not locked by another process.\n\
                     Details: {err}"
                );
            });

        (report, install(store))
    }
}

fn install(store: Store) -> GlobalStoreGuard {
    GLOBAL_STORE.set(store).unwrap_or_else(|_| {
        panic!(
            "amethystate: Global store is already initialized.\n\
                 Ensure `init_global` is called exactly once during application startup."
        );
    });

    GlobalStoreGuard { _private: () }
}

impl IntoGlobalStore for StoreBuilder {
    fn into_store_builder(self) -> StoreBuilder {
        self
    }
}

impl IntoGlobalStore for &str {
    fn into_store_builder(self) -> StoreBuilder {
        StoreBuilder::new(self)
    }
}

impl IntoGlobalStore for &Path {
    fn into_store_builder(self) -> StoreBuilder {
        StoreBuilder::new(self)
    }
}

/// Opens the process-wide store, once.
///
/// ```no_run
/// fn main() {
///     let _ame = amethystate::init_global("./app/settings");
///
///     // ...
/// }
/// ```
#[must_use = "dropped here, the global store is closed here - bind it in `main` \
              (`let _ame = ...`) so the last writes are flushed on the way out"]
#[allow(clippy::needless_doctest_main)]
pub fn init_global<T: IntoGlobalStore>(source: T) -> GlobalStoreGuard {
    source.init_global()
}

/// [`init_global`], with every `#[migrate]` step in the binary collected as
/// well, and what the pass did.
pub fn init_global_with_migration<T: IntoGlobalStore>(
    source: T,
) -> (MigrationReport, GlobalStoreGuard) {
    source.init_global_with_migration()
}

pub fn global_store() -> Store {
    GLOBAL_STORE.get().unwrap().clone()
}

/// Closes the process-wide store: writes what it still holds, stops its
/// background thread and lets go of the file.
///
/// [`GlobalStoreGuard`] calls this and logs a failure; call it directly when
/// the failure is worth acting on - offering to retry, saving elsewhere, or
/// not exiting yet - since only a caller can do any of that.
///
/// Nothing else can stand in. A static is never dropped, so the close every
/// other store gets from `Drop` never runs here, and the thread would outlive
/// `main` for the same reason. Left out, everything written inside the last
/// debounce interval is lost on a clean return.
///
/// Afterwards the store answers every read and write with
/// [`StorageError::Closed`](crate::store::StorageError::Closed), so this
/// belongs where nothing follows it. Calling it twice is fine.
///
/// Does nothing, successfully, when no global store was ever initialised.
///
/// ```no_run
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let _ame = amethystate::init_global("./app/settings");
///
///     // ...
///
///     amethystate::shutdown()?;
///     Ok(())
/// }
/// ```
pub fn shutdown() -> StorageResult<()> {
    match GLOBAL_STORE.get() {
        Some(store) => store.close(),
        None => Ok(()),
    }
}
