#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;
use std::collections::HashMap;

mod common;
use common::text_backend;

#[test]
fn a_text_map_over_deeper_keys_when_the_value_type_would_take_them() {
    let path = TempPath::new("map_deeper_text");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    store.set(["widths", "left", "px"], &800u32).unwrap();
    store.set(["widths", "left", "pct"], &50u32).unwrap();
    store.save_now().unwrap();

    match store.kv().map::<String, HashMap<String, u32>>("widths") {
        Err(refused) => println!("REFUSED: {refused:?}"),
        Ok(map) => println!("OPENED: {:?}", map.entries().collect::<Vec<_>>()),
    }
}
