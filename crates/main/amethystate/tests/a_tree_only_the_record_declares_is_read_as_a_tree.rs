#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

#[amethystate(prefix = "look", version = 1)]
pub struct Look {
    #[amestate(path = "font.size", default = 14u32)]
    pub font_size: u32,
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

#[test]
fn a_tree_only_the_record_declares_is_read_as_a_tree() {
    let path = TempPath::new("hidden_by_record");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let _look = Look::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(120));

    let at = meta_path(path.path());
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();
    meta["schema.hidden"] = meta["schema.look"].clone();
    std::fs::write(&at, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

    let mut data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path.path()).unwrap()).unwrap();
    data["hidden"] = serde_json::json!({ "font": { "size": 20 } });
    std::fs::write(path.path(), serde_json::to_string_pretty(&data).unwrap()).unwrap();

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build()
        .unwrap();

    assert_eq!(
        store.get::<u32>(["hidden", "font", "size"]).unwrap(),
        Some(20),
        "{meta}\n{data}"
    );
}
