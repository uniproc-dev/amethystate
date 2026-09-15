use amethystate::MigrationError;
use amethystate::store::OpenStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

#[amethystate(prefix = "clash", version = 1)]
pub struct Width {
    pub width: u32,
}

#[amethystate(prefix = "clash", version = 1)]
pub struct Height {
    pub height: u32,
}

#[backends(all)]
fn two_structs_on_one_prefix_without_an_id_are_refused(backend: Backend) {
    let at = TempPath::new("clash");

    let Err(refused) = StoreBuilder::new(at.path())
        .backend(backend)
        .build_with_migration()
    else {
        panic!("{backend:?}: the store opened over one line declared twice");
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
        "[clash] is declared twice at version 1, by `Height` and by `Width`: two structs sharing \
         a prefix are two lines of declarations, and each needs an `id` of its own",
        "{backend:?}"
    );
}
