use amethystate_core::path::StorePath;
use std::sync::Arc;
use uuid::Uuid;

pub type SubscriptionId = u64;
pub type StoreCallback = Arc<dyn Fn(&StoreEvent) + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    Set,
    Delete,

    /// Everything under a prefix went away as one operation. The event path is
    /// the prefix.
    DeletePrefix,
}

/// The writer of a change that arrived from the file rather than from a handle
/// in this process.
///
/// Its bytes are `amethystate_disk` in ASCII, so it reads as itself in a log
/// and cannot collide with a generated id: the version and variant bits are
/// not a v4's.
pub const EXTERNAL_EDIT: Uuid = Uuid::from_u128(0x616d_6574_6879_7374_6174_655f_6469_736b);

#[derive(Debug, Clone)]
pub struct StoreEvent {
    pub path: StorePath,
    pub op: StoreOp,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
    pub source: Option<Uuid>,
}

impl StoreEvent {
    /// Whether this change came off the disk.
    pub fn is_external_edit(&self) -> bool {
        self.source == Some(EXTERNAL_EDIT)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecFormat {
    #[cfg(test)]
    Default,

    #[cfg(feature = "redb")]
    MessagePack,

    #[cfg(feature = "json")]
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
