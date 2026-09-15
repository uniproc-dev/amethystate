#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;

mod v1 {
    use super::*;

    #[amethystate(prefix = "panel", version = 1)]
    pub struct Panel {
        pub width: u32,
        pub legacy: u32,
    }
}

#[amethystate(prefix = "panel", version = 2)]
pub struct Panel {
    pub width: u32,
    pub height: u32,
}

#[migrate]
fn migrate_panel_v1_to_v2(old: AmeData<v1::Panel>) -> amethystate::MigrationResult<AmeData<Panel>> {
    Ok(AmeData::<Panel> {
        width: old.width,
        height: old.legacy,
    })
}

fn recorded(meta: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(meta).unwrap()).unwrap()
}

fn version_of_panel(meta: &std::path::Path) -> Option<serde_json::Value> {
    recorded(meta)["schema.panel"]
        .as_array()?
        .iter()
        .find(|one| one["struct_name"] == serde_json::json!("Panel"))
        .map(|one| one["version"].clone())
}

#[test]
fn a_prefix_a_compiled_step_targets_is_not_judged_for_drift_through_build() {
    let at = TempPath::new("compiled_step_waits");
    let meta = at.path().with_extension("meta");

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        store.set(["panel", "width"], &640u32).unwrap();
        store.set(["panel", "legacy"], &480u32).unwrap();
        store.close().unwrap();
    }

    let mut older = recorded(&meta);
    older["schema.panel"] = serde_json::json!([{
        "version": 1,
        "struct_name": "Panel",
        "fields": [
            { "name": "width", "type_name": "u32", "shape": { "role": "field", "optional": false } },
            { "name": "legacy", "type_name": "u32", "shape": { "role": "field", "optional": false } }
        ]
    }]);
    std::fs::write(&meta, serde_json::to_string_pretty(&older).unwrap()).unwrap();

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        store.close().unwrap();
    }

    assert_eq!(
        version_of_panel(&meta),
        Some(serde_json::json!(2)),
        "{:#}",
        recorded(&meta)
    );
}
