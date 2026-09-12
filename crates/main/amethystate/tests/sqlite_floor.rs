#![cfg(feature = "sqlite")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

fn user_version(path: &std::path::Path) -> i32 {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn set_user_version(path: &std::path::Path, value: i32) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.pragma_update(None, "user_version", value).unwrap();
}

#[test]
fn a_new_store_records_the_floor_it_needs() -> anyhow::Result<()> {
    let path = TempPath::new("sqlite_floor_new");
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Sqlite)
        .build()?;
    store.close()?;

    let file = path.path().with_extension("db");
    assert_eq!(
        user_version(&file),
        3_007_000,
        "WAL is where the floor comes from, and it is in the header"
    );

    Ok(())
}

#[test]
fn a_store_needing_a_newer_sqlite_is_refused_by_name() {
    let path = TempPath::new("sqlite_floor_high");
    let file = path.path().with_extension("db");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Sqlite)
            .build()
            .unwrap();
        store.close().unwrap();
    }

    set_user_version(&file, 9_099_000);

    let refused = StoreBuilder::new(path.path())
        .backend(Backend::Sqlite)
        .build()
        .expect_err("the header asks for a sqlite this build does not link");

    let printed = format!("{refused:?}");
    assert!(
        printed.contains("9099000"),
        "the refusal names the version the store needs: {printed}"
    );
    assert!(
        !printed.contains("malformed database schema"),
        "and it is our message rather than sqlite's: {printed}"
    );
}

#[test]
fn a_store_written_before_the_floor_existed_still_opens() -> anyhow::Result<()> {
    let path = TempPath::new("sqlite_floor_zero");
    let file = path.path().with_extension("db");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Sqlite)
            .build()?;
        store.close()?;
    }

    set_user_version(&file, 0);

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Sqlite)
        .build()?;
    store.close()?;

    assert_eq!(
        user_version(&file),
        3_007_000,
        "and the floor is written on the way through"
    );

    Ok(())
}
