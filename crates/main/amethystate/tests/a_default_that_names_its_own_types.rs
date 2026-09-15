use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::BTreeMap;

#[amethystate(prefix = "turbofish")]
pub struct Held {
    #[amestate(default = BTreeMap::<String, u32>::new())]
    pub sizes: BTreeMap<String, u32>,

    #[amestate(default = BTreeMap::<String, u32>::from([("cpu".to_string(), 110u32)]))]
    pub seeded: BTreeMap<String, u32>,

    #[amestate(default = 800u32)]
    pub width: u32,
}

#[backends(all)]
fn a_default_spelling_its_types_is_one_entry_and_not_two(backend: Backend) {
    let at = TempPath::new("turbofish_default");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();

    let held = Held::new_with(&store).unwrap();

    assert!(held.sizes().get().is_empty());
    assert_eq!(held.seeded().get().get("cpu"), Some(&110));
    assert_eq!(held.width().get(), 800);
}
