//! What a store says about how its bytes were written, and what a build does
//! with a fact it does not know.
//!
//! Two layers, and only one of them moves. The set has to be read before the
//! data can be, so it cannot describe its own addressing: the separator, the
//! escape, and one record holding one value are frozen, and everything they
//! describe - the codec, the key encoding, the document layout - is free to
//! change as long as the change is named here.
//!
//! A name decides how already written bytes read back, or it does not, and the
//! namespace says which. A `codec.*`, `path.*` or `layout` fact this build has
//! no name for, or a name it knows at a value it does not write, refuses the
//! open and says which fact stopped it. Anything else is carried through
//! untouched, so an older build writing a store a newer one made does not drop
//! what it could not read.
//!
//! The application's schema is a separate mechanism with its own versions and
//! its own migrations. The two must not become one.

use crate::store::builder::Backend;
use crate::store::{StorageError, StorageResult};
use error_stack::Report;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Namespaces where an unrecognised name or value refuses the open.
const DECIDING: [&str; 3] = ["codec", "path", "layout"];

/// The bookkeeping record the set is stored under, at the root.
pub const RECORD: &str = "format";

/// What one store says about how it was written.
///
/// Its presence is the marker: a store without one predates facts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageFactSet(BTreeMap<String, String>);

impl StorageFactSet {
    /// What this build writes for `engine`.
    pub fn of(engine: Backend) -> Self {
        let mut facts = BTreeMap::new();

        let codec = codec_of(engine);
        facts.insert("codec".to_string(), codec.to_string());

        if codec == "msgpack" {
            facts.insert("codec.struct".to_string(), "map".to_string());
            facts.insert("codec.bytes".to_string(), "bin".to_string());
        }
        facts.insert("path.sep".to_string(), ".".to_string());
        facts.insert("path.escape".to_string(), "\\".to_string());
        facts.insert("path.key".to_string(), key_of(engine).to_string());
        facts.insert("layout".to_string(), layout_of(engine).to_string());

        Self(facts)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Records a fact that has no accessor of its own.
    ///
    /// Forging a set is a way to make a store unopenable, so it is reachable
    /// only from a test.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(name.into(), value.into());
        self
    }

    /// Whether a build writing `ours` may open a store recording this.
    ///
    /// `Ok` where the two agree on every deciding fact either of them states;
    /// otherwise a refusal naming the fact they part on.
    ///
    /// Recording nothing at all is the one set that decides nothing: a store
    /// written before there were facts says nothing about its bytes and is read
    /// as this build would write it. A set that states some and not others is a
    /// different thing - it was written while the deciding fact it is missing
    /// meant something else - and that is what the second pass catches.
    pub fn read_by(&self, ours: &StorageFactSet) -> StorageResult<()> {
        if self.is_empty() {
            return Ok(());
        }

        for (name, value) in &self.0 {
            if !deciding(name) {
                continue;
            }

            match ours.get(name) {
                None => return Err(unknown_fact(name, value)),
                Some(known) if known != value => return Err(unknown_value(name, value, known)),
                Some(_) => {}
            }
        }

        for (name, value) in &ours.0 {
            if deciding(name) && self.get(name).is_none() {
                return Err(fact_not_recorded(name, value));
            }
        }

        Ok(())
    }

    /// The facts `ours` has no name for, kept so a later write does not drop
    /// them.
    pub fn unknown_to(&self, ours: &StorageFactSet) -> StorageFactSet {
        StorageFactSet(
            self.0
                .iter()
                .filter(|(name, _)| ours.get(name).is_none())
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        )
    }

    /// This set, with `kept` filled in where it says nothing.
    pub fn carrying(mut self, kept: &StorageFactSet) -> StorageFactSet {
        for (name, value) in &kept.0 {
            self.0.entry(name.clone()).or_insert_with(|| value.clone());
        }
        self
    }
}

impl fmt::Display for StorageFactSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (name, value) in &self.0 {
            if !first {
                f.write_str(", ")?;
            }
            write!(f, "{name}={value}")?;
            first = false;
        }
        Ok(())
    }
}

fn deciding(name: &str) -> bool {
    let head = name.split('.').next().unwrap_or(name);
    DECIDING.contains(&head)
}

