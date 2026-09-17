use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod current {
    use super::*;

    #[amethystate(prefix = "copied", version = 1)]
    pub struct Note {
        #[amestate(default = 4u32)]
        pub pages: u32,
    }
}

mod archive {
    use super::*;

    #[amethystate(prefix = "copied", version = 1)]
    pub struct Note {
        #[amestate(default = 4u32)]
        pub pages: u32,
    }
}

#[backends(all)]
fn one_declaration_compiled_twice_opens(backend: Backend) {
    let at = TempPath::new("copied_note");

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrate()
        .unwrap_or_else(|refused| panic!("{backend:?}: {refused:?}"));

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    assert_eq!(current::Note::new_with(&store).unwrap().pages().get(), 4);
    let _ = archive::Note::new_with;
}
