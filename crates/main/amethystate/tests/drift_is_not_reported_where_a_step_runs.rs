#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::migration::{ComponentOutcome, MigrationError};
use amethystate::store::OpenStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

#[amethystate(prefix = "panel", version = 2)]
pub struct Panel {
    #[amestate(default = 0u32)]
    pub height: u32,
}

fn recorded(meta: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(meta).unwrap()).unwrap()
}

#[test]
fn a_prefix_whose_step_failed_is_reported_once_and_not_as_drift() {
    let at = TempPath::new("failed_step_not_drift");
    let meta = at.path().with_extension("meta");

    {
        let store = StoreBuilder::new(at.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let panel = Panel::new_with(&store).unwrap();
        panel.height().set(480).unwrap();
        store.close().unwrap();
    }

    let mut older = recorded(&meta);
    older["meta.panel"] = serde_json::json!({ "versions": { "": 1 } });
    older["schema.panel"][0]["version"] = serde_json::json!(1);
    older["schema.panel"][0]["fields"][0]["name"] = serde_json::json!("width");
    std::fs::write(&meta, serde_json::to_string_pretty(&older).unwrap()).unwrap();

    let refused = StoreBuilder::new(at.path())
        .backend(Backend::Json)
        .migrations(|m| {
            m.for_node::<Panel>().step(2, "turns the data down", |_| {
                Err(MigrationError::Custom("this data is not ours".into()).into())
            });
        })
        .migrate();

    let Err(OpenStore::Migrating {
        report: Some(report),
        ..
    }) = refused
    else {
        panic!("a step that failed let the store open");
    };

    let about_panel: Vec<&ComponentOutcome> = report
        .components
        .iter()
        .filter(|one| one.prefixes.iter().any(|at| at.to_string() == "panel"))
        .map(|one| &one.outcome)
        .collect();

    assert_eq!(about_panel.len(), 1, "{report:?}");
    assert!(
        matches!(about_panel[0], ComponentOutcome::Failed { .. }),
        "{report:?}"
    );
}
