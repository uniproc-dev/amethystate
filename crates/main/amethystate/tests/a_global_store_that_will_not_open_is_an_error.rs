#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AlreadyInstalled, InitGlobal, install_global, try_init_global};
use amethystate_core::test_utils::TempPath;

#[test]
fn a_global_store_that_will_not_open_is_an_error_and_the_slot_stays_free() {
    let broken = TempPath::new("global_broken");
    std::fs::write(broken.path(), "{ this never finished").unwrap();

    let refused = try_init_global(StoreBuilder::new(broken.path()).backend(Backend::Json));
    assert!(
        matches!(refused, Err(InitGlobal::Open(_))),
        "a file that will not parse was not an open error: {refused:?}"
    );

    let good = TempPath::new("global_good");
    let store = StoreBuilder::new(good.path())
        .backend(Backend::Json)
        .build()
        .unwrap();
    let guard = install_global(store).expect("the failed open left the slot taken");

    let other = TempPath::new("global_other");
    let second = StoreBuilder::new(other.path())
        .backend(Backend::Json)
        .build()
        .unwrap();
    let Err(AlreadyInstalled { store: handed_back }) = install_global(second) else {
        panic!("a second store was installed over the first");
    };
    drop(handed_back);

    let late = TempPath::new("global_late");
    let again = try_init_global(StoreBuilder::new(late.path()).backend(Backend::Json));
    assert!(
        matches!(again, Err(InitGlobal::AlreadyInstalled)),
        "a second init was not told the slot is taken: {again:?}"
    );
    assert!(
        !late.path().exists(),
        "a store was opened only to be turned away"
    );

    drop(guard);
}
