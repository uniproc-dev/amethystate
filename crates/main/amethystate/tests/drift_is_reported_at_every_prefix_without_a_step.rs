#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

#[amethystate(prefix = "alpha")]
pub struct Alpha {
    pub width: u32,
    pub height: u32,
}

#[amethystate(prefix = "beta")]
pub struct Beta {
    pub width: u32,
    pub height: u32,
}

fn recorded(meta: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(meta).unwrap()).unwrap()
}

fn an_older_shape(struct_name: &str) -> serde_json::Value {
    serde_json::json!([{
        "version": 1,
        "struct_name": struct_name,
        "fields": [
            { "name": "width", "type_name": "u32", "shape": { "role": "field", "optional": false } },
            { "name": "legacy", "type_name": "u32", "shape": { "role": "field", "optional": false } }
        ]
    }])
}

#[test]
fn every_prefix_without_a_step_is_judged_for_drift() {
    let at = TempPath::new("drift_every_prefix");
    let meta = at.path().with_extension("meta");

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        for prefix in ["alpha", "beta"] {
            store.set([prefix, "width"], &640u32).unwrap();
            store.set([prefix, "legacy"], &480u32).unwrap();
        }
        store.close().unwrap();
    }

    let mut older = recorded(&meta);
    older["schema.alpha"] = an_older_shape("Alpha");
    older["schema.beta"] = an_older_shape("Beta");
    std::fs::write(&meta, serde_json::to_string_pretty(&older).unwrap()).unwrap();

    let (_store, report) = StoreBuilder::new(at.path())
        .backend(Backend::Json)
        .migrate()
        .unwrap();

    let mut drifted: Vec<String> = report
        .components
        .iter()
        .flat_map(|one| one.nagging.iter())
        .map(|record| record.prefix.to_string())
        .collect();
    drifted.sort();

    assert_eq!(drifted, vec!["alpha", "beta"], "{report:?}");
}
