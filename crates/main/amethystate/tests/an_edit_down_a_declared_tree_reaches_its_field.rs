#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::time::Duration;

#[amethystate(prefix = "look", version = 1)]
pub struct Look {
    #[amestate(path = "font.size", default = 14u32)]
    pub font_size: u32,
}

fn settle() {
    std::thread::sleep(Duration::from_millis(400));
}

#[test]
fn an_edit_down_a_declared_tree_reaches_its_field() {
    let path = TempPath::new("look_edited_by_hand");
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .disk(|d| {
            d.debounce(Duration::from_millis(20))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    let look = Look::new_with(&store).unwrap();
    look.font_size().set(16).unwrap();
    store.save_now().unwrap();
    settle();

    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path.path()).unwrap()).unwrap();
    held["look"]["font"]["size"] = serde_json::json!(20);
    std::fs::write(path.path(), serde_json::to_string_pretty(&held).unwrap()).unwrap();
    settle();

    assert_eq!(look.font_size().get(), 20, "{held}");
}
