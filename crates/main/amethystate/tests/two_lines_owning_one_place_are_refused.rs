use amethystate::store::OpenStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{MigrationError, ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "dup", id = "as_a_value", version = 1)]
pub struct AsAValue {
    pub x: u32,
}

#[amethystate(prefix = "dup", id = "as_a_map", version = 1)]
pub struct AsAMap {
    pub x: ReactiveMap<String, u32>,
}

#[backends(all)]
fn two_lines_owning_one_place_are_refused(backend: Backend) {
    let at = TempPath::new("dup_place");

    let Err(refused) = StoreBuilder::new(at.path())
        .backend(backend)
        .build_with_migration()
    else {
        panic!(
            "{backend:?}: two lines at `dup` both own `x`, one as a value and one as a map, and \
             the store opened as if either could be what is stored there"
        );
    };

    let OpenStore::Migrating { why } = refused else {
        panic!("{backend:?}: {refused:?}");
    };
    let report = why.into_report();
    let Some(said) = report.downcast_ref::<MigrationError>() else {
        panic!("{backend:?}: {report:?}");
    };

    assert_eq!(
        said.to_string(),
        "`dup.x` is owned by two lines of declarations, `AsAMap` and `AsAValue`: a place holds \
         one thing, so one of them has to give it up",
        "{backend:?}"
    );
}
