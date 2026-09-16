use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::time::Duration;

mod watched_v1 {
    use super::*;

    #[amethystate(prefix = "watched", version = 1)]
    pub struct Watched {
        #[amestate(default = 1u32)]
        pub kept: u32,
        #[amestate(default = 1u32)]
        pub gone: u32,
    }
}

#[amethystate(prefix = "watched", version = 2)]
pub struct Watched {
    #[amestate(default = 1u32)]
    pub kept: u32,
}

#[migrate]
fn watched_v1_to_v2(
    old: AmeData<watched_v1::Watched>,
) -> amethystate::MigrationResult<AmeData<Watched>> {
    Ok(AmeData::<Watched> { kept: old.kept })
}

#[backends(all)]
fn a_migration_is_not_undone_by_the_watcher_of_the_store_it_opened(backend: Backend) {
    for round in 0..20 {
        let path = TempPath::new("watched");

        {
            let store = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();
            let v1 = watched_v1::Watched::new_with(&store).unwrap();
            v1.gone().set(42).unwrap();
            store.flush_prefix(StorePath::root()).unwrap();
        }

        let (store, _report) = StoreBuilder::new(path.path())
            .backend(backend)
            .disk(|d| d.watch_every(Duration::from_millis(1)))
            .migrations(|m| {
                m.collect_codegen();
            })
            .build_with_migration()
            .unwrap();

        assert_eq!(
            store.get::<u32>(["watched", "gone"]).unwrap(),
            None,
            "{backend:?}, round {round}: the place the step dropped came back"
        );
    }
}
