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
//! What it finds there was not necessarily written by a map. A declared map
//! keeps a level of its own, every text engine stores its entries as members
//! of that one object, and all three grammars let a member be named with the
//! empty string. A store path is a sequence of names, and the empty name is
//! not one, so there is no address to reach that member by and no key a map
//! could give it.
//!
//! The run below says what each engine does with one. It keeps its place in
//! the file: the map never wrote it and never rewrites it, and a save that
//! rewrites the document whole leaves it where it was.

#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::ReactiveMap;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;

mod common;

#[amethystate(prefix = "panel")]
pub struct Panel {
    pub widths: ReactiveMap<String, u64>,
}

fn with_an_unnamed_member(document: &str, backend: Backend) -> String {
    let opened = match backend.extension() {
        "toml" => "[panel.widths]\n",
        "ron" => "\"widths\": {",
        _ => "\"widths\": {",
    };

    assert!(
        document.contains(opened),
        "on {}: the map has no level to hold a nameless member, so nothing is \
         being measured. The document was:\n{document}",
        backend.extension()
    );

    match backend.extension() {
        "toml" => document.replacen(opened, &format!("{opened}\"\" = 80\n"), 1),
        "ron" => document.replacen(opened, &format!("{opened}\n            \"\": 80,"), 1),
        _ => document.replacen(opened, &format!("{opened}\n      \"\": 80,"), 1),
    }
}

#[test]
fn a_name_no_path_can_address() -> anyhow::Result<()> {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("map_opening_scan_{}", backend.extension()));

        {
            let store = StoreBuilder::new(path.path()).backend(backend).build()?;
            let panel = Panel::new_with(&store)?;
            panel.widths().insert("cpu".to_string(), &120)?;
            store.save_now()?;
        }

        let pristine = std::fs::read_to_string(path.path())?;
        std::fs::write(path.path(), with_an_unnamed_member(&pristine, backend))?;

        //@act
        //@show what the map holds
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        let panel = Panel::new_with(&store)?;

        let held: Vec<String> = panel.widths().keys().collect();
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

        panel.widths().insert("gpu".to_string(), &90)?;
        store.save_now()?;

        let after = std::fs::read_to_string(path.path())?;
        assert!(
            after.contains("\"\""),
            "on {}: a save rewrote the document and dropped the nameless member:\n{after}",
            backend.extension()
        );
    }

    Ok(())
}
