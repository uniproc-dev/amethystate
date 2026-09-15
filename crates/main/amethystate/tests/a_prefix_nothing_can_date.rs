#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

mod v1 {
    use super::*;

    #[amethystate(prefix = "tally", version = 1)]
    pub struct Counted {
        #[amestate(default = 0u32)]
        pub hits: u32,
    }
}

#[amethystate(prefix = "tally", version = 2)]
pub struct Counted {
    #[amestate(default = 0u32)]
    pub hits: u32,
}

#[migrate]
fn bump_once(old: AmeData<v1::Counted>) -> amethystate::MigrationResult<AmeData<Counted>> {
    Ok(AmeData::<Counted> { hits: old.hits + 1 })
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(120));
}

fn opened(path: &TempPath) -> (amethystate::Store, amethystate::MigrationReport) {
    StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap()
}

#[test]
fn the_open_that_meets_the_missing_bookkeeping_runs_nothing() {
    let path = TempPath::new("dateless_prefix");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let counted = v1::Counted::new_with(&store).unwrap();
        counted.hits().set(0).unwrap();
        drop(counted);
        store.save_now().unwrap();
    }
    settle();

    {
        let (store, report) = opened(&path);
        assert!(!report.has_failures());
        assert_eq!(Counted::new_with(&store).unwrap().hits().get(), 1);
        store.save_now().unwrap();
    }
    settle();

    std::fs::remove_file(meta_path(path.path())).unwrap();

    let (store, report) = opened(&path);

    assert!(
        report.components.iter().any(|component| matches!(
            component.outcome,
            amethystate::migration::ComponentOutcome::Skipped(
                amethystate::migration::NotMigrated::BookkeepingLost { taken_as: 2 }
            )
        )),
        "keys with nothing to date them were migrated anyway"
    );
    assert_eq!(Counted::new_with(&store).unwrap().hits().get(), 1);
}

#[test]
fn a_step_does_not_run_twice_when_the_bookkeeping_went_missing() {
    let path = TempPath::new("dateless_no_rerun");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let counted = v1::Counted::new_with(&store).unwrap();
        counted.hits().set(0).unwrap();
        drop(counted);
        store.save_now().unwrap();
    }
    settle();

    {
        let (store, _) = opened(&path);
        assert_eq!(Counted::new_with(&store).unwrap().hits().get(), 1);
        store.save_now().unwrap();
    }
    settle();

    std::fs::remove_file(meta_path(path.path())).unwrap();

    {
        let (store, _) = opened(&path);
        assert_eq!(Counted::new_with(&store).unwrap().hits().get(), 1);
        store.save_now().unwrap();
    }
    settle();

    let (store, _) = opened(&path);
    assert_eq!(
        Counted::new_with(&store).unwrap().hits().get(),
        1,
        "the open that met the loss dated the prefix, so the one after it had \
         nothing left to invent"
    );
}

#[test]
fn the_keys_are_dated_by_what_the_code_declares() {
    let path = TempPath::new("dateless_records_nothing");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let counted = v1::Counted::new_with(&store).unwrap();
        counted.hits().set(0).unwrap();
        drop(counted);
        store.save_now().unwrap();
    }
    settle();

    {
        let (store, _) = opened(&path);
        let _ = Counted::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    std::fs::remove_file(meta_path(path.path())).unwrap();

    {
        let (store, _) = opened(&path);
        store.save_now().unwrap();
    }
    settle();

    let meta = std::fs::read_to_string(meta_path(path.path())).unwrap();
    let held: serde_json::Value = serde_json::from_str(&meta).unwrap();

    assert_eq!(
        held["meta.tally"]["versions"][""], 2,
        "the open that met the loss left the prefix undated:\n{meta}"
    );
}

#[test]
fn a_first_migration_over_data_written_before_it_still_runs() {
    let path = TempPath::new("dateless_first_step");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let counted = v1::Counted::new_with(&store).unwrap();
        counted.hits().set(7).unwrap();
        drop(counted);
        store.save_now().unwrap();
    }
    settle();

    let (store, report) = opened(&path);

    assert!(!report.has_failures());
    assert_eq!(
        Counted::new_with(&store).unwrap().hits().get(),
        8,
        "a store written before the step existed has a recorded shape, which is \
         what dates it"
    );
}
