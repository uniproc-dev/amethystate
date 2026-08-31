use crate::reactive::error::{WriteError, WriteResult};
use crate::reactive::watch::{Watch, Watchable};
use crate::store::Durable;
use amethystate_core::{Signal, SignalSubscription};
use error_stack::ResultExt;
use std::fmt::{self, Debug};
use std::sync::Arc;
use uuid::Uuid;

type Writer<T> = Arc<dyn Fn(T) -> WriteResult<()> + Send + Sync>;

/// How a cell commits, for the primitives that have somewhere to commit to.
/// A cell holding a value in memory has none, and needs none.
#[derive(Clone)]
pub(crate) struct CellCommit {
    pub(crate) now: Arc<dyn Fn() -> WriteResult<()> + Send + Sync>,
    pub(crate) start: Arc<dyn Fn() -> crate::store::Commit + Send + Sync>,
}

/// A view on a reactive value that lives somewhere else, with the primitive
/// behind it erased - along with its type parameters, so fields, map entries
/// and in-memory values share one type and one collection.
///
/// A cell holds nothing open: it keeps a weak reference to the primitive it
/// views, so a forgotten cell in a UI does not keep the store file open. Once
/// that primitive is dropped every write fails with
/// [`WriteError::SourceGone`]
/// and [`get`](ReactiveCell::get) returns `None`.
///
/// Absence is a value here: a cell over a map entry whose key is missing reads
/// `None` and refuses to write, because creating the entry is the map's job. A
/// cell holding a value in memory has no source to lose and never takes either
/// branch.
pub struct ReactiveCell<T> {
    cache: Signal<Option<T>>,
    writer: Writer<T>,
    alive: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    origin: Uuid,
    commit: Option<CellCommit>,
    _keepalive: Option<Arc<dyn Send + Sync>>,
    _owner: Option<Arc<dyn Send + Sync>>,
}

impl<T> Clone for ReactiveCell<T> {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            writer: Arc::clone(&self.writer),
            alive: self.alive.clone(),
            origin: self.origin,
            commit: self.commit.clone(),
            _keepalive: self._keepalive.clone(),
            _owner: self._owner.clone(),
        }
    }
}

