use crate::store::Check;
use crate::store::StorageError;
use crate::store::traits::StoredAs;
use error_stack::Report;

/// What building a struct does about a stored value it will not accept: one
/// that does not decode into the field's type, and one a declared check
/// refuses.
///
/// The value got there somehow - a file edited by hand, a migration that left
/// something behind, a codec that took what it cannot read back - and the two
/// answers serve different applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnUnreadable {
    /// Construction fails, naming the path. Nothing half-built is handed out.
    #[default]
    Refuse,

    /// The field takes its declared default and construction carries on.
    ///
    /// The stored value is left where it is, so a person can still fix the file
    /// by hand, and the field says the store does not agree with what it is
    /// reporting: [`Field::try_get`](crate::Field::try_get) answers `Err` from
    /// the moment it is built until a change decodes.
    UseDefault,
}

/// Whether the store had the bytes and they would not read back, as against
/// not having them at all.
///
/// The one place the difference is decided, because two callers need it and
/// have to agree: the policy, to know whether a default may stand in, and the
/// constructor, to say which of the two it is failing with.
pub fn will_not_read(why: &Report<StorageError>) -> bool {
    why.frames()
        .filter_map(|frame| frame.downcast_ref::<StorageError>())
        .any(|context| *context == StorageError::Codec)
}

impl OnUnreadable {
    /// Whether this failure is one [`OnUnreadable::UseDefault`] stands in for.
    ///
    /// A decode failure, and that alone. A store that cannot be read at all
    /// propagates: there is no default to stand in for a file that is not
    /// there.
    pub(crate) fn covers(&self, why: &Report<StorageError>) -> bool {
        matches!(self, OnUnreadable::UseDefault) && will_not_read(why)
    }
}

/// What a map does with an entry it cannot read: one whose key does not parse
/// as the map's key type, or whose value does not decode into its value type.
///
/// Its own answer rather than [`OnUnreadable`]'s, because the two are asked
/// about different things. A field holds one value, and the way to carry on
/// without it is to stand its declared default in its place. A map holds data
/// somebody wrote, and it has no default for one entry - what it declares is
/// the map to seed a store holding none. So carrying on here means the entry
/// left where it is and out of what the map reports, which is a third answer
/// and is spelled as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnreadableEntries {
    /// Building the map fails, naming the entry.
    #[default]
    Refuse,

    /// The entry is left out and the rest of the map is built.
    ///
    /// It stays on disk, so a person can still fix the file, and it is named
    /// in a line at `error`. A key that is not an entry of this map at all is
    /// a question about places rather than about reading, and is refused under
    /// either answer.
    Skip,
}

/// What a field falls back to when neither it nor the struct holding it said.
///
/// The last word in a chain that starts at the field: a field's own rule wins
/// over its struct's, and a struct's over this. So an application can say what
/// it wants of everything that did not care, without touching a declaration -
/// and nothing that did care is quietly overruled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fallbacks {
    pub on_unreadable: OnUnreadable,
    pub on_delete: OnDelete,
    pub unreadable_entries: UnreadableEntries,
}

impl Fallbacks {
    /// What a field does about a value it cannot read, where neither it nor
    /// the struct holding it said. Without this, [`OnUnreadable::Refuse`].
    pub fn on_unreadable(mut self, rule: OnUnreadable) -> Self {
        self.on_unreadable = rule;
        self
    }

    /// What a field reports once the key behind it is gone, where neither it
    /// nor the struct holding it said. Without this, [`OnDelete::Keep`].
    pub fn on_delete(mut self, rule: OnDelete) -> Self {
        self.on_delete = rule;
        self
    }

    /// What a map does with an entry it cannot read, where neither it nor the
    /// struct holding it said. Without this, [`UnreadableEntries::Refuse`].
    pub fn unreadable_entries(mut self, rule: UnreadableEntries) -> Self {
        self.unreadable_entries = rule;
        self
    }
}

/// What a field does when its key is deleted under it.
///
/// A deletion is somebody else's doing - another handle, a migration, a hand
/// edited file - and the two answers disagree about what a field is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnDelete {
    /// The field goes on reporting the last value it held.
    ///
    /// A deleted key is not a value, and the declared default is a
    /// compile-time guess - the least likely thing the person was looking at.
    /// Keeping is also what stops a removal and an undecodable value from
    /// being the same observable, which everything else here works to keep
    /// apart.
    #[default]
    Keep,

    /// The field reports its declared default again, as if it had never been
    /// written.
    UseDefault,
}

/// What a field does about the store disagreeing with it: a value it cannot
/// read, a key removed under it, and a value its declared check refuses.
///
/// One value carries all of it, so "what did this field decide" has a single
/// answer to hold and a single place to add to.
pub struct ReadRules<TValue> {
    pub(crate) on_unreadable: OnUnreadable,
    pub(crate) on_delete: OnDelete,
    pub(crate) check: Option<Check<TValue>>,
    pub(crate) stored_as: StoredAs<TValue>,
}

impl<TValue> Default for ReadRules<TValue> {
    fn default() -> Self {
        Self {
            on_unreadable: OnUnreadable::default(),
            on_delete: OnDelete::default(),
            check: None,
            stored_as: StoredAs::default(),
        }
    }
}

impl<TValue> ReadRules<TValue> {
    /// The rules a field takes when nothing says otherwise: refuse a value
    /// that will not decode, keep what it holds when the key is removed, and
    /// judge nothing.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_unreadable(mut self, policy: OnUnreadable) -> Self {
        self.on_unreadable = policy;
        self
    }

    /// How this field is stored, when that is not how its type would be.
    ///
    /// What `#[amestate(with = ..)]` and its halves put here.
    pub fn stored_as(mut self, how: StoredAs<TValue>) -> Self {
        self.stored_as = how;
        self
    }

    pub fn on_delete(mut self, policy: OnDelete) -> Self {
        self.on_delete = policy;
        self
    }

    /// The rule every value coming in from the store has to pass.
    pub fn check(mut self, check: Check<TValue>) -> Self {
        self.check = Some(check);
        self
    }
}

/// What a declared struct wrote about reading, so the struct holding it can be
/// checked against it while it compiles.
///
/// `None` is a struct that said nothing and takes whatever it is built under.
/// The macro implements this for everything it generates; nothing else should.
pub trait DeclaredPolicy {
    const ON_UNREADABLE: Option<OnUnreadable>;
    const ON_DELETE: Option<OnDelete>;
    const UNREADABLE_ENTRIES: Option<UnreadableEntries>;
}
