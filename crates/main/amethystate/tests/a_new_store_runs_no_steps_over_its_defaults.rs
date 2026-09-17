use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "tally", version = 1)]
    pub struct Tally {
        pub count: u32,
    }
}

#[amethystate(prefix = "tally", version = 2)]
pub struct Tally {
    pub count: ReactiveMap<String, u32>,
}

#[migrate]
fn migrate_tally_v1_to_v2(old: AmeData<v1::Tally>) -> amethystate::MigrationResult<AmeData<Tally>> {
    Ok(AmeData::<Tally> {
        count: [("total".to_string(), old.count)].into_iter().collect(),
    })
}

fn migrated(at: &TempPath, backend: Backend) -> amethystate::Store {
    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .migrate()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    store
}

#[backends(all)]
fn a_new_store_runs_no_steps_over_its_defaults(backend: Backend) {
    let at = TempPath::new("new_store_no_steps");

    let store = migrated(&at, backend);

    assert_eq!(
        store.get::<u32>(["tally", "count", "total"]).unwrap(),
        None,
        "{backend:?}: a store nothing had written to was given what the step made of the defaults"
    );
    assert_eq!(
        Tally::new_with(&store).unwrap().count().len(),
        0,
        "{backend:?}"
    );
}

#[backends(all)]
fn a_store_holding_data_runs_the_steps_of_a_line_with_no_version(backend: Backend) {
    let at = TempPath::new("old_store_runs_steps");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        store.kv().set("elsewhere", &1u32).unwrap();
        store.save_now().unwrap();
    }

    let store = migrated(&at, backend);

    assert_eq!(
        store.get::<u32>(["tally", "count", "total"]).unwrap(),
        Some(0),
        "{backend:?}"
    );
}
