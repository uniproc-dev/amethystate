use crate::reactive::cell::CellCommit;
use crate::reactive::cell::ReactiveCell;
use crate::reactive::error::WriteValue;
use crate::store::StoreBackend;
use crate::store::facts::Facts;
use crate::{ReactiveMap, ReactiveMapKey, ReactiveMapValue};
use amethystate_core::{MapChange, Signal};
use error_stack::Report;
use std::sync::Arc;

impl<K, V> ReactiveMap<K, V>
where
    K: ReactiveMapKey,
    V: ReactiveMapValue,
{
    /// An entry cell that keeps the map alive, for handing one entry somewhere
    /// and letting go of the map.
    ///
    /// The cell owns the map, so it stays readable and writable for as long as
    /// it is held. [`ReactiveMap::entry_cell`] is the view onto a map something
    /// else keeps.
    ///
    /// The map it takes is the one it keeps. Nothing is taken from a struct by
    /// doing this: a generated accessor hands out an `Arc::clone` of the map,
    /// so the cell becomes one more owner of it.
    pub fn into_entry_cell(self, key: K) -> ReactiveCell<V> {
        let cell = self.entry_cell(key);
        cell.owning(Arc::new(self))
    }

    /// A live cell over one map entry.
    ///
    /// The cell is a view and keeps only a weak reference to the map, so once
    /// the map is dropped every write fails and
    /// [`get`](ReactiveCell::get) reads `None`. Where the cell is the only
    /// handle that survives, use [`ReactiveMap::into_entry_cell`].
    ///
    /// An absent key reads `None`, and writing to one fails - creating the
    /// entry is [`ReactiveMap::insert`]'s job.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// let widths = store.kv().map::<String, u64>("columns").unwrap();
    /// widths.insert("cpu".to_string(), &100).unwrap();
    /// let cpu = widths.entry_cell("cpu".to_string());
    ///
    /// assert_eq!(cpu.get(), Some(100));
    ///
    /// // Writing through the cell writes the map entry.
    /// cpu.set(120).unwrap();
    /// assert_eq!(widths.get("cpu"), Some(120));
    ///
    /// // And a write to the map reaches the cell.
    /// widths.update("cpu", &200).unwrap();
    /// assert_eq!(cpu.get(), Some(200));
    ///
    /// // Removing the key empties the cell, and it refuses to put it back.
    /// widths.remove("cpu").unwrap();
    /// assert_eq!(cpu.get(), None);
    /// assert!(cpu.set(80).is_err());
    /// ```
    pub fn entry_cell(&self, key: K) -> ReactiveCell<V> {
        let cache = Signal::new(self.get(&key));

        let sink = cache.clone();
        let read = self.subscribe_key(key.clone(), move |change| {
            let (next, source) = match change {
                MapChange::Insert { value, source, .. }
                | MapChange::Update {
                    new_value: value,
                    source,
                    ..
                } => (Some(value.clone()), *source),
                MapChange::Remove { source, .. } | MapChange::Clear { source } => (None, *source),
            };
            sink.set_forwarded(next, source);
        });

        let origin = self.inner.instance_id;
        let weak = Arc::downgrade(&self.inner);
        let write_key = key;

        let flush = weak.clone();
        let start = weak.clone();
        let commit = CellCommit {
            now: Arc::new(move || {
                let inner = flush.upgrade().ok_or(WriteValue::SourceGone)?;
                inner
                    .store
                    .flush_prefix(&inner.path)
                    .map_err(Report::from)
                    .attach_key(&inner.path)
                    .map_err(|why| WriteValue::from_store(&inner.path, why))
            }),
            start: Arc::new(move || match start.upgrade() {
                Some(inner) => inner.store.flush_async(),
                None => crate::store::Commit::gone(),
            }),
        };

        let alive = weak.clone();
        ReactiveCell::from_parts(
            cache,
            Arc::new(move |value: V| {
                let inner = weak.upgrade().ok_or(WriteValue::SourceGone)?;
                let map = ReactiveMap::<K, V> { inner };
                map.update(&write_key, &value)
            }),
            Some(Arc::new(move || alive.strong_count() > 0)),
            origin,
            Some(commit),
            Some(Arc::new(read)),
        )
    }
}
