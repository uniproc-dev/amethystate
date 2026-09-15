#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;

#[amethystate(prefix = "look")]
pub struct Look {
    #[amestate(path = "font.size", default = 14u32)]
    pub font_size: u32,
}

#[amethystate(prefix = "palette")]
pub struct Palette {
    #[amestate(default = {})]
    pub colours: ReactiveMap<String, u32>,
}

#[test]
fn a_deleted_field_leaves_no_empty_level_where_no_map_is_declared() {
    let path = TempPath::new("look_emptied_level");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let look = Look::new_with(&store).unwrap();
        look.font_size().set(16).unwrap();
        store.save_now().unwrap();
        store.delete(["look", "font", "size"]).unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(120));

    insta::assert_snapshot!(std::fs::read_to_string(path.path()).unwrap());
}
