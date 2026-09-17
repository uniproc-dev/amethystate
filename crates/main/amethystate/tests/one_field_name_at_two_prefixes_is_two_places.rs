use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "kitchen", id = "lamp", version = 1)]
pub struct KitchenLamp {
    #[amestate(default = 3u32)]
    pub brightness: u32,
}

#[amethystate(prefix = "hall", id = "lamp", version = 1)]
pub struct HallLamp {
    #[amestate(default = 5u32)]
    pub brightness: u32,
}

#[amethystate(prefix = "porch", id = "light", version = 1)]
pub struct PorchLight {
    #[amestate(default = 7u32)]
    pub brightness: u32,
}

#[backends(all)]
fn one_field_name_at_two_prefixes_is_two_places(backend: Backend) {
    let at = TempPath::new("lamps_apart");

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrate()
        .unwrap_or_else(|refused| panic!("{backend:?}: {refused:?}"));

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    assert_eq!(KitchenLamp::new_with(&store).unwrap().brightness().get(), 3);
    assert_eq!(HallLamp::new_with(&store).unwrap().brightness().get(), 5);
    assert_eq!(PorchLight::new_with(&store).unwrap().brightness().get(), 7);
}
