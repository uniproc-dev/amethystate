use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "ui", mode = "both")]
#[derive(Clone, Debug)]
pub struct Ui {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[backends(all)]
fn a_struct_that_derives_what_the_macro_derives_still_builds(backend: Backend) {
    let path = TempPath::new("derive_beside");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let ui = Ui::new_with(&store).unwrap();
    ui.width().set(1280).unwrap();

    let held = ui.clone();
    assert_eq!(held.width().get(), 1280);
    assert!(format!("{ui:?}").contains("Ui"));
}
