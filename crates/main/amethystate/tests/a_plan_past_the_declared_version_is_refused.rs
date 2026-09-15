use amethystate::MigrationError;
use amethystate::store::OpenStore;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

#[amethystate(prefix = "ahead", version = 1)]
pub struct Ahead {
    pub count: u32,
}

#[backends(all)]
fn a_plan_past_the_declared_version_is_refused(backend: Backend) {
    let at = TempPath::new("plan_ahead");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["elsewhere"], &1u32).unwrap();
        store.save_now().unwrap();
    }

    let Err(refused) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.for_node::<Ahead>()
                .step(1, "one", |_| Ok(()))
                .step(2, "two", |_| Ok(()));
        })
        .build_with_migration()
    else {
        panic!("{backend:?}: a plan to v2 over a struct declaring v1 opened the store");
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
        "[ahead] has a step to v2 and its struct declares v1: nothing declares the shape that \
         step would leave",
        "{backend:?}"
    );
}