impl<T> ReactiveCell<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// `keepalive` must hold the subscription feeding `cache` unless something
    /// captured by `writer` already does.
    pub(crate) fn from_parts(
        cache: Signal<Option<T>>,
        writer: Writer<T>,
        alive: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
        origin: Uuid,
        commit: Option<CellCommit>,
        keepalive: Option<Arc<dyn Send + Sync>>,
    ) -> Self {
        Self {
            cache,
            writer,
            alive,
            origin,
            commit,
            _keepalive: keepalive,
            _owner: None,
        }
    }

    /// Keeps `owner` alive for as long as the cell is.
    ///
    /// A cell is a view, and normally something else owns what it views. Where
    /// nothing does - a cell handed out for a bare path, whose field exists
    /// only because the cell was asked for - the cell is the owner, and this is
    /// where it says so.
    pub(crate) fn owning(mut self, owner: Arc<dyn Send + Sync>) -> Self {
        self._owner = Some(owner);
        self
    }

    /// An in-memory cell, for reactive values that are not persisted.
    pub fn new(initial: T) -> Self {
        let cache = Signal::new(Some(initial));
        let sink = cache.clone();

        Self {
            cache,
            writer: Arc::new(move |value| {
                sink.set(Some(value));
                Ok(())
            }),
            alive: None,
            origin: Uuid::new_v4(),
            commit: None,
            _keepalive: None,
            _owner: None,
        }
    }

    /// The current value, read from the cache rather than the store.
    ///
    /// `None` means there is nothing to read, and what can put it there depends
    /// on how the cell was made:
    ///
    /// - a view - [`Field::cell`](crate::Field::cell),
    ///   [`ReactiveMap::entry_cell`](crate::ReactiveMap::entry_cell) - reads
    ///   `None` once the primitive it views is dropped, and an entry view reads
    ///   `None` while its key is absent;
    /// - an owning cell - [`Field::into_cell`](crate::Field::into_cell),
    ///   [`ReactiveMap::into_entry_cell`](crate::ReactiveMap::into_entry_cell),
    ///   [`Kv::cell`](crate::store::Kv::cell) - cannot lose its source, so only
    ///   an absent entry reads `None`, and one over a field never does;
    /// - a cell holding a value in memory is always `Some`.
    ///
    /// For a cell over a persisted value the cache is kept current by the
    /// store subscription, so this sees writes made anywhere - including from
    /// another process editing the file.
    ///
    /// ```
    /// # use amethystate::ReactiveCell;
    /// let mode = ReactiveCell::new("dark".to_string());
    ///
    /// assert_eq!(mode.get(), Some("dark".to_string()));
    /// mode.set("light".to_string()).unwrap();
    /// assert_eq!(mode.get(), Some("light".to_string()));
    /// ```
    pub fn get(&self) -> Option<T> {
        match &self.alive {
            Some(alive) if !alive() => None,
            _ => self.cache.get(),
        }
    }

    /// The same writes, each returning only once the value is on disk.
    ///
    /// A cell over a value held in memory has nothing to commit to, and its
    /// writes here behave as the plain ones do.
    pub fn durable(&self) -> Durable<'_, Self> {
        Durable(self)
    }

    /// Fails if the write is refused - by an interceptor, or by the store.
    /// The cache is left to the store subscription, so a refused write never
    /// shows up in `get()`.
    pub fn set(&self, value: T) -> WriteResult<()> {
        (self.writer)(value)
    }

    /// Read-modify-write, not atomic.
    /// Reads the value, writes back what `f` returns, and yields the new
    /// value.
    ///
    /// Fails with [`WriteError::SourceGone`]
    /// when there is nothing to read - there is no value to hand `f`.
    ///
    /// ```
    /// # use amethystate::ReactiveCell;
    /// let hits = ReactiveCell::new(0u32);
    ///
    /// assert_eq!(hits.update(|n| n + 1).unwrap(), 1);
    /// assert_eq!(hits.get(), Some(1));
    /// ```
    pub fn update<F>(&self, f: F) -> WriteResult<T>
    where
        F: FnOnce(T) -> T,
    {
        let next = f(self.get().ok_or(WriteError::SourceGone)?);
        self.set(next.clone())?;
        Ok(next)
    }

    /// Same caveat as [`ReactiveCell::update`]: read-modify-write, not atomic.
    /// Reads the value, lets `f` change it in place, and writes it back.
    pub fn modify<F>(&self, f: F) -> WriteResult<()>
    where
        F: FnOnce(&mut T),
    {
        let mut value = self.get().ok_or(WriteError::SourceGone)?;
        f(&mut value);
        self.set(value)
    }

    /// Calls `callback` on every change, until the returned handle is dropped.
    ///
    /// A callback sees the change while it is still buffered, before it
    /// reaches disk - see [the module docs](crate::reactive).
    #[track_caller]
    pub fn subscribe<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&Option<T>) + Send + Sync + 'static,
    {
        self.cache.subscribe(callback)
    }

    /// Subscribes with the provenance of each change, so a subscriber can tell
    /// its own writes from everyone else's.
    ///
    /// A callback sees the change while it is still buffered, before it
    /// reaches disk - see [the module docs](crate::reactive).
    #[track_caller]
    pub fn subscribe_with_source<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&Option<T>, Option<Uuid>) + Send + Sync + 'static,
    {
        self.cache.subscribe_with_source(callback)
    }

    /// Configures a subscription: filtering, provenance, and whether it is a
    /// callback or a stream. See [`Watch`].
    pub fn subscription_with(&self) -> Watch<Self> {
        Watch::new(self.clone())
    }
}

impl<T> Watchable for ReactiveCell<T>
where
    T: Clone + Send + Sync + 'static,
{
    type Item = Option<T>;

    fn watch_id(&self) -> Uuid {
        self.origin
    }

    #[track_caller]
    fn watch_raw<F>(&self, callback: F) -> SignalSubscription
    where
        F: Fn(&Option<T>, Option<Uuid>) + Send + Sync + 'static,
    {
        self.cache.subscribe_with_source(callback)
    }
}

impl<T: Debug + Clone + Send + Sync + 'static> Debug for ReactiveCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReactiveCell")
            .field("value", &self.get())
            .finish_non_exhaustive()
    }
}