fn unknown_fact(name: &str, value: &str) -> Report<StorageError> {
    Report::new(StorageError::Open)
        .attach(format!("unknown fact: {name}={value}"))
        .attach("the store was written by a newer build, and this fact decides how its bytes read")
}

fn fact_not_recorded(name: &str, ours: &str) -> Report<StorageError> {
    Report::new(StorageError::Open)
        .attach(format!(
            "the store records no {name}, and this build writes {name}={ours}"
        ))
        .attach(
            "it was written while that fact said something else, so what its bytes hold cannot \
             be told from what this build would write",
        )
}

fn unknown_value(name: &str, theirs: &str, ours: &str) -> Report<StorageError> {
    Report::new(StorageError::Open)
        .attach(format!(
            "fact {name} is {theirs}, and this build writes {ours}"
        ))
        .attach("a known fact with an unknown value, which is the case that gets forgotten")
}

const fn codec_of(engine: Backend) -> &'static str {
    match engine {
        #[cfg(feature = "redb")]
        Backend::Redb => "msgpack",
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => "sonic-json",
        #[cfg(feature = "json")]
        Backend::Json => "json",
        #[cfg(feature = "toml")]
        Backend::Toml => "toml",
        #[cfg(feature = "ron")]
        Backend::Ron => "ron",
    }
}

/// How a path is spelled where the engine addresses by it.
///
/// `levels` is [`Key`](amethystate_core::path::Key): each level's bytes, each
/// terminated, so byte order is level order and a subtree is a prefix.
/// `joined` is the separator and escape above, which is what a document holds
/// as one name.
const fn key_of(engine: Backend) -> &'static str {
    match engine {
        #[cfg(feature = "redb")]
        Backend::Redb => "levels",
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => "levels",
        #[cfg(feature = "json")]
        Backend::Json => "joined",
        #[cfg(feature = "toml")]
        Backend::Toml => "joined",
        #[cfg(feature = "ron")]
        Backend::Ron => "joined",
    }
}

const fn layout_of(engine: Backend) -> &'static str {
    match engine {
        #[cfg(feature = "redb")]
        Backend::Redb => "flat",
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => "flat",
        #[cfg(feature = "json")]
        Backend::Json => "nested",
        #[cfg(feature = "toml")]
        Backend::Toml => "nested",
        #[cfg(feature = "ron")]
        Backend::Ron => "nested",
    }
}

/// Where an engine keeps the set.
///
/// Off [`StoreBackend`](crate::StoreBackend) on purpose: the write half can
/// make a store unopenable, and an open is the only caller. Implemented by the
/// concrete engines, which is all `settle` needs - it runs inside `open`,
/// where the type is known.
pub(crate) trait FormatRecord {
    fn format_facts(&self) -> StorageResult<Option<StorageFactSet>>;
    fn set_format_facts(&self, facts: &StorageFactSet) -> StorageResult<()>;
}

/// The same, reachable from a test.
///
/// A public trait so it can be named in a signature, and object-safe so a
/// `Store` can hand one out through its `dyn StoreBackend`. Every engine gets
/// this for free.
#[cfg(feature = "test-utils")]
pub trait TestFormatRecord {
    fn facts(&self) -> StorageResult<Option<StorageFactSet>>;
    fn set_facts(&self, facts: &StorageFactSet) -> StorageResult<()>;
}

#[cfg(feature = "test-utils")]
impl<T: FormatRecord> TestFormatRecord for T {
    fn facts(&self) -> StorageResult<Option<StorageFactSet>> {
        self.format_facts()
    }

    fn set_facts(&self, facts: &StorageFactSet) -> StorageResult<()> {
        self.set_format_facts(facts)
    }
}

/// Reads what a store records, refuses if this build cannot honour it, and
/// writes the set back with anything unrecognised carried through.
///
/// Called while opening, before a migration runs against bytes this build may
/// not understand.
pub(crate) fn settle<B: FormatRecord + ?Sized>(store: &B, engine: Backend) -> StorageResult<()> {
    let ours = StorageFactSet::of(engine);

    match store.format_facts()? {
        None => store.set_format_facts(&ours),
        Some(theirs) => {
            theirs.read_by(&ours)?;

            let kept = theirs.unknown_to(&ours);
            let settled = ours.carrying(&kept);
            if settled == theirs {
                return Ok(());
            }
            store.set_format_facts(&settled)
        }
    }
}

