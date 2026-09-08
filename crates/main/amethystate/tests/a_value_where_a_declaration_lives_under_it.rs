#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

#[amethystate(prefix = "held")]
pub struct Held {
    #[amestate(default = 1u32)]
    pub width: u32,
}

fn edited_by_hand(contents: &str) -> TempPath {
    let path = TempPath::new("value_under_declaration");
    {
        StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
    }
    std::fs::write(path.path(), contents).unwrap();
    path
}

#[test]
fn clearing_walks_past_a_value_where_a_declaration_lives_under_it() {
    let path = edited_by_hand(r#"{"held": 7}"#);

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build()
        .unwrap();

    let cleared = store.kv().clear().unwrap();

    assert!(
        cleared.removed.iter().any(|at| at.as_str() == "held"),
        "nothing declares the value at `held` - a declaration under it is what made the \
         walk look there - so a clear takes it: {cleared:?}"
    );
}

#[test]
fn resetting_walks_past_a_value_where_a_declaration_lives_under_it() {
    let path = edited_by_hand(r#"{"held": 7}"#);

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build()
        .unwrap();

    let cleared = store.kv().reset_to_defaults().unwrap();

    assert!(
        cleared.kept.iter().any(|at| at.as_str() == "held"),
        "a reset takes what is declared, and nothing declares this: {cleared:?}"
    );
}
