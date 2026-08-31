//! Tags: option, toml, json, ron, choosing an engine
//!
//! Features: json, toml, ron
//!
//! What each engine writes for a value that is there and holds nothing.
//!
//! `{}` and `null` are different documents, and an absent key is a third thing
//! again. Which of the three an engine produces for `None` decides what a
//! schema can say about it: a property that may be null is not a property that
//! may be missing, and the two are written differently.
//!
//! The engines answer in two ways, and the run below says which is which.
//!
//! Note what the two that do have a null still do differently: ron spells the
//! `Option` in the document, so a hand-edited file has to say `Some(...)`,
//! while json writes the value bare and lets `null` carry the absence.
//!
//! TOML has no null, so a key holding nothing is a key that is not written.
//! That is how every TOML config expresses an optional setting, and it means
//! the format answers `set(None)` and `delete` with the same document - a
//! distinction the other engines keep and this one cannot.

use amethystate::amethystate;
use amethystate::store::StoreBackend;
use amethystate::store::builder::{Backend, StoreBuilder, default_backend};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::error::Error;

mod common;

#[amethystate(prefix = "maybe")]
pub struct Held {
    #[amestate(default = None)]
    pub note: Option<String>,
}

fn note_path() -> StorePath {
    StorePath::from_segments(["maybe", "note"])
}

fn holds_nothing(backend: Backend) -> Option<Option<String>> {
    if backend.extension() == "toml" {
        None
    } else {
        Some(None)
    }
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn what_a_document_holds_for_nothing() -> Result<(), Box<dyn Error + Send + Sync>> {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("absent_or_null_{}", backend.extension()));
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;

        //@act
        let held = Held::new_with(&store)?;

        //@show a value
        held.note().durable().set(Some("here".to_string()))?;
        let with_value = std::fs::read_to_string(path.path())?;
        //@show-end

        //@show the same key set to nothing
        held.note().durable().set(None)?;
        let with_nothing = std::fs::read_to_string(path.path())?;
        //@show-end

        let read: Option<Option<String>> = store.get(note_path())?;
        //@end

        common::measured(&[
            ("engine", backend.extension()),
            ("get::<Option<String>>", &format!("{read:?}")),
            ("a value", with_value.replace("\r\n", "\n").trim()),
            (
                "the same key set to nothing",
                with_nothing.replace("\r\n", "\n").trim(),
            ),
            ("lang", backend.extension()),
        ]);

        assert_ne!(
            with_value, with_nothing,
            "on {}: a value and its absence must not write the same document",
            backend.extension()
        );
        assert_eq!(
            read,
            holds_nothing(backend),
            "on {}: what the engine reads back for a value set to nothing",
            backend.extension()
        );
    }

    Ok(())
}

#[test]
fn nothing_and_gone_are_different() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("absent_or_gone");
    let store = StoreBuilder::new(path.path()).build()?;

    let held = Held::new_with(&store)?;
    held.note().set(None)?;

    assert_eq!(
        store.get::<Option<String>>(note_path())?,
        holds_nothing(default_backend()),
        "set to nothing"
    );

    StoreBackend::delete(&store, &note_path())?;

    assert_eq!(
        store.get::<Option<String>>(note_path())?,
        None,
        "deleted: there is no key"
    );

    Ok(())
}