/// The same, for an engine known only by the codec it runs.
#[cfg(feature = "text")]
pub(crate) fn settle_for_codec<B: FormatRecord + ?Sized>(
    store: &B,
    codec: crate::store::CodecFormat,
) -> StorageResult<()> {
    settle(store, Backend::writing(codec))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(feature = "redb", feature = "json"))]
    use crate::store::builder::Backend;
    use crate::store::builder::default_backend;

    fn ours() -> StorageFactSet {
        StorageFactSet::of(default_backend())
    }

    #[test]
    fn a_store_written_by_this_build_opens() {
        let theirs = ours();
        assert!(theirs.read_by(&ours()).is_ok());
    }

    #[test]
    fn a_fact_outside_the_deciding_namespaces_is_ignored() {
        let theirs = ours().with("wrote.by", "something newer");
        assert!(theirs.read_by(&ours()).is_ok());
    }

    #[test]
    fn an_unknown_deciding_fact_refuses_and_names_it() {
        let theirs = ours().with("codec.frames", "chunked");

        let refused = theirs.read_by(&ours()).unwrap_err();
        let printed = format!("{refused:?}");

        assert!(
            printed.contains("codec.frames=chunked"),
            "the refusal names the fact: {printed}"
        );
    }

    #[test]
    fn a_known_fact_with_an_unknown_value_refuses_too() {
        let theirs = ours().with("path.sep", "/");

        let refused = theirs.read_by(&ours()).unwrap_err();
        let printed = format!("{refused:?}");

        assert!(
            printed.contains("path.sep"),
            "the refusal names the fact: {printed}"
        );
    }

    #[test]
    fn a_store_with_no_facts_at_all_predates_them() {
        assert!(StorageFactSet::default().is_empty());
        assert!(StorageFactSet::default().read_by(&ours()).is_ok());
    }

    #[test]
    fn a_set_that_states_some_and_not_the_rest_is_refused_by_the_missing_name() {
        let theirs = StorageFactSet::default().with("codec", ours().get("codec").expect("a codec"));

        let refused = theirs
            .read_by(&ours())
            .expect_err("a set stating one deciding fact is not a set stating none");
        let printed = format!("{refused:?}");

        assert!(
            printed.contains("the store records no "),
            "a fact the store leaves out is a different refusal from one it states: {printed}"
        );
    }

    #[cfg(all(feature = "redb", feature = "json"))]
    #[test]
    fn a_flat_store_written_before_the_key_encoding_is_refused_by_name() {
        let mut older = BTreeMap::new();
        for (name, value) in &StorageFactSet::of(Backend::Redb).0 {
            if name != "path.key" {
                older.insert(name.clone(), value.clone());
            }
        }

        let refused = StorageFactSet(older)
            .read_by(&StorageFactSet::of(Backend::Redb))
            .expect_err("its keys are the joined spelling and this build writes levels");
        let printed = format!("{refused:?}");

        assert!(
            printed.contains("path.key=levels"),
            "the refusal says what this build writes: {printed}"
        );
    }

    #[cfg(all(feature = "redb", feature = "json"))]
    #[test]
    fn one_engines_store_is_refused_by_another() {
        let written = StorageFactSet::of(Backend::Redb);

        let refused = written
            .read_by(&StorageFactSet::of(Backend::Json))
            .expect_err("msgpack bytes are not json");
        let printed = format!("{refused:?}");

        assert!(
            printed.contains("codec"),
            "the refusal names the codec: {printed}"
        );
    }

    #[test]
    fn what_a_build_does_not_understand_survives_its_writes() {
        let theirs = ours().with("wrote.by", "redb 9.0").with("codec", "msgpack");

        let kept = theirs.unknown_to(&ours());
        let written = ours().carrying(&kept);

        assert_eq!(written.get("wrote.by"), Some("redb 9.0"));
        assert_eq!(
            written.get("codec"),
            ours().get("codec"),
            "and what it does understand is its own"
        );
    }
}
