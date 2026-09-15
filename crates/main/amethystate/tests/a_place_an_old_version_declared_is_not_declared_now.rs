#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;

mod v1 {
    use super::*;

    #[amethystate(prefix = "shelf", version = 1)]
    pub struct Shelf {
        #[amestate(path = "legacy.label")]
        pub label: String,
    }
}

#[amethystate(prefix = "shelf", version = 2)]
pub struct Shelf {
    #[amestate(default = 3u32)]
    pub count: u32,
}

#[migrate]
fn forget_the_label(old: AmeData<v1::Shelf>) -> amethystate::MigrationResult<AmeData<Shelf>> {
    let _ = old.label;
    Ok(AmeData::<Shelf> { count: 3 })
}

#[test]
fn a_place_only_an_old_version_declared_is_written_as_one_name() {
    let path = TempPath::new("shelf_old_place");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let _shelf = Shelf::new_with(&store).unwrap();
        store
            .set(["shelf", "legacy", "label"], &"kept".to_string())
            .unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(120));

    insta::assert_snapshot!(std::fs::read_to_string(path.path()).unwrap());
}
