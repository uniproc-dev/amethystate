use amethystate::amethystate;
use amethystate::store::StoreBackend;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "resets", on_delete = UseDefault)]
pub struct Resets {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate(prefix = "holds")]
pub struct Holds {
    #[amestate(default = 800u32)]
    pub width: u32,
}

//@show a field that wants the default back when its key goes
#[amethystate(prefix = "mixed_delete")]
pub struct MixedDelete {
    #[amestate(default = 800u32)]
    pub width: u32,

    #[amestate(default = 600u32, on_delete = UseDefault)]
    pub height: u32,
}
//@show-end

#[backends(all)]
fn use_default_reports_the_declared_default_again(backend: Backend) {
    let path = TempPath::new("delete_policy_resets");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let state = Resets::new_with(&store).unwrap();

    state.width().set(1200).unwrap();
    assert_eq!(state.width().get(), 1200);

    StoreBackend::delete(&store, &StorePath::from_segments(["resets", "width"])).unwrap();

    assert_eq!(state.width().get(), 800);
}

#[backends(all)]
fn a_deleted_key_goes_on_reporting_the_last_value(backend: Backend) {
    let path = TempPath::new("delete_policy_holds");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let state = Holds::new_with(&store).unwrap();

    state.width().set(1200).unwrap();

    StoreBackend::delete(&store, &StorePath::from_segments(["holds", "width"])).unwrap();

    assert_eq!(state.width().get(), 1200);
    assert_eq!(store.get::<u32>(["holds", "width"]).unwrap(), None);
}

#[backends(all)]
fn a_field_may_disagree_with_the_struct(backend: Backend) {
    let path = TempPath::new("delete_policy_mixed");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let state = MixedDelete::new_with(&store).unwrap();

    state.width().set(1200).unwrap();
    state.height().set(900).unwrap();

    StoreBackend::delete(&store, &StorePath::from_segments(["mixed_delete", "width"])).unwrap();
    StoreBackend::delete(
        &store,
        &StorePath::from_segments(["mixed_delete", "height"]),
    )
    .unwrap();

    assert_eq!(state.width().get(), 1200);
    assert_eq!(state.height().get(), 600);
}
