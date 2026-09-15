#![cfg(feature = "json")]

use amethystate::MigrationError;
use amethystate::amethystate;
use amethystate::migration::ComponentOutcome;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

#[amethystate(prefix = "recorded", version = 1)]
pub struct Recorded {
    #[amestate(default = 1u32)]
    pub count: u32,
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

#[test]
fn a_line_recorded_twice_is_not_read_as_its_first_record() {
    let path = TempPath::new("recorded_twice");

    {
        let (store, _) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build_with_migration()
            .unwrap();
        let _recorded = Recorded::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(120));

    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    held["schema.recorded"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "version": 1,
            "struct_name": "Recorded",
            "fields": [
                { "name": "total", "type_name": "u32",
                  "shape": { "role": "field", "optional": false } },
            ],
        }));

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap();
    drop(store);

    let said: Vec<String> = report
        .components
        .iter()
        .filter_map(|component| match &component.outcome {
            ComponentOutcome::Failed { error } => error
                .downcast_ref::<MigrationError>()
                .map(ToString::to_string),
            _ => None,
        })
        .collect();

    assert_eq!(
        said,
        ["[recorded] is recorded twice, and which record is the shape on disk cannot be told"],
        "{report:?}"
    );
}
