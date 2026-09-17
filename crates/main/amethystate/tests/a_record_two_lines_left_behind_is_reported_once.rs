#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

#[amethystate(prefix = "rosette", id = "badge", version = 1)]
pub struct Badge {
    #[amestate(default = 1u32)]
    pub count: u32,
}

#[amethystate(prefix = "rosette", id = "ribbon", version = 1)]
pub struct Ribbon {
    #[amestate(default = 2u32)]
    pub length: u32,
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

#[test]
fn a_record_no_line_at_its_prefix_declares_is_reported_once() {
    let path = TempPath::new("rosette_left_behind");

    {
        let (store, _) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .migrate()
            .unwrap();
        let _badge = Badge::new_with(&store).unwrap();
        let _ribbon = Ribbon::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(120));

    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    held["schema.rosette"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "version": 1,
            "struct_name": "Rosette",
            "fields": [
                { "name": "colour", "type_name": "String",
                  "shape": { "role": "field", "optional": false } },
            ],
        }));

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .migrate()
        .unwrap();
    drop(store);

    let nagging: Vec<_> = report
        .components
        .iter()
        .flat_map(|component| component.nagging.iter())
        .collect();

    assert_eq!(nagging.len(), 1, "{report:?}");
}
