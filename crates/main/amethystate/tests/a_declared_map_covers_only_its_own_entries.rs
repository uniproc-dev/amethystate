#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;

#[amethystate(prefix = "shop")]
pub struct Shop {
    #[amestate(default = {})]
    pub prices: ReactiveMap<String, u32>,
}

#[test]
fn a_key_nobody_declares_is_written_as_one_name_beside_a_declared_map() {
    let path = TempPath::new("shop_beside_a_map");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let shop = Shop::new_with(&store).unwrap();
        shop.prices.insert("tea".to_string(), &3).unwrap();
        store.set(["misc", "a", "b"], &"loose".to_string()).unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(120));

    insta::assert_snapshot!(std::fs::read_to_string(path.path()).unwrap());
}
