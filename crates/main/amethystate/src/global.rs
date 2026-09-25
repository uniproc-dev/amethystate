use crate::store::{OpenStore, StorageResult};
use crate::{MigrationReport, Store, StoreBackend, StoreBuilder};
use std::fmt;
use std::path::Path;
use std::sync::{PoisonError, RwLock};

/// The process-wide store.
///
/// Rust does not drop statics, so nothing here is closed on the way out: an
/// ordinary store writes what it has buffered from its `Drop`, and this one
/// never reaches one. [`shutdown`] is that step, said out loud.
///
/// A closed store stays here until another is put in its place, so a read
/// after the close answers `Closed` rather than finding nothing.
static GLOBAL_STORE: RwLock<Option<Store>> = RwLock::new(None);

fn held() -> Option<Store> {
    GLOBAL_STORE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Closes the process-wide store when it goes out of scope.
///
/// A local is dropped and a static is not, which is the whole of why this
/// exists: held in `main`, it closes the store at the end of `main` - while the
/// logger, the threads and the allocator are all still up - rather than leaving
/// the last writes to a static that is never dropped.
///
/// [`GlobalStoreGuard::close`] is the door and the `Drop` is the net. Closing
/// by hand hands back what the last flush did, so a caller who can offer a
/// retry or save elsewhere gets the chance; letting it drop closes just the
/// same and logs a failure that nobody is left to act on.
///
/// Dropping it early closes the store early, and every read and write after
/// that answers [`StorageError::Closed`](crate::store::StorageError::Closed).
/// Binding it in `main` is what puts that at the end rather than in the middle.
///
/// It closes the store it was handed out with and no other: once that store is
/// closed another can be put in place, and a guard dropped after that leaves
/// the new one open.
#[must_use = "dropped here, the global store is closed here - bind it in `main` \
              (`let _ame = ...`) so the last writes are flushed on the way out"]
#[derive(Debug)]
pub struct GlobalStoreGuard {
    store: Option<Store>,
}

impl GlobalStoreGuard {
    /// Closes the process-wide store and hands back what the closing flush did.
    ///
    /// ```no_run
    /// fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let ame = amethystate::init_global("./app/settings");
    ///
    ///     // ...
    ///
    ///     ame.close()?;
    ///     Ok(())
    /// }
    /// ```
    #[allow(clippy::needless_doctest_main)]
    pub fn close(mut self) -> StorageResult<()> {
        match self.store.take() {
            Some(store) => Ok(store.close()?),
            None => Ok(()),
        }
    }

    /// [`GlobalStoreGuard::close`], awaited rather than blocked on.
    pub fn close_async(mut self) -> crate::store::Commit {
        match self.store.take() {
            Some(store) => StoreBackend::close_async(&store),
            None => crate::store::Commit::ready(Ok(())),
        }
    }
}

impl Drop for GlobalStoreGuard {
    fn drop(&mut self) {
        let Some(store) = self.store.take() else {
            return;
        };
        if let Err(report) = store.close() {
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

    /// Opens the process-wide store as
    /// [`StoreBuilder::build`](crate::StoreBuilder::build) does, and hands back
    /// the guard that closes it.
    ///
    /// Panics when the store will not open or an open one is in place already;
    /// [`StoreBuilder::build_global`](crate::StoreBuilder::build_global)
    /// answers both as an error.
    fn init_global(self) -> GlobalStoreGuard {
        self.into_store_builder()
            .build_global()
            .unwrap_or_else(|why| {
                panic!("amethystate: the global store was not put in place: {why:?}")
            })
    }
}

/// The same terminals as the builder's, each putting the store it opens in
/// place as the process-wide one.
///
/// Nothing is opened when an open store is in place already. A closed one is
/// replaced.
impl StoreBuilder {
    /// [`StoreBuilder::build`], put in place.
    pub fn build_global(self) -> Result<GlobalStoreGuard, InitGlobal> {
        refuse_a_second()?;
        let store = self.build().map_err(InitGlobal::Open)?;
        installed(store)
    }

    /// [`StoreBuilder::migrate`], put in place.
    pub fn migrate_global(self) -> Result<(GlobalStoreGuard, MigrationReport), InitGlobal> {
        refuse_a_second()?;
        let (store, report) = self.migrate().map_err(InitGlobal::Open)?;
        Ok((installed(store)?, report))
    }
}

impl crate::store::builder::WithSteps {
    /// [`WithSteps::migrate`](crate::store::builder::WithSteps::migrate), put in
    /// place.
    pub fn migrate_global(self) -> Result<(GlobalStoreGuard, MigrationReport), InitGlobal> {
        self.into_builder().migrate_global()
    }
}

#[cfg(feature = "memory")]
impl crate::store::builder::OrInMemory<crate::store::builder::WithSteps> {
    /// [`OrInMemory::migrate`](crate::store::builder::OrInMemory::migrate), put
    /// in place.
    pub fn migrate_global(
        self,
    ) -> Result<(GlobalStoreGuard, MigrationReport, crate::store::Persistence), InitGlobal> {
        refuse_a_second()?;
        let (store, report, persistence) = self.migrate();
        Ok((installed(store)?, report, persistence))
    }
}

#[cfg(feature = "memory")]
impl crate::store::builder::OrInMemory {
    /// [`OrInMemory::build`](crate::store::builder::OrInMemory::build), put in
    /// place. The one refusal left is a store already there.
    pub fn build_global(self) -> Result<(GlobalStoreGuard, crate::store::Persistence), InitGlobal> {
        refuse_a_second()?;
        let (store, persistence) = self.build();
        Ok((installed(store)?, persistence))
    }

    /// [`OrInMemory::migrate`](crate::store::builder::OrInMemory::migrate), put
    /// in place.
    pub fn migrate_global(
        self,
    ) -> Result<(GlobalStoreGuard, MigrationReport, crate::store::Persistence), InitGlobal> {
        refuse_a_second()?;
        let (store, report, persistence) = self.migrate();
        Ok((installed(store)?, report, persistence))
    }
}

fn refuse_a_second() -> Result<(), InitGlobal> {
    match held() {
        Some(store) if !store.is_closed() => Err(InitGlobal::AlreadyInstalled),
        _ => Ok(()),
    }
}

fn installed(store: Store) -> Result<GlobalStoreGuard, InitGlobal> {
    install_global(store).map_err(|_| InitGlobal::AlreadyInstalled)
}

/// Why the process-wide store was not put in place.
pub enum InitGlobal {
    /// The store would not open. Nothing was installed.
    Open(OpenStore),

    /// An open store is in place already, and no second one was opened.
    AlreadyInstalled,
}

impl fmt::Display for InitGlobal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(_) => f.write_str("the global store would not open"),
            Self::AlreadyInstalled => f.write_str("a global store is in place already"),
        }
    }
}

