use uuid::Uuid;

/// Who made a change the store is telling people about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A handle in this process, by the id its writes carry.
    Handle(Uuid),

    /// A write through the store's own API, with no handle behind it.
    Store,

    /// The file was edited outside this process, and the watcher read it.
    Disk,
}

impl Source {
    /// The handle that made it, for a subscriber telling its own writes from
    /// everyone else's.
    pub fn handle(self) -> Option<Uuid> {
        match self {
            Source::Handle(id) => Some(id),
            Source::Store | Source::Disk => None,
        }
    }
}

impl From<Option<Uuid>> for Source {
    fn from(handle: Option<Uuid>) -> Self {
        handle.map_or(Source::Store, Source::Handle)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change<T> {
    pub source: Option<Uuid>,
    pub old_value: T,
    pub new_value: T,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MapChange<K, V> {
    Insert {
        key: K,
        value: V,
        source: Option<Uuid>,
    },
    Update {
        key: K,
        /// What was there before. Bytes that will not read as `V` fall back to
        /// what the map last held, so a key it knows always carries one.
        old_value: Option<V>,
        new_value: V,
        source: Option<Uuid>,
    },
    Remove {
        key: K,
        old_value: Option<V>,
        source: Option<Uuid>,
    },
    Clear {
        source: Option<Uuid>,
    },
}

impl<K, V> MapChange<K, V> {
    pub fn key(&self) -> Option<&K> {
        match self {
            MapChange::Insert { key, .. } => Some(key),
            MapChange::Update { key, .. } => Some(key),
            MapChange::Remove { key, .. } => Some(key),
            MapChange::Clear { .. } => None,
        }
    }

    pub fn source(&self) -> Option<Uuid> {
        match self {
            MapChange::Insert { source, .. } => *source,
            MapChange::Update { source, .. } => *source,
            MapChange::Remove { source, .. } => *source,
            MapChange::Clear { source } => *source,
        }
    }
}
