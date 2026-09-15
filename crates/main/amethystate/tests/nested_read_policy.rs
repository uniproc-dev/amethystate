use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate]
pub struct Inherits {
    #[amestate(default = 5432u16)]
    pub port: u16,
}

#[amethystate(on_unreadable = Refuse)]
pub struct Insists {
    #[amestate(default = 5432u16)]
    pub port: u16,
}

#[amethystate(prefix = "lenient_root", on_unreadable = UseDefault)]
pub struct LenientRoot {
    #[amestate(nested)]
    pub db: Inherits,
}

#[amethystate(prefix = "strict_child", on_unreadable = UseDefault)]
pub struct StrictChild {
    #[amestate(nested)]
    pub db: Insists,
}

#[backends(all)]
fn a_nested_struct_inherits_what_the_one_holding_it_decided(backend: Backend) {
    let path = TempPath::new("nested_policy_inherits");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["lenient_root", "db", "port"], &"not a number".to_string())
        .unwrap();

    let state = LenientRoot::new_with(&store).unwrap();

    assert_eq!(state.db().port().get(), 5432);
    assert!(state.db().port().try_get().is_err());
}

#[backends(all)]
fn what_the_nested_struct_declared_wins(backend: Backend) {
    let path = TempPath::new("nested_policy_insists");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["strict_child", "db", "port"], &"not a number".to_string())
        .unwrap();

    assert!(StrictChild::new_with(&store).is_err());
}

#[backends(all)]
fn a_nested_struct_opens_normally_when_nothing_is_wrong(backend: Backend) {
    let path = TempPath::new("nested_policy_ordinary");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let state = LenientRoot::new_with(&store).unwrap();

    assert_eq!(state.db().port().try_get().unwrap(), 5432);
}