impl fmt::Debug for InitGlobal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(why) => write!(f, "{self}: {why:?}"),
            Self::AlreadyInstalled => fmt::Display::fmt(self, f),
        }
    }
}

impl std::error::Error for InitGlobal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open(why) => Some(why),
            Self::AlreadyInstalled => None,
        }
    }
}

/// A store handed to [`install_global`] while another open one was in place,
/// handed back untouched.
pub struct AlreadyInstalled {
    pub store: Store,
}

impl fmt::Display for AlreadyInstalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a global store is in place already")
    }
}

impl fmt::Debug for AlreadyInstalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for AlreadyInstalled {}

/// Puts a store that is already open in place as the process-wide one.
///
/// For a caller that opens the store itself - to answer a failed open as its
/// own error, or to hand the same store to something else first.
///
/// A closed store in place is replaced; an open one refuses the new store.
pub fn install_global(store: Store) -> Result<GlobalStoreGuard, AlreadyInstalled> {
    let mut slot = GLOBAL_STORE.write().unwrap_or_else(PoisonError::into_inner);
    if slot.as_ref().is_some_and(|held| !held.is_closed()) {
        return Err(AlreadyInstalled { store });
    }
    *slot = Some(store.clone());

    Ok(GlobalStoreGuard { store: Some(store) })
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

/// Opens the process-wide store.
///
/// ```no_run
/// fn main() {
///     let _ame = amethystate::init_global("./app/settings");
///
///     // ...
/// }
/// ```
#[allow(clippy::needless_doctest_main)]
pub fn init_global<T: IntoGlobalStore>(source: T) -> GlobalStoreGuard {
    source.init_global()
}

/// The process-wide store.
///
/// Panics when nothing installed one. Every generated accessor on a struct
/// opened globally goes through here, so the panic is what a field read before
/// `init_global` looks like - and it says that rather than leaving a caller
/// with an unwrap on a `None`.
pub fn global_store() -> Store {
    held().expect(
        "amethystate: the global store is not initialized.\n\
         Call `init_global`, `build_global` or `migrate_global` during startup, \
         and keep the guard it returns alive for as long as the store is used.",
    )
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
/// belongs where nothing follows it - or where another store is put in place
/// next. Calling it twice is fine.
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
    match held() {
        Some(store) => Ok(store.close()?),
        None => Ok(()),
    }
}

/// [`shutdown`], awaited rather than blocked on: the closing write runs
/// without holding the thread that awaits it.
pub fn shutdown_async() -> crate::store::Commit {
    match held() {
        Some(store) => StoreBackend::close_async(&store),
        None => crate::store::Commit::ready(Ok(())),
    }
}
