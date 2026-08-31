#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::amethystate;
use amethystate::store::StoreBackend;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;

mod common;
use common::text_backend;

macro_rules! doc {
    (json = $j:expr, toml = $t:expr, ron = $r:expr $(,)?) => {{
        #[cfg(feature = "json")]
        {
            $j
        }
        #[cfg(all(feature = "toml", not(feature = "json")))]
        {
            $t
        }
        #[cfg(all(feature = "ron", not(feature = "json"), not(feature = "toml")))]
        {
            $r
        }
    }};
}

fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(120));
}

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(default = 1280u16)]
    pub width: u16,
}

fn seeded(suffix: &str, contents: &str) -> TempPath {
    let path = TempPath::new(suffix);
    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        cfg.width().set(800).unwrap();
        drop(cfg);
        store.save_now().unwrap();
    }
    settle();
    std::fs::write(path.path(), contents).unwrap();
    path
}

/// A declared `u16` whose stored value became a string. Reading it must not
/// pass for a number, and building the struct must say so rather than come up
/// with something.
#[test]
fn a_field_whose_value_became_a_string_is_reported() {
    let path = seeded(
        "tamper_type_string",
        doc! {
            json = "{ \"cfg\": { \"width\": \"wide\" } }\n",
            toml = "[cfg]\nwidth = \"wide\"\n",
            ron  = "{\"cfg\": {\"width\": \"wide\"}}",
        },
    );

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let built = Cfg::new_with(&store);

    assert!(
        built.is_err(),
        "a string read back as a u16 field without complaint: {:?}",
        built.map(|c| c.width().get())
    );
}

/// The same for a value the format calls a number but the field cannot hold.
#[test]
fn a_field_whose_value_became_a_float_is_reported() {
    let path = seeded(
        "tamper_type_float",
        doc! {
            json = "{ \"cfg\": { \"width\": 1.5 } }\n",
            toml = "[cfg]\nwidth = 1.5\n",
            ron  = "{\"cfg\": {\"width\": 1.5}}",
        },
    );

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let built = Cfg::new_with(&store);
    assert!(
        built.is_err(),
        "1.5 read back as a u16 field: {:?}",
        built.map(|c| c.width().get())
    );
}

/// A value that overflows the declared width. Truncating it silently would be
/// worse than refusing.
#[test]
fn a_field_whose_value_overflows_is_reported() {
    let path = seeded(
        "tamper_type_overflow",
        doc! {
            json = "{ \"cfg\": { \"width\": 99999999 } }\n",
            toml = "[cfg]\nwidth = 99999999\n",
            ron  = "{\"cfg\": {\"width\": 99999999}}",
        },
    );

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let built = Cfg::new_with(&store);
    assert!(
        built.is_err(),
        "99999999 read back as a u16 field: {:?}",
        built.map(|c| c.width().get())
    );
}

/// A leaf that became a branch. The field is no longer a number; the store must
/// not answer as though it were, and the subtree must be visible as one.
#[test]
#[cfg_attr(
    feature = "toml",
    ignore = "known: `with_bytes_de` cuts at the first `=`, so toml reads a section back as one of its own keys - see TODO.md"
)]
fn a_leaf_that_became_a_branch_is_reported() {
    let path = seeded(
        "tamper_leaf_to_branch",
        doc! {
            json = "{ \"cfg\": { \"width\": { \"px\": 800 } } }\n",
            toml = "[cfg.width]\npx = 800\n",
            ron  = "{\"cfg\": {\"width\": {\"px\": 800}}}",
        },
    );

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    assert_eq!(
        store.get::<u16>(["cfg", "width", "px"]).unwrap(),
        Some(800),
        "the child under the branch is unreachable"
    );

    let read = Cfg::new_with(&store).map(|c| c.width().get());
    assert!(read.is_err(), "a branch read back as a u16 field: {read:?}");
}

/// A branch that became a leaf. Deleting the prefix has to clear it, and a map
/// bound to that path has to come up empty rather than take the scalar for an
/// entry.
#[test]
fn a_branch_that_became_a_leaf_can_still_be_cleared() {
    let path = TempPath::new("tamper_branch_to_leaf");
    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["items", "a"], &1u32).unwrap();
        store.set(["items", "b"], &2u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    std::fs::write(
        path.path(),
        doc! {
            json = "{ \"items\": 7 }\n",
            toml = "items = 7\n",
            ron  = "{\"items\": 7}",
        },
    )
    .unwrap();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let entries = StoreBackend::scan_keys(&store, &StorePath::segment("items")).unwrap();
    assert_eq!(
        entries,
        vec![StorePath::segment("items")],
        "the scalar is listed as the node at the prefix, as on every engine"
    );

    store.delete_prefix(["items"]).unwrap();
    assert_eq!(
        store.get::<u32>(["items"]).unwrap(),
        None,
        "the scalar survived a delete of its own prefix"
    );
}

/// A section the schema declares whose value on disk is a scalar. Building the
/// struct writes the field's default under it, and the walker replaces whatever
/// was in the way with an empty map first - so the value is gone with no error
/// and nothing to migrate from.
#[test]
fn a_section_that_holds_a_scalar_is_not_thrown_away_on_startup() {
    let path = seeded(
        "tamper_section_scalar",
        doc! {
            json = "{ \"cfg\": 5 }\n",
            toml = "cfg = 5\n",
            ron  = "{\"cfg\": 5}",
        },
    );

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        let _cfg = Cfg::new_with(&store).unwrap();
        drop(_cfg);
        store.save_now().unwrap();
    }
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains('5'),
        "the value that stood where the section goes was dropped: {written}"
    );
}

/// The same for a list, which is a value a hand-written config is full of.
#[test]
fn a_section_that_holds_a_list_is_not_thrown_away_on_startup() {
    let path = seeded(
        "tamper_section_list",
        doc! {
            json = "{ \"cfg\": [11, 22, 33] }\n",
            toml = "cfg = [11, 22, 33]\n",
            ron  = "{\"cfg\": [11, 22, 33]}",
        },
    );

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        let _cfg = Cfg::new_with(&store).unwrap();
        drop(_cfg);
        store.save_now().unwrap();
    }
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("22"),
        "the list that stood where the section goes was dropped: {written}"
    );
}

/// Keys the schema never declared must survive the rewrite the store does on
/// every save - at the top level and nested beside a declared one.
#[test]
fn undeclared_keys_survive_a_round_trip() {
    let path = seeded(
        "tamper_extra_keys",
        doc! {
            json = "{ \"cfg\": { \"width\": 800, \"note\": \"keep me\" }, \"other\": { \"deep\": { \"x\": 5 } } }\n",
            toml = "[cfg]\nwidth = 800\nnote = \"keep me\"\n\n[other.deep]\nx = 5\n",
            ron  = "{\"cfg\": {\"width\": 800, \"note\": \"keep me\"}, \"other\": {\"deep\": {\"x\": 5}}}",
        },
    );

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        cfg.width().set(1024).unwrap();
        drop(cfg);
        store.save_now().unwrap();
    }
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("keep me"),
        "an undeclared sibling key was dropped: {written}"
    );
    assert!(
        written.contains('5'),
        "an undeclared nested subtree was dropped: {written}"
    );
}
