//! Tags: keys, paths, json, toml, ron
//!
//! Features: json, toml, ron
//!
//! A name the document allows and a path cannot address.
//!
//! A map is resident: opening it scans the level below its path and holds
//! every entry it finds. The depth a scan reaches never comes into it - a
//! map's entries are one level down, whatever the store's ceiling is.
//!
//! What it finds there was not necessarily written by a map. Every text
//! engine stores a map's entries as members of one object, and all three
//! grammars let a member be named with the empty string. A store path is a
//! sequence of names, and the empty name is not one, so there is no address
//! to reach that member by and no key a map could give it.
//!
//! The run below says what each engine does with one. It stays in the file
//! either way: the map never wrote it and never rewrites it.

#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::error::Error;

mod common;

fn with_an_unnamed_member(document: &str, backend: Backend) -> String {
    match backend.extension() {
        "toml" => format!("{document}\"\" = 80\n"),
        "ron" => document.replacen("\"widths\": {", "\"widths\": {\n        \"\": 80,", 1),
        _ => document.replacen("\"widths\": {", "\"widths\": {\n    \"\": 80,", 1),
    }
}

#[test]
fn a_name_no_path_can_address() -> Result<(), Box<dyn Error + Send + Sync>> {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("map_opening_scan_{}", backend.extension()));

        {
            let store = StoreBuilder::new(path.path()).backend(backend).build()?;
            let widths = store.kv().map::<String, u64>("widths")?;
            widths.insert("cpu".to_string(), &120)?;
            store.save_now()?;
        }

        let pristine = std::fs::read_to_string(path.path())?;
        std::fs::write(path.path(), with_an_unnamed_member(&pristine, backend))?;

        //@act
        //@show what the map holds
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        let widths = store.kv().map::<String, u64>("widths")?;

        let held: Vec<String> = widths.keys().collect();
        //@show-end
        //@end

        common::measured(&[
            ("engine", backend.extension()),
            ("what the map holds", &format!("{held:?}")),
            (
                "the document it opened",
                std::fs::read_to_string(path.path())?
                    .replace("\r\n", "\n")
                    .trim(),
            ),
            ("lang", backend.extension()),
        ]);

        assert_eq!(
            held,
            ["cpu"],
            "on {}: the map took a name no path can address",
            backend.extension()
        );
    }

    Ok(())
}
