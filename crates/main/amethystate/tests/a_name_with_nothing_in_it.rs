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
fn a_member_named_with_nothing_is_an_entry_like_any_other() -> anyhow::Result<()> {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("empty_name_{}", backend.extension()));

        {
            let store = StoreBuilder::new(path.path()).backend(backend).build()?;
            let panel = Panel::new_with(&store)?;
            panel.widths().insert("cpu".to_string(), &120)?;
            store.save_now()?;
        }

        let pristine = std::fs::read_to_string(path.path())?;
        std::fs::write(path.path(), with_an_unnamed_member(&pristine, backend))?;

        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        let panel = Panel::new_with(&store)?;

        let held: Vec<String> = panel.widths().keys().collect();

        assert_eq!(
            held,
            ["", "cpu"],
            "on {}: the map passed over a member named with nothing",
            backend.extension()
        );

        assert_eq!(
            panel.widths().get(""),
            Some(80),
            "on {}: the entry is listed and will not answer",
            backend.extension()
        );

        panel.widths().insert("".to_string(), &90)?;
        store.save_now()?;
        drop(store);

        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        let panel = Panel::new_with(&store)?;

        assert_eq!(
            panel.widths().get(""),
            Some(90),
            "on {}: a write to the entry did not come back",
            backend.extension()
        );
    }

    Ok(())
}
