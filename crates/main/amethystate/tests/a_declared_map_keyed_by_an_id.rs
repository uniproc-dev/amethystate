use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{Id, ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "net")]
pub struct Net {
    pub ports: ReactiveMap<Id<u16>, bool>,
}

#[backends(all)]
fn a_map_keyed_by_an_id_round_trips_through_a_declaration(backend: Backend) {
    let path = TempPath::new("declared_id_map");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let net = Net::new_with(&store).unwrap();

        net.ports().insert(Id::new(9), &true).unwrap();
        net.ports().insert(Id::new(10), &false).unwrap();
        net.ports().insert(Id::new(100), &true).unwrap();

        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let net = Net::new_with(&store).unwrap();

    let listed: Vec<u16> = net.ports().keys().map(|id| *id.get()).collect();
    assert_eq!(listed, [10, 100, 9]);

    assert_eq!(net.ports().get("9"), Some(true));
    assert_eq!(net.ports().get("10"), Some(false));
}
