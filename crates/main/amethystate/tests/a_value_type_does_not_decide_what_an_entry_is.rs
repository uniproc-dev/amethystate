use amethystate::Store;
use amethystate::store::LoadMap;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::HashMap;

fn named(refused: &LoadMap) -> (String, &str, &str) {
    let LoadMap::KeyIsNotAnEntry {
        under,
        stored,
        said,
    } = refused
    else {
        panic!("refused, but not as a key that is not an entry: {refused:?}")
    };

    (under.to_string(), stored.as_ref(), said.as_ref())
}

fn over_two_leaves(store: &Store) {
    store.set(["widths", "left", "px"], &800u32).unwrap();
    store.set(["widths", "left", "pct"], &50u32).unwrap();
    store.save_now().unwrap();
}

#[backends(all)]
fn a_value_type_that_would_take_the_level_below_does_not_make_it_an_entry(backend: Backend) {
    let path = TempPath::new("entry_by_value_type");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    over_two_leaves(&store);

    let would_take = store
        .kv()
        .map::<String, HashMap<String, u32>>("widths")
        .expect_err("a type that fits the level below must not turn it into one entry");

    let would_not = store
        .kv()
        .map::<String, u32>("widths")
        .expect_err("and neither must a type that does not fit it");

    assert_eq!(
        named(&would_take),
        named(&would_not),
        "the same refusal, naming the same key, for both value types"
    );

    let (under, stored, said) = named(&would_take);

    assert_eq!(
        (under.as_str(), stored),
        ("widths", "widths.left.pct"),
        "the first key a sorted scan reaches that is not an entry"
    );
    assert!(said.contains("a map owns the level below it"), "{said}");
}