impl<T> Durable<'_, ReactiveCell<T>>
where
    T: Clone + Send + Sync + 'static,
{
    /// Writes the value.
    ///
    /// Returns only once it is on disk rather than buffered - or straight
    /// away for an in-memory cell, which has nothing to commit to.
    pub fn set(&self, value: T) -> WriteResult<()> {
        self.0.set(value)?;
        self.commit()
    }

    /// Writes the value, resolving once it is on disk.
    ///
    /// Like every future, this does nothing until awaited - the write included.
    pub async fn set_async(&self, value: T) -> WriteResult<()> {
        self.0.set(value)?;
        self.commit_async().await
    }

    /// Reads the value, writes back what `f` returns, and yields it.
    ///
    /// Returns only once it is on disk rather than buffered.
    pub fn update<F>(&self, f: F) -> WriteResult<T>
    where
        F: FnOnce(T) -> T,
    {
        let value = self.0.update(f)?;
        self.commit()?;
        Ok(value)
    }

    /// Reads the value, writes back what `f` returns, and resolves once it is
    /// on disk.
    ///
    /// Like every future, this does nothing until awaited - the write included.
    pub async fn update_async<F>(&self, f: F) -> WriteResult<T>
    where
        F: FnOnce(T) -> T,
    {
        let value = self.0.update(f)?;
        self.commit_async().await?;
        Ok(value)
    }

    /// Reads the value, lets `f` change it in place, and writes it back.
    ///
    /// Returns only once it is on disk rather than buffered.
    pub fn modify<F>(&self, f: F) -> WriteResult<()>
    where
        F: FnOnce(&mut T),
    {
        self.0.modify(f)?;
        self.commit()
    }

    /// Reads the value, lets `f` change it in place, and resolves once it is on
    /// disk.
    ///
    /// Like every future, this does nothing until awaited - the write included.
    pub async fn modify_async<F>(&self, f: F) -> WriteResult<()>
    where
        F: FnOnce(&mut T),
    {
        self.0.modify(f)?;
        self.commit_async().await
    }

    fn commit(&self) -> WriteResult<()> {
        match &self.0.commit {
            Some(commit) => (commit.now)(),
            None => Ok(()),
        }
    }

    async fn commit_async(&self) -> WriteResult<()> {
        let pending = self.0.commit.as_ref().map(|commit| (commit.start)());
        match pending {
            Some(commit) => commit
                .await
                .change_context(WriteError::Storage)
                .attach("committing a cell write"),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Store;
    use crate::store::{StateScope, StorePath};
    use crate::test_utils::unique_store;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct UiScope;
    impl StateScope for UiScope {
        const PATH: StorePath = StorePath::from_static(&["ui"], "ui");
        const KEY: &'static str = "ui";
    }

    fn stored_field(store: &Store, name: &'static str, default: u64) -> crate::Field<u64> {
        crate::store::field::<UiScope, u64>(store, [name], default, Uuid::new_v4()).expect("field")
    }

    #[test]
    fn volatile_cell_round_trips() {
        let cell = ReactiveCell::new(7u64);
        assert_eq!(cell.get(), Some(7));

        cell.set(9).expect("volatile write cannot fail");
        assert_eq!(cell.get(), Some(9));
    }

    #[test]
    fn field_cell_writes_reach_the_store() {
        let store = unique_store("write-through");
        let field = stored_field(&store, "width", 110);
        let cell = field.cell();

        cell.set(200).expect("write should reach the store");

        assert_eq!(store.get::<u64>(["ui", "width"]).unwrap(), Some(200));
        assert_eq!(
            cell.get(),
            Some(200),
            "cache must reflect the committed value"
        );
        assert_eq!(field.get(), 200, "field and cell share one cache");
    }

    #[test]
    fn a_cell_dies_with_the_field_it_views() {
        let store = unique_store("outlive");
        let cell = {
            let field = stored_field(&store, "height", 10);
            assert_eq!(field.cell().get(), Some(10));
            field.cell()
        };

        store.set(["ui", "height"], &42u64).expect("external write");

        assert_eq!(cell.get(), None, "the field is gone, so the view is empty");
        assert!(cell.set(1).is_err(), "and there is nowhere to write");
    }

    #[test]
    fn subscribers_see_writes_from_anywhere() {
        let store = unique_store("subscribe");
        let field = stored_field(&store, "depth", 1);
        let cell = field.cell();

        let seen = Arc::new(Mutex::new(Vec::new()));
        let cap = seen.clone();
        let _sub = cell.subscribe(move |v: &Option<u64>| cap.lock().unwrap().push(v.unwrap()));

        cell.set(2).unwrap();
        field.set(3).unwrap();
        store.set(["ui", "depth"], &4u64).unwrap();

        assert_eq!(*seen.lock().unwrap(), vec![2, 3, 4]);
    }

    #[test]
    fn write_through_cell_fires_once_per_write() {
        let store = unique_store("single-fire");
        let field = stored_field(&store, "fires", 0);
        let cell = field.cell();

        let fires = Arc::new(AtomicUsize::new(0));
        let cap = fires.clone();
        let _sub = cell.subscribe(move |_: &Option<u64>| {
            cap.fetch_add(1, Ordering::SeqCst);
        });

        cell.set(1).unwrap();

        assert_eq!(fires.load(Ordering::SeqCst), 1, "one write, one fire");
    }

    #[test]
    fn rejected_write_reports_and_leaves_the_cache_alone() {
        let store = unique_store("rejected");
        let field = stored_field(&store, "guarded", 5);
        let _guard = field.intercept(|_change| None);
        let cell = field.cell();

        let result = cell.set(99);

        assert!(result.is_err(), "rejected write must not report success");
        assert_eq!(
            cell.get(),
            Some(5),
            "cache must not hold a value the store refused"
        );
        assert_eq!(store.get::<u64>(["ui", "guarded"]).unwrap(), Some(5));
    }

    #[test]
    fn clones_share_one_value() {
        let cell = ReactiveCell::new(1u64);
        let twin = cell.clone();

        twin.set(2).unwrap();

        assert_eq!(cell.get(), Some(2), "clones are handles onto the same cell");
    }
}
