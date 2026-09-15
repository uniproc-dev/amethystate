//! Walking a declared struct's fields at runtime.
//!
//! [`InspectorBackend`](crate::store::InspectorBackend) asks the store what is
//! on disk, from outside the program that wrote it. This asks the other side,
//! from inside: what the declaration says, and what the struct is holding right
//! now.
//!
//! Nothing here is typed. A caller that knows the type reads the field; this is
//! for the callers that do not - a screen that draws a row per setting, a dump
//! in a bug report, a terminal that lists what an application has.

use std::fmt;

use crate::migration::fields::Role;
use amethystate_core::path::StorePath;

/// One field, as somebody looking at it sees it.
pub struct FieldView<'a> {
    /// The name in the source: what `state.font_size` is called in Rust.
    pub declared: &'static str,

    /// The name it is stored under, which is `declared` unless `path` or
    /// `rename_all` said otherwise. A dot in it is a level.
    pub stored: &'static str,

    /// Where it is, whole. What a person editing the file is looking at.
    pub at: StorePath,

    /// The type it was declared as, written the way it was written.
    pub type_name: &'static str,

    /// The doc comment on the field, empty where there was none.
    ///
    /// **A description, not a label.** It is in whatever language it was
    /// written in and fixed when the crate was compiled, so it belongs in a
    /// dump, a developer's terminal or generated documentation - not in a
    /// shipped interface. What a settings screen wants is a string from its
    /// own catalogue, and what it should look that string up by is
    /// [`at`](FieldView::at): the stored path survives a Rust rename, and
    /// changes only when the data moves, which is a migration either way.
    pub described: &'static str,

    pub role: Role,

    /// Whether it is kept in memory and never stored.
    pub volatile: bool,

    /// What it holds, rendered: the value for a leaf, how many entries for a
    /// map, and `<opaque>` where the type cannot be printed at all.
    ///
    /// Rendered rather than borrowed because a field hands its value back by
    /// value - there is no reference to lend - and this is for showing.
    pub shown: String,

    /// What the store and this field disagree about, if anything.
    pub disagreement: Option<Disagreement>,

    /// The fields under this one, for a `nested` field.
    pub inside: Option<&'a dyn Inspect>,
}

/// What a field and the store do not agree about.
///
/// The value is not what the field reports: either the store holds something
/// the field cannot read, or a declared check refused it. Either way the field
/// is reporting its default or the last thing it held, and the stored bytes are
/// where they were - so it can be looked at, and fixed.
#[derive(Debug, Clone)]
pub struct Disagreement {
    pub at: StorePath,
    pub reason: Reason,
}

/// `non_exhaustive` for the same reason [`StorageError`] is: it is a list of
/// what the store can be found disagreeing about, it grows, and it is read
/// rather than dispatched on.
///
/// [`StorageError`]: crate::store::StorageError
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Reason {
    /// What is stored will not read back as the declared type.
    WillNotRead(std::sync::Arc<str>),

    /// It read, and a declared check refused it.
    Refused(std::sync::Arc<str>),

    /// The field never wrote its default, because the store already held
    /// something at its path that seeding would have destroyed - so the two
    /// have disagreed from the first moment the field existed.
    Occupied(std::sync::Arc<str>),

    /// The store has let go of its file, so what the field holds is the last
    /// thing it heard rather than what is on disk.
    Closed,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::WillNotRead(why) => write!(f, "will not read: {why}"),
            Reason::Refused(why) => write!(f, "refused: {why}"),
            Reason::Occupied(why) => write!(f, "already held something else: {why}"),
            Reason::Closed => f.write_str("the store was closed, so this is the last it heard"),
        }
    }
}

impl fmt::Display for Disagreement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { at, reason } = self;
        write!(f, "{at} {reason}")
    }
}

impl std::error::Error for Disagreement {}

/// A struct that can list its own fields.
///
/// Implemented by `#[amethystate]` for every struct it generates.
pub trait Inspect {
    /// Where this struct's fields hang.
    fn at(&self) -> StorePath;

    /// How many fields were declared.
    fn field_count(&self) -> usize;

    /// One of them, by position. `None` past the end.
    fn field_at(&self, index: usize) -> Option<FieldView<'_>>;

    /// All of them, in the order they were declared.
    fn fields(&self) -> Fields<'_>
    where
        Self: Sized,
    {
        Fields {
            of: self,
            next: 0,
            left: self.field_count(),
        }
    }

    /// One of them by its name in the source.
    fn field(&self, declared: &str) -> Option<FieldView<'_>>
    where
        Self: Sized,
    {
        (0..self.field_count())
            .filter_map(|i| self.field_at(i))
            .find(|view| view.declared == declared)
    }
}

pub struct Fields<'a> {
    of: &'a dyn Inspect,
    next: usize,
    left: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = FieldView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let view = self.of.field_at(self.next)?;
        self.next += 1;
        self.left -= 1;
        Some(view)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for Fields<'_> {}
