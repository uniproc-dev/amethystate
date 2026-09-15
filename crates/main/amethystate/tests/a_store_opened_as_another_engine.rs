use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

fn written_then_opened(written: Backend, opened: Backend) -> Result<(), String> {
    let at = TempPath::new("another_engine");

    {
        let store = StoreBuilder::new(at.path())
            .backend(written)
            .build()
            .unwrap();
        store.kv().set("port", &8080u16).unwrap();
        store.close().unwrap();
    }

    match StoreBuilder::new(at.path()).backend(opened).build() {
        Ok(store) => {
            let read = store.kv().get::<u16>("port");
            let _ = store.close();
            Err(format!("opened, and read {read:?}"))
        }
        Err(_) => Ok(()),
    }
}

#[test]
fn a_store_opened_as_another_engine_is_refused() {
    let pairs = [
        (Backend::Json, Backend::Toml),
        (Backend::Toml, Backend::Json),
        (Backend::Json, Backend::Ron),
        (Backend::Ron, Backend::Json),
        (Backend::Toml, Backend::Ron),
        (Backend::Redb, Backend::Sqlite),
        (Backend::Sqlite, Backend::Redb),
        (Backend::Json, Backend::Redb),
        (Backend::Redb, Backend::Json),
    ];

    let opened: Vec<String> = pairs
        .iter()
        .filter_map(|(written, opened)| {
            written_then_opened(*written, *opened)
                .err()
                .map(|why| format!("{written:?} as {opened:?}: {why}"))
        })
        .collect();

    assert!(opened.is_empty(), "{opened:#?}");
}
