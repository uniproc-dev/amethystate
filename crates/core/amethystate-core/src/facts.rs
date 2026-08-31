//! What a failure was about, as types rather than as sentences.
//!
//! Attached to a report with the [`Facts`] extension trait, and read back out
//! of one with [`all`].

use crate::path::StorePath;
use error_stack::{Frame, Report, ResultExt};
use std::fmt;
use std::path::{Path, PathBuf};

/// Every fact of type `T` the report carries, innermost first.
///
/// `Report::request_ref` would be the natural way and is nightly-only, so this
/// walks the frames instead.
pub fn all<T: Send + Sync + 'static, C>(report: &Report<C>) -> impl Iterator<Item = &T> {
    report.frames().filter_map(Frame::downcast_ref::<T>)
}

macro_rules! fact {
    ($(#[$meta:meta])* $name:ident($ty:ty) => $label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(pub $ty);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($label, ": {}"), self.0)
            }
        }
    };
}

/// The store's own file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreFile(pub PathBuf);

impl fmt::Display for StoreFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "store: {}", self.0.display())
    }
}

/// The sidecar the text engines keep their bookkeeping in, which is a
/// different file from the one holding the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaFile(pub PathBuf);

impl fmt::Display for MetaFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "meta file: {}", self.0.display())
    }
}

fact!(
    /// The key an operation was about.
    Key(StorePath) => "key"
);
fact!(
    /// The subtree an operation was about.
    Prefix(StorePath) => "prefix"
);
fact!(
    /// A key as it sits on disk, before anything has read it as a path - which
    /// is the only state it is in when the reason for the failure is that it
    /// cannot be read as one.
    RawKey(String) => "stored key"
);
fact!(
    /// The engine table it was in.
    Table(String) => "table"
);
fact!(
    /// Why a declared check would not have the value the store holds, in the
    /// words the check itself gave.
    ///
    /// A value carrying this decoded perfectly well: what it failed is the
    /// application's own rule about what the value may be.
    Refused(String) => "refused"
);
fact!(
    /// One level below a prefix, as it was scanned rather than as a caller
    /// spelled it.
    Entry(String) => "entry"
);
fact!(
    /// The prefix a migration step was running over, which is what the keys in
    /// the same report are relative to.
    Migrating(String) => "migrating"
);
fact!(
    /// A node in the bookkeeping sidecar, named by its kind and the prefix it
    /// describes rather than by any key a caller wrote.
    MetaNode(String) => "meta node"
);
fact!(
    /// How large the value was.
    ValueBytes(usize) => "value bytes"
);
fact!(
    /// How far a scan had got when it failed.
    Read(usize) => "read so far"
);
fact!(
    /// How much the write buffer was holding.
    Buffered(usize) => "buffered entries"
);

/// Attaches a fact to a failing result, lazily: nothing is built on the path
/// that succeeds.
pub trait Facts: ResultExt + Sized {
    fn attach_store_file(self, file: &Path) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_meta_file(self, file: &Path) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_key(self, key: &StorePath) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_prefix(self, prefix: &StorePath) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_raw_key(self, key: &str) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_table(self, table: &str) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_meta_node(self, node: &str) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_entry(self, name: &str) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_migrating(self, prefix: &str) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_value_bytes(self, len: usize) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_read_so_far(self, count: usize) -> Result<Self::Ok, Report<Self::Context>>;
    fn attach_buffered(self, count: usize) -> Result<Self::Ok, Report<Self::Context>>;
}

impl<R: ResultExt> Facts for R {
    fn attach_store_file(self, file: &Path) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| StoreFile(file.to_path_buf()))
    }

    fn attach_meta_file(self, file: &Path) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| MetaFile(file.to_path_buf()))
    }

    fn attach_key(self, key: &StorePath) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Key(key.clone()))
    }

    fn attach_prefix(self, prefix: &StorePath) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Prefix(prefix.clone()))
    }

    fn attach_raw_key(self, key: &str) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| RawKey(key.to_owned()))
    }

    fn attach_table(self, table: &str) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Table(table.to_owned()))
    }

    fn attach_meta_node(self, node: &str) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| MetaNode(node.to_owned()))
    }

    fn attach_entry(self, name: &str) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Entry(name.to_owned()))
    }

    fn attach_migrating(self, prefix: &str) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Migrating(prefix.to_owned()))
    }

    fn attach_value_bytes(self, len: usize) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| ValueBytes(len))
    }

    fn attach_read_so_far(self, count: usize) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Read(count))
    }

    fn attach_buffered(self, count: usize) -> Result<R::Ok, Report<R::Context>> {
        self.attach_with(|| Buffered(count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::error::WriteError;

    fn failing() -> Result<(), Report<WriteError>> {
        Err(Report::new(WriteError::Storage))
    }

    #[test]
    fn a_fact_reads_back_by_its_type() {
        let path = StorePath::from_segments(["ui", "theme"]);
        let report = failing().attach_key(&path).unwrap_err();

        let found: Vec<&Key> = all::<Key, _>(&report).collect();
        assert_eq!(found, vec![&Key(path)]);
    }

    #[test]
    fn two_facts_of_different_types_do_not_collide() {
        let key = StorePath::from_segments(["ui", "theme"]);
        let prefix = StorePath::segment("ui");
        let report = failing().attach_key(&key).attach_prefix(&prefix).unwrap_err();

        assert_eq!(all::<Key, _>(&report).count(), 1);
        assert_eq!(all::<Prefix, _>(&report).count(), 1);
        assert_eq!(
            all::<Prefix, _>(&report).next(),
            Some(&Prefix(prefix)),
            "a prefix must not be read back as a key"
        );
    }

    #[test]
    fn the_same_fact_attached_twice_is_visible_as_twice() {
        let key = StorePath::from_segments(["ui", "theme"]);
        let report = failing().attach_key(&key).attach_key(&key).unwrap_err();

        assert_eq!(
            all::<Key, _>(&report).count(),
            2,
            "a duplicate has to be countable before anything can collapse it"
        );
    }

    #[test]
    fn a_fact_prints_with_its_label() {
        let report = failing()
            .attach_key(&StorePath::from_segments(["ui", "theme"]))
            .unwrap_err();

        assert!(
            format!("{report:?}").contains("key: ui.theme"),
            "the label lives on the type, so it must reach the printed report"
        );
    }
}
