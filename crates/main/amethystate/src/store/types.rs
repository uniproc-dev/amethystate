use amethystate_core::Source;
use amethystate_core::path::StorePath;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// One subscription, among every subscription any store in this process has
/// handed out.
///
/// Drawn from a single count for the whole process rather than one per store,
/// so an id one store handed out names nothing in another: handed to the wrong
/// store's [`StoreBackend::unsubscribe`](crate::StoreBackend::unsubscribe), it
/// removes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubscriptionId(u64);

impl SubscriptionId {
    /// An id no store in this process has handed out before - what an engine
    /// gives the subscription it is registering.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// What a subscriber does with a change, and what it says about it.
///
/// The answer travels back to whoever made the change: a subscriber runs on
/// that thread, so a value nobody could read back is something the writer can
/// be told rather than something only the log knows.
pub type StoreCallback =
    Arc<dyn Fn(&StoreEvent) -> crate::store::StorageResult<()> + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    Set,
    Delete,

    /// Everything under a prefix went away as one operation. The event path is
    /// the prefix.
    DeletePrefix,
}

#[derive(Debug, Clone)]
pub struct StoreEvent {
    pub path: StorePath,
    pub op: StoreOp,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
    pub source: Source,

    /// Where this change comes in the order the store settled them.
    ///
    /// Minted where the change was settled - under the lock that decided which
    /// of two racing writes is the later one - so whoever applies these to a
    /// value of their own can end on the same one the store did, whatever
    /// order the callbacks happen to run in.
    pub at: u64,
}

impl StoreEvent {
    /// Whether this change was made outside this process: off the disk, or by
    /// another page of the same site.
    pub fn is_external_edit(&self) -> bool {
        matches!(self.source, Source::Disk | Source::AnotherPage)
    }
}

/// What a subscriber asked to hear about. `Prefix` matches by level, not by
/// characters: `ui` hears `ui.theme` and not `uix.width`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionKind {
    Any,
    ExactPath(StorePath),
    Prefix(StorePath),
}

/// How an engine renders a value.
///
/// `non_exhaustive` for the reason [`Backend`](crate::store::builder::Backend)
/// is: the variants are behind feature flags, so the set depends on how this
/// crate was built, and a new engine adds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecFormat {
    #[cfg(any(feature = "redb", feature = "memory"))]
    MessagePack,

    #[cfg(any(feature = "json", feature = "localstorage"))]
    Json,

    #[cfg(feature = "sqlite")]
    SonicJson,

    #[cfg(feature = "toml")]
    Toml,

    #[cfg(feature = "ron")]
    Ron,
}

#[derive(Clone)]
pub struct SubscriptionEntry {
    pub id: SubscriptionId,
    pub kind: SubscriptionKind,
    pub callback: StoreCallback,
}

/// A store subscription that ends when this is dropped.
///
/// Both fields are private, and the drop is what they are for: `id` is the key
/// the store removes the entry by, and `store` is which store it is removed
/// from. An id nobody registered would remove nothing and leak the callback, a
/// colliding one would remove a stranger's subscription, and another store's
/// handle would unsubscribe from the wrong place.
///
/// It lives here, with the store, because a map holds one as well as a field
/// does, and the primitives factory is what builds them.
pub struct StoreSubscription {
    store: crate::Store,
    id: SubscriptionId,
}

impl StoreSubscription {
    pub(crate) fn new(store: crate::Store, id: SubscriptionId) -> Self {
        Self { store, id }
    }

    /// Which subscription this is, for a caller that wants to say so.
    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    /// The store it is on, for a durable write that has to flush it.
    ///
    /// Lent rather than handed over: what a holder must not be able to do is
    /// *replace* it, since that is what the drop unsubscribes from.
    pub(crate) fn store(&self) -> &crate::Store {
        &self.store
    }
}

impl Drop for StoreSubscription {
    fn drop(&mut self) {
        self.store.unsubscribe(self.id);
    }
}
