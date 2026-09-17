use amethystate::store::OpenStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{MigrationError, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::BTreeSet;

#[amethystate(prefix = "nest", id = "whole", version = 1)]
pub struct Whole {
    #[amestate(default = 1u32)]
    pub x: u32,
}

#[amethystate(prefix = "nest", id = "part", version = 1)]
pub struct Part {
    #[amestate(path = "x.y", default = 2u32)]
    pub inner: u32,
}

#[backends(all)]
fn a_line_owning_a_place_inside_anothers_is_refused(backend: Backend) {
    let at = TempPath::new("nest_inside");

    let Err(refused) = StoreBuilder::new(at.path()).backend(backend).migrate() else {
        panic!(
            "{backend:?}: `nest.x` is a value of one line and `nest.x.y` a place of another, and \
             the store opened as if both could be stored"
        );
    };

    let OpenStore::Migrating { why, .. } = refused else {
        panic!("{backend:?}: {refused:?}");
    };
    let report = why.into_report();
    let Some(MigrationError::ClaimedTwice { at, between }) =
        report.downcast_ref::<MigrationError>()
    else {
        panic!("{backend:?}: {report:?}");
    };

    assert!(
        at == "nest.x" || at == "nest.x.y",
        "{backend:?}: named {at}"
    );
    assert_eq!(
        BTreeSet::from([between.0, between.1]),
        BTreeSet::from(["Part", "Whole"]),
        "{backend:?}"
    );
}
