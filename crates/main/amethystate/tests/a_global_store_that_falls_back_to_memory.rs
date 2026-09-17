#![cfg(all(feature = "memory", feature = "json"))]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{OpenStore, StoreLayout};
use amethystate::{InitGlobal, StoreBackend, global_store};
use amethystate_core::test_utils::TempPath;

#[test]
fn a_global_store_that_will_not_open_is_put_in_place_in_memory_and_says_so() {
    let broken = TempPath::new("global_fallback_broken");
    std::fs::write(broken.path(), "{ this never finished").unwrap();

    let (guard, persistence) = StoreBuilder::new(broken.path())
        .backend(Backend::Json)
        .or_in_memory()
        .build_global()
        .expect("nothing was in place, so a store should have been");

    assert!(
        matches!(persistence.because(), Some(OpenStore::WouldNotOpen { .. })),
        "{persistence:?}"
    );
    assert_eq!(global_store().files_layout(), Some(StoreLayout::InMemory));
    global_store().kv().set("port", &8080u16).unwrap();

    let late = TempPath::new("global_fallback_late");
    let again = StoreBuilder::new(late.path())
        .backend(Backend::Json)
        .or_in_memory()
        .migrate_global();
    assert!(
        matches!(again, Err(InitGlobal::AlreadyInstalled)),
        "a second init was not told the slot is taken: {:?}",
        again.map(|(_, _, persistence)| persistence)
    );
    assert!(
        !late.path().exists(),
        "a store was opened only to be turned away"
    );

    drop(guard);
    assert_eq!(
        std::fs::read_to_string(broken.path()).unwrap(),
        "{ this never finished"
    );
}
