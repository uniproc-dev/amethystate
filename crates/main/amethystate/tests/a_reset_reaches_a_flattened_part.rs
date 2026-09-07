use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::HashMap;

fn seeded() -> HashMap<String, u32> {
    let mut held = HashMap::new();
    held.insert("alpha".to_string(), 1);
    held
}

#[amethystate]
pub struct Part {
    #[amestate(default = seeded())]
    pub items: ReactiveMap<String, u32>,
}

#[amethystate(prefix = "flatheld")]
pub struct Holder {
    #[amestate(nested, flatten)]
    pub part: Part,
}

#[backends(all)]
fn a_reset_puts_back_the_defaults_of_a_map_inside_a_flattened_node(backend: Backend) {
    let path = TempPath::new("reset_flattened");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    {
        let held = Holder::new_with(&store).unwrap();
        held.part().items().insert("beta".into(), &2u32).unwrap();
        assert_eq!(held.part().items().get("alpha"), Some(1));
        drop(held);
        store.save_now().unwrap();
    }

    store
        .kv()
        .namespace("flatheld")
        .reset_to_defaults()
        .unwrap();

    let again = Holder::new_with(&store).unwrap();
    let back: Vec<String> = again.part().items().keys().collect();

    assert_eq!(
        back,
        ["alpha".to_string()],
        "a flattened node lends its fields no segment, so the map's marker is at the \
         holder's level and a reset has to clear it there, on {}",
        backend.extension()
    );
}
