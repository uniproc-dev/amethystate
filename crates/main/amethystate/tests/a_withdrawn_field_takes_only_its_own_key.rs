use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "board", version = 1)]
    pub struct Board {
        pub keep: u32,
        pub legacy: u32,
    }
}

#[amethystate(prefix = "board", version = 2)]
pub struct Board {
    pub keep: u32,
}

#[migrate]
fn migrate_board_v1_to_v2(old: AmeData<v1::Board>) -> amethystate::MigrationResult<AmeData<Board>> {
    Ok(AmeData::<Board> { keep: old.keep })
}

#[backends(Redb, Sqlite)]
fn a_key_written_under_a_withdrawn_field_outlives_it(backend: Backend) {
    let at = TempPath::new("withdrawn_own_key");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let board = v1::Board::new_with(&store).unwrap();
        board.keep().set(3).unwrap();
        board.legacy().set(9).unwrap();
        store.set(["board", "legacy", "note"], &7u32).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();
    assert!(!report.has_failures(), "{backend:?}: {report:?}");

    assert_eq!(
        store.get::<u32>(["board", "legacy"]).unwrap(),
        None,
        "{backend:?}"
    );
    assert_eq!(
        store.get::<u32>(["board", "legacy", "note"]).unwrap(),
        Some(7),
        "{backend:?}"
    );
    assert_eq!(Board::new_with(&store).unwrap().keep().get(), 3);
}
